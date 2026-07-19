use std::{path::Path, sync::Arc};

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::{
    backend::sessions::local_files::{
        app_list_local_directory, app_read_local_text_file, app_write_local_text_file_if_unchanged,
        DirectorySnapshot, LocalFileItem,
    },
    state::AppState,
    term::{PtyHandle, TermView},
    theme::ThemePalette,
    view::text_editor::{TextInput, TextInputEvent, TextInputMode},
};

const MAX_EDITOR_BYTES: u64 = 1024 * 1024;

pub struct LocalSessionWorkspace {
    shell: String,
    pty: Option<Arc<PtyHandle>>,
    terminal: Option<Entity<TermView>>,
    files: Option<DirectorySnapshot>,
    editor: Option<LocalFileEditor>,
    error: Option<String>,
    app_state: Entity<AppState>,
}

#[derive(Clone)]
struct LocalFileEditor {
    path: String,
    input: Entity<TextInput>,
    content: String,
    original_size: u64,
    original_modified_nanos: u128,
    original_sha256: String,
    busy: bool,
    dirty: bool,
    discard_armed: bool,
    error: Option<String>,
}

impl LocalSessionWorkspace {
    pub fn send_command(&self, command: &str, append_carriage_return: bool) -> anyhow::Result<()> {
        let pty = self
            .pty
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("本地 PTY 未运行"))?;
        pty.write_input(command.as_bytes())?;
        if append_carriage_return {
            pty.write_input(b"\r")?;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if let Some(pty) = self.pty.take() {
            pty.terminate();
        }
        self.terminal = None;
        self.editor = None;
    }

    pub fn spawn(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let shell = default_shell();
        let files = app_list_local_directory(None);
        match PtyHandle::spawn(&shell, 80, 24) {
            Ok((pty, _)) => {
                #[allow(clippy::arc_with_non_send_sync)]
                let pty = Arc::new(pty);
                let terminal = cx.new(|cx| TermView::new(cx, pty.clone(), 80, 24));
                Self {
                    shell,
                    pty: Some(pty),
                    terminal: Some(terminal),
                    files: files.as_ref().ok().cloned(),
                    editor: None,
                    error: files.err().map(|error| error.to_string()),
                    app_state,
                }
            }
            Err(error) => Self {
                shell,
                pty: None,
                terminal: None,
                files: files.as_ref().ok().cloned(),
                editor: None,
                error: Some(match files {
                    Ok(_) => error.to_string(),
                    Err(file_error) => format!("{error}; {file_error}"),
                }),
                app_state,
            },
        }
    }

    fn load_directory(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        match app_list_local_directory(path) {
            Ok(snapshot) => {
                self.files = Some(snapshot);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn choose_directory(&mut self, cx: &mut Context<Self>) {
        let current = self.files.as_ref().map(|files| files.path.clone());
        cx.spawn(async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .set_title("选择本地目录")
                .set_directory(current.unwrap_or_default())
                .pick_folder()
                .await;
            if let Some(selected) = selected {
                let path = selected.path().to_string_lossy().into_owned();
                let _ = this.update(cx, |workspace, cx| workspace.load_directory(Some(path), cx));
            }
        })
        .detach();
    }

    fn open_parent(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.files.as_ref().map(|files| files.path.clone()) else {
            return;
        };
        let parent = Path::new(&current)
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or(current);
        self.load_directory(Some(parent), cx);
    }

    fn open_item(&mut self, item: LocalFileItem, cx: &mut Context<Self>) {
        if item.r#type == "folder" {
            self.load_directory(Some(item.path), cx);
            return;
        }
        match app_read_local_text_file(&item.path, MAX_EDITOR_BYTES) {
            Ok(file) => {
                let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
                let content = file.content;
                let input = cx.new(|cx| {
                    let mut input = TextInput::new(
                        content.clone(),
                        "",
                        TextInputMode::MultiLine,
                        false,
                        palette,
                        cx,
                    );
                    input.set_height(px(520.0), cx);
                    input.set_headless(true, cx);
                    input.request_focus(cx);
                    input
                });
                cx.subscribe(&input, |workspace, _, event, cx| match event {
                    TextInputEvent::Changed(value) => {
                        if let Some(editor) = workspace.editor.as_mut() {
                            editor.content = value.clone();
                            mark_editor_changed(editor);
                        }
                    }
                    TextInputEvent::Save => workspace.save_editor(cx),
                    TextInputEvent::Cancel => workspace.cancel_editor(cx),
                    TextInputEvent::Submit => {}
                })
                .detach();
                self.editor = Some(LocalFileEditor {
                    path: item.path,
                    input,
                    content,
                    original_size: file.size,
                    original_modified_nanos: file.modified_nanos,
                    original_sha256: file.sha256,
                    busy: false,
                    dirty: false,
                    discard_armed: false,
                    error: None,
                });
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        editor.busy = true;
        editor.error = None;
        let result = app_write_local_text_file_if_unchanged(
            &editor.path,
            editor.original_size,
            editor.original_modified_nanos,
            &editor.original_sha256,
            &editor.content,
            MAX_EDITOR_BYTES,
        );
        editor.busy = false;
        match result {
            Ok(file) => {
                editor.original_size = file.size;
                editor.original_modified_nanos = file.modified_nanos;
                editor.original_sha256 = file.sha256;
                editor.dirty = false;
            }
            Err(error) => editor.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn cancel_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.dirty && !editor.discard_armed {
            editor.discard_armed = true;
            editor.error = Some("内容尚未保存；再次按 Esc 放弃修改".to_string());
        } else {
            self.editor = None;
        }
        cx.notify();
    }

    fn render_files(&self, palette: ThemePalette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows =
            self.files
                .as_ref()
                .map(|snapshot| snapshot.items.clone())
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let item_for_open = item.clone();
                    div()
                        .h(px(32.0))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(palette.border)
                        .id(("local-file", index))
                        .cursor_pointer()
                        .hover(move |style| style.bg(palette.surface_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_item(item_for_open.clone(), cx)
                        }))
                        .child(
                            div()
                                .w(px(14.0))
                                .text_xs()
                                .text_color(if item.r#type == "folder" {
                                    palette.accent
                                } else {
                                    palette.text_soft
                                })
                                .child(if item.r#type == "folder" { "D" } else { "F" }),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .flex_1()
                                .truncate()
                                .text_sm()
                                .child(item.name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.text_soft)
                                .child(item.size),
                        )
                });
        div()
            .min_h(px(0.0))
            .flex_1()
            .overflow_hidden()
            .children(rows)
    }

    fn render_editor(
        &self,
        editor: LocalFileEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let visible = editor.input.read(cx).content_with_cursor();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.7))
            .child(
                div()
                    .w(px(820.0))
                    .h(px(620.0))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(div().truncate().text_lg().child(editor.path))
                            .child(div().text_xs().text_color(palette.text_muted).child(
                                if editor.busy {
                                    "保存中"
                                } else if editor.dirty {
                                    "未保存"
                                } else {
                                    "已保存"
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child("本地 UTF-8 文本编辑器 · Cmd/Ctrl+S 保存 · Esc 关闭"),
                    )
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.0))
                            .flex_1()
                            .overflow_hidden()
                            .p_3()
                            .rounded_md()
                            .bg(palette.background)
                            .border_1()
                            .border_color(palette.accent)
                            .font_family("monospace")
                            .text_sm()
                            .child(visible)
                            .child(editor.input.clone()),
                    )
                    .when_some(editor.error, |view, error| {
                        view.child(div().text_xs().text_color(palette.danger).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("local-close-editor")
                                    .px_3()
                                    .py_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_editor(cx)))
                                    .child("关闭"),
                            )
                            .child(
                                div()
                                    .id("local-save-editor")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_color(palette.background)
                                    .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx)))
                                    .child("保存"),
                            ),
                    ),
            )
    }
}

fn mark_editor_changed(editor: &mut LocalFileEditor) {
    editor.dirty = true;
    editor.discard_armed = false;
    editor.error = None;
}

impl Drop for LocalSessionWorkspace {
    fn drop(&mut self) {
        if let Some(pty) = self.pty.take() {
            pty.terminate();
        }
    }
}

impl Render for LocalSessionWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let shell = self.shell.clone();
        let terminal = self.terminal.clone();
        let error = self.error.clone();
        let editor = self.editor.clone();
        let current_path = self
            .files
            .as_ref()
            .map(|files| files.path.clone())
            .unwrap_or_else(|| "-".to_string());
        div()
            .size_full()
            .relative()
            .flex()
            .bg(palette.background)
            .child(
                div()
                    .min_w(px(0.0))
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .bg(palette.surface)
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child(format!("本地终端 · {shell}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.success)
                                    .child("本机 PTY"),
                            ),
                    )
                    .child(
                        div()
                            .min_h(px(0.0))
                            .flex_1()
                            .when_some(terminal, |view, terminal| view.child(terminal))
                            .when_some(error.clone(), |view, error| {
                                view.flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(palette.danger)
                                    .child(format!("本地终端或文件服务错误：{error}"))
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(320.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(palette.surface)
                    .border_l_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .h(px(44.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .id("local-parent")
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(|this, _, _, cx| this.open_parent(cx)))
                                    .child("上级"),
                            )
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .flex_1()
                                    .truncate()
                                    .text_xs()
                                    .child(current_path),
                            )
                            .child(
                                div()
                                    .id("local-refresh")
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let path =
                                            this.files.as_ref().map(|files| files.path.clone());
                                        this.load_directory(path, cx);
                                    }))
                                    .child("刷新"),
                            )
                            .child(
                                div()
                                    .id("local-choose-directory")
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.choose_directory(cx)),
                                    )
                                    .child("选择"),
                            ),
                    )
                    .child(self.render_files(palette, cx))
                    .when_some(error, |view, error| {
                        view.child(
                            div()
                                .p_3()
                                .text_xs()
                                .text_color(palette.danger)
                                .child(error),
                        )
                    }),
            )
            .when_some(editor, |view, editor| {
                view.child(self.render_editor(editor, palette, cx))
            })
    }
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "powershell.exe".to_string()
            } else if std::path::Path::new("/bin/zsh").exists() {
                "/bin/zsh".to_string()
            } else if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash".to_string()
            } else {
                "sh".to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_resolution_never_returns_empty() {
        assert!(!default_shell().trim().is_empty());
    }
}
