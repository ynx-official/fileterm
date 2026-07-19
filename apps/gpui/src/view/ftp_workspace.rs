use std::{collections::HashMap, path::PathBuf, sync::Arc};

use gpui::{div, prelude::*, px, Context, IntoElement, KeyDownEvent, Render, Window};

use crate::{
    ftp::client::{join_remote_path, FtpSession},
    sftp::{
        client::RemoteFileEntry,
        file_manager::FileManager,
        transfer::{
            TransferControl, TransferDirection, TransferFileIdentity, TransferIoOutcome,
            TransferProgress, TransferService, TransferTask, TransferTaskStatus,
        },
    },
    state::AppState,
    theme::ThemePalette,
};

pub struct FtpWorkspace {
    tab_id: String,
    session: Arc<FtpSession>,
    app_state: gpui::Entity<AppState>,
    files: FileManager,
    transfers: TransferService,
    transfer_controls: HashMap<String, TransferControl>,
    editor: Option<FtpFileEditor>,
    error: Option<String>,
}

#[derive(Clone)]
struct FtpFileEditor {
    path: String,
    content: String,
    cursor_byte: usize,
    original_size: u64,
    original_sha256: String,
    busy: bool,
    dirty: bool,
    discard_armed: bool,
    error: Option<String>,
}

impl FtpWorkspace {
    pub fn new(
        tab_id: String,
        session: Arc<FtpSession>,
        remote_path: String,
        transfer_journal_path: PathBuf,
        app_state: gpui::Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut transfers = TransferService::new(transfer_journal_path);
        let error = transfers.load().err().map(|error| error.to_string());
        let mut workspace = Self {
            tab_id,
            session,
            app_state,
            files: FileManager::new(),
            transfers,
            transfer_controls: HashMap::new(),
            editor: None,
            error,
        };
        workspace.load_path(remote_path, cx);
        workspace
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        for control in self.transfer_controls.values() {
            control.cancel();
        }
        self.transfer_controls.clear();
        let session = self.session.clone();
        cx.spawn(async move |_, _| session.close().await).detach();
    }

    fn load_path(&mut self, path: String, cx: &mut Context<Self>) {
        self.files.loading = true;
        self.files.error = None;
        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let result = session.list_dir(Some(&path)).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(entries) => workspace.files.replace_listing(path, entries),
                    Err(error) => workspace.files.fail_loading(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_path(self.files.cwd.to_string_lossy().to_string(), cx);
    }

    fn create_directory(&mut self, cx: &mut Context<Self>) {
        let cwd = self.files.cwd.to_string_lossy().to_string();
        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let name = rfd::AsyncFileDialog::new()
                .set_title("选择任意本地目录名作为新建目录名")
                .pick_folder()
                .await
                .and_then(|handle| handle.file_name().into());
            let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
                return;
            };
            let result = session.mkdir(&join_remote_path(&cwd, &name)).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(()) => workspace.refresh(cx),
                    Err(error) => workspace.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn upload(&mut self, cx: &mut Context<Self>) {
        let cwd = self.files.cwd.to_string_lossy().to_string();
        cx.spawn(async move |this, cx| {
            let files = rfd::AsyncFileDialog::new()
                .pick_files()
                .await
                .unwrap_or_default();
            let _ = this.update(cx, |workspace, cx| {
                for file in files {
                    let local = file.path().to_path_buf();
                    let Some(name) = local.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let destination = join_remote_path(&cwd, name);
                    match workspace.transfers.enqueue(
                        TransferDirection::Upload,
                        &local.to_string_lossy(),
                        &destination,
                        Some(&workspace.tab_id),
                    ) {
                        Ok(id) => workspace.start_transfer(&id, false, cx),
                        Err(error) => workspace.error = Some(error.to_string()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn download(&mut self, entry: RemoteFileEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            return;
        }
        cx.spawn(async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .set_file_name(&entry.name)
                .save_file()
                .await;
            let _ = this.update(cx, |workspace, cx| {
                let Some(file) = selected else { return };
                match workspace.transfers.enqueue(
                    TransferDirection::Download,
                    &entry.path,
                    &file.path().to_string_lossy(),
                    Some(&workspace.tab_id),
                ) {
                    Ok(id) => workspace.start_transfer(&id, false, cx),
                    Err(error) => workspace.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_editor(&mut self, entry: RemoteFileEntry, cx: &mut Context<Self>) {
        const MAX_EDITOR_BYTES: u64 = 1024 * 1024;
        if entry.is_dir || entry.size > MAX_EDITOR_BYTES {
            if entry.size > MAX_EDITOR_BYTES {
                self.error = Some("文件超过 1 MB，请下载后使用外部编辑器".to_string());
                cx.notify();
            }
            return;
        }
        let session = self.session.clone();
        let path = entry.path;
        cx.spawn(async move |this, cx| {
            let result = session.read_text_file(&path, MAX_EDITOR_BYTES).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(file) => {
                        workspace.editor = Some(FtpFileEditor {
                            path,
                            cursor_byte: file.content.len(),
                            content: file.content,
                            original_size: file.size,
                            original_sha256: file.sha256,
                            busy: false,
                            dirty: false,
                            discard_armed: false,
                            error: None,
                        });
                    }
                    Err(error) => workspace.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        const MAX_EDITOR_BYTES: u64 = 1024 * 1024;
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        editor.busy = true;
        editor.error = None;
        let session = self.session.clone();
        let path = editor.path.clone();
        let content = editor.content.clone();
        let original_size = editor.original_size;
        let original_sha256 = editor.original_sha256.clone();
        cx.spawn(async move |this, cx| {
            let result = session
                .write_text_file_if_unchanged(
                    &path,
                    original_size,
                    &original_sha256,
                    &content,
                    MAX_EDITOR_BYTES,
                )
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if let Some(editor) = workspace.editor.as_mut() {
                    editor.busy = false;
                    match result {
                        Ok(file) => {
                            editor.original_size = file.size;
                            editor.original_sha256 = file.sha256;
                            editor.dirty = false;
                            editor.error = None;
                        }
                        Err(error) => editor.error = Some(error.to_string()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_editor_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        if event.keystroke.key == "s"
            && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
        {
            self.save_editor(cx);
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" if editor.dirty && !editor.discard_armed => {
                editor.discard_armed = true;
                editor.error = Some("内容尚未保存；再次按 Esc 放弃修改".to_string());
                cx.notify();
            }
            "escape" => {
                self.editor = None;
                cx.notify();
            }
            "left" => {
                editor.cursor_byte =
                    super::text_editor::previous_char_boundary(&editor.content, editor.cursor_byte)
            }
            "right" => {
                editor.cursor_byte =
                    super::text_editor::next_char_boundary(&editor.content, editor.cursor_byte)
            }
            "up" => {
                editor.cursor_byte = super::text_editor::move_cursor_vertically(
                    &editor.content,
                    editor.cursor_byte,
                    -1,
                )
            }
            "down" => {
                editor.cursor_byte = super::text_editor::move_cursor_vertically(
                    &editor.content,
                    editor.cursor_byte,
                    1,
                )
            }
            "home" => {
                editor.cursor_byte =
                    super::text_editor::line_start(&editor.content, editor.cursor_byte)
            }
            "end" => {
                editor.cursor_byte =
                    super::text_editor::line_end(&editor.content, editor.cursor_byte)
            }
            "enter" | "return" => edit_ftp_content(editor, Some("\n"), false),
            "backspace" => edit_ftp_content(editor, None, true),
            "delete" => delete_ftp_content(editor),
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    edit_ftp_content(editor, Some(text), false);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn delete(&mut self, entry: RemoteFileEntry, cx: &mut Context<Self>) {
        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let confirmed = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("删除 FTP 项目")
                .set_description(format!("确定删除 {}？此操作无法撤销。", entry.path))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await
                == rfd::MessageDialogResult::Yes;
            if !confirmed {
                return;
            }
            let result = session.delete(&entry).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(()) => workspace.refresh(cx),
                    Err(error) => workspace.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_transfer(&mut self, id: &str, overwrite: bool, cx: &mut Context<Self>) {
        let Some(task) = self.transfers.get(id).cloned() else {
            return;
        };
        let session = self.session.clone();
        let control = self
            .transfer_controls
            .entry(id.to_string())
            .or_default()
            .clone();
        control.reset();
        let (progress_tx, mut progress_rx) = tokio::sync::watch::channel(TransferProgress {
            transferred_bytes: task.transferred_bytes.unwrap_or(0),
            total_bytes: task.total_bytes.unwrap_or(0),
        });
        let progress_id = id.to_string();
        cx.spawn(async move |this, cx| {
            while progress_rx.changed().await.is_ok() {
                let progress = *progress_rx.borrow_and_update();
                if this
                    .update(cx, |workspace, cx| {
                        if let Err(error) =
                            workspace.transfers.update_progress(&progress_id, progress)
                        {
                            workspace.error = Some(error.to_string());
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let transfer_id = id.to_string();
        cx.spawn(async move |this, cx| {
            let prepared = inspect_transfer(&session, &task).await;
            let (identity, offset) = match prepared {
                Ok(value) => value,
                Err(error) => {
                    fail_transfer(&this, cx, &transfer_id, error.to_string());
                    return;
                }
            };
            let ready = this.update(cx, |workspace, cx| {
                let result = workspace
                    .transfers
                    .prepare_running(&transfer_id, identity, offset);
                if let Err(error) = &result {
                    workspace.error = Some(error.to_string());
                }
                cx.notify();
                result.is_ok()
            });
            if !matches!(ready, Ok(true)) {
                return;
            }
            let result = execute_transfer(&session, &task, overwrite, control, progress_tx).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(TransferIoOutcome::Completed) => {
                        if let Err(error) = workspace
                            .transfers
                            .finish_io(&transfer_id, TransferIoOutcome::Completed)
                            .and_then(|_| workspace.transfers.complete(&transfer_id))
                        {
                            workspace.error = Some(error.to_string());
                        }
                    }
                    Ok(outcome) => {
                        if let Err(error) = workspace.transfers.finish_io(&transfer_id, outcome) {
                            workspace.error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = workspace.transfers.fail(&transfer_id, message.clone());
                        workspace.error = Some(message);
                    }
                }
                workspace.transfer_controls.remove(&transfer_id);
                workspace.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn pause_transfer(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(control) = self.transfer_controls.get(id) {
            control.pause();
        }
        if let Err(error) = self.transfers.pause(id) {
            self.error = Some(error.to_string());
        }
        cx.notify();
    }

    fn cancel_transfer(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(control) = self.transfer_controls.get(id) {
            control.cancel();
        }
        if let Err(error) = self.transfers.cancel(id) {
            self.error = Some(error.to_string());
        }
        cx.notify();
    }

    fn resume_transfer(&mut self, id: &str, overwrite: bool, cx: &mut Context<Self>) {
        match self.transfers.resume(id) {
            Ok(()) => self.start_transfer(id, overwrite, cx),
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn render_files(&self, palette: ThemePalette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows =
            self.files
                .entries
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, entry)| {
                    let open_path = entry.path.clone();
                    let edit_entry = entry.clone();
                    let download_entry = entry.clone();
                    let delete_entry = entry.clone();
                    let is_dir = entry.is_dir;
                    div()
                        .id(("ftp-entry", index))
                        .h(px(34.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .border_b_1()
                        .border_color(palette.border)
                        .hover(move |style| style.bg(palette.surface_hover))
                        .when(is_dir, |row| {
                            row.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.load_path(open_path.clone(), cx)
                                }))
                        })
                        .child(
                            div()
                                .w(px(20.0))
                                .text_color(if is_dir {
                                    palette.accent
                                } else {
                                    palette.text_soft
                                })
                                .child(if is_dir { "D" } else { "F" }),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .flex_1()
                                .truncate()
                                .text_sm()
                                .child(entry.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.text_soft)
                                .child(if is_dir {
                                    String::new()
                                } else {
                                    format_bytes(entry.size)
                                }),
                        )
                        .when(!is_dir, |row| {
                            row.child(
                                div()
                                    .id(("ftp-edit", index))
                                    .px_2()
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_editor(edit_entry.clone(), cx)
                                    }))
                                    .child("编辑"),
                            )
                            .child(
                                div()
                                    .id(("ftp-download", index))
                                    .px_2()
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.download(download_entry.clone(), cx)
                                    }))
                                    .child("下载"),
                            )
                        })
                        .child(
                            div()
                                .id(("ftp-delete", index))
                                .px_2()
                                .cursor_pointer()
                                .text_color(palette.danger)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete(delete_entry.clone(), cx)
                                }))
                                .child("删除"),
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
        editor: FtpFileEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let visible = super::text_editor::with_visible_cursor(&editor.content, editor.cursor_byte);
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
                            .child("FTP UTF-8 文本编辑器 · Cmd/Ctrl+S 保存 · Esc 关闭"),
                    )
                    .child(
                        div()
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
                            .child(visible),
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
                                    .id("ftp-close-editor")
                                    .px_3()
                                    .py_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let dirty = this.editor.as_ref().is_some_and(|e| e.dirty);
                                        if dirty {
                                            if let Some(editor) = this.editor.as_mut() {
                                                editor.discard_armed = true;
                                                editor.error = Some(
                                                    "内容尚未保存；再次按 Esc 放弃修改".to_string(),
                                                );
                                            }
                                        } else {
                                            this.editor = None;
                                        }
                                        cx.notify();
                                    }))
                                    .child("关闭"),
                            )
                            .child(
                                div()
                                    .id("ftp-save-editor")
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

    fn render_transfers(&self, palette: ThemePalette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .transfers
            .list()
            .iter()
            .rev()
            .take(8)
            .cloned()
            .enumerate()
            .map(|(index, task)| {
                let pause_id = task.id.clone();
                let resume_id = task.id.clone();
                let overwrite_id = task.id.clone();
                let cancel_id = task.id.clone();
                let conflict = task.status == TransferTaskStatus::Failed
                    && task
                        .message
                        .as_deref()
                        .is_some_and(|message| message.contains("destination already exists"));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .child(div().truncate().text_xs().child(task.name))
                            .child(div().text_xs().text_color(palette.text_soft).child(format!(
                                "{} · {:.0}%",
                                status_label(task.status),
                                task.progress * 100.0
                            ))),
                    )
                    .when(task.status == TransferTaskStatus::Running, |row| {
                        row.child(action(
                            "暂停",
                            ("ftp-pause", index),
                            palette.accent,
                            cx,
                            move |this, cx| this.pause_transfer(&pause_id, cx),
                        ))
                    })
                    .when(
                        matches!(
                            task.status,
                            TransferTaskStatus::Paused | TransferTaskStatus::Failed
                        ),
                        |row| {
                            row.child(action(
                                "继续",
                                ("ftp-resume", index),
                                palette.accent,
                                cx,
                                move |this, cx| this.resume_transfer(&resume_id, false, cx),
                            ))
                        },
                    )
                    .when(conflict, |row| {
                        row.child(action(
                            "覆盖",
                            ("ftp-overwrite", index),
                            palette.danger,
                            cx,
                            move |this, cx| this.resume_transfer(&overwrite_id, true, cx),
                        ))
                    })
                    .when(!task.status.terminal(), |row| {
                        row.child(action(
                            "取消",
                            ("ftp-cancel", index),
                            palette.danger,
                            cx,
                            move |this, cx| this.cancel_transfer(&cancel_id, cx),
                        ))
                    })
            });
        div()
            .max_h(px(220.0))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(palette.border_strong)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .child(format!("传输任务 · {}", self.transfers.list().len())),
            )
            .children(rows)
    }
}

fn edit_ftp_content(editor: &mut FtpFileEditor, text: Option<&str>, backspace: bool) {
    let changed = if backspace {
        super::text_editor::backspace(&mut editor.content, &mut editor.cursor_byte)
    } else {
        text.is_some_and(|text| {
            super::text_editor::insert(&mut editor.content, &mut editor.cursor_byte, text)
        })
    };
    if changed {
        mark_editor_changed(editor);
    }
}

fn delete_ftp_content(editor: &mut FtpFileEditor) {
    if super::text_editor::delete(&mut editor.content, &mut editor.cursor_byte) {
        mark_editor_changed(editor);
    }
}

fn mark_editor_changed(editor: &mut FtpFileEditor) {
    editor.dirty = true;
    editor.discard_armed = false;
    editor.error = None;
}

impl Drop for FtpWorkspace {
    fn drop(&mut self) {
        for control in self.transfer_controls.values() {
            control.cancel();
        }
    }
}

impl Render for FtpWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let parent = self.session.parent_path();
        let editor = self.editor.clone();
        div()
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::handle_editor_key))
            .flex()
            .flex_col()
            .bg(palette.background)
            .text_color(palette.text)
            .child(
                div()
                    .h(px(44.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .id("ftp-parent")
                            .px_2()
                            .cursor_pointer()
                            .text_color(palette.accent)
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.load_path(parent.clone(), cx)
                                }),
                            )
                            .child("上级"),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .child(self.files.cwd.to_string_lossy().to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child(format!("FTP · {}", self.session.endpoint())),
                    )
                    .child(
                        div()
                            .id("ftp-refresh")
                            .px_2()
                            .cursor_pointer()
                            .text_color(palette.accent)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                            .child("刷新"),
                    )
                    .child(
                        div()
                            .id("ftp-mkdir")
                            .px_2()
                            .cursor_pointer()
                            .text_color(palette.accent)
                            .on_click(cx.listener(|this, _, _, cx| this.create_directory(cx)))
                            .child("新建目录"),
                    )
                    .child(
                        div()
                            .id("ftp-upload")
                            .px_2()
                            .cursor_pointer()
                            .text_color(palette.accent)
                            .on_click(cx.listener(|this, _, _, cx| this.upload(cx)))
                            .child("上传"),
                    ),
            )
            .when(self.files.loading, |view| {
                view.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(palette.text_soft)
                        .child("正在读取远端目录…"),
                )
            })
            .child(self.render_files(palette, cx))
            .child(self.render_transfers(palette, cx))
            .when_some(
                self.files.error.clone().or_else(|| self.error.clone()),
                |view, error| {
                    view.child(
                        div()
                            .p_3()
                            .text_xs()
                            .text_color(palette.danger)
                            .child(error),
                    )
                },
            )
            .when_some(editor, |view, editor| {
                view.child(self.render_editor(editor, palette, cx))
            })
    }
}

fn action(
    label: &'static str,
    id: impl Into<gpui::ElementId>,
    color: gpui::Hsla,
    cx: &mut Context<FtpWorkspace>,
    callback: impl Fn(&mut FtpWorkspace, &mut Context<FtpWorkspace>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .cursor_pointer()
        .text_xs()
        .text_color(color)
        .on_click(cx.listener(move |this, _, _, cx| callback(this, cx)))
        .child(label)
}

fn fail_transfer(
    this: &gpui::WeakEntity<FtpWorkspace>,
    cx: &mut gpui::AsyncApp,
    id: &str,
    message: String,
) {
    let id = id.to_string();
    let _ = this.update(cx, |workspace, cx| {
        let _ = workspace.transfers.fail(&id, message.clone());
        workspace.error = Some(message);
        workspace.transfer_controls.remove(&id);
        cx.notify();
    });
}

async fn inspect_transfer(
    session: &FtpSession,
    task: &TransferTask,
) -> anyhow::Result<(TransferFileIdentity, u64)> {
    let source = task
        .source_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer source path is missing"))?;
    let partial = task
        .partial_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer partial path is missing"))?;
    match task.direction {
        TransferDirection::Upload => {
            let metadata = tokio::fs::metadata(source).await?;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs());
            let offset = session.remote_file_size_if_exists(partial).await?;
            Ok((
                TransferFileIdentity {
                    size: metadata.len(),
                    modified_at,
                },
                offset,
            ))
        }
        TransferDirection::Download => {
            let (size, modified_at) = session.remote_file_identity(source).await?;
            let offset = match tokio::fs::metadata(partial).await {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error.into()),
            };
            Ok((TransferFileIdentity { size, modified_at }, offset))
        }
    }
}

async fn execute_transfer(
    session: &FtpSession,
    task: &TransferTask,
    overwrite: bool,
    control: TransferControl,
    progress: tokio::sync::watch::Sender<TransferProgress>,
) -> anyhow::Result<TransferIoOutcome> {
    let source = task
        .source_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer source path is missing"))?;
    let destination = task
        .destination_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer destination path is missing"))?;
    let partial = task
        .partial_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer partial path is missing"))?;
    match task.direction {
        TransferDirection::Upload => {
            let outcome = session
                .upload_file(PathBuf::from(source).as_path(), partial, control, progress)
                .await?;
            if outcome == TransferIoOutcome::Completed {
                session
                    .finalize_upload(partial, destination, overwrite)
                    .await?;
            } else if outcome == TransferIoOutcome::Canceled {
                session.remove_remote_file_if_exists(partial).await?;
            }
            Ok(outcome)
        }
        TransferDirection::Download => {
            let partial_path = PathBuf::from(partial);
            let outcome = session
                .download_file(source, &partial_path, control, progress)
                .await?;
            if outcome == TransferIoOutcome::Completed {
                finalize_local_download(&partial_path, &PathBuf::from(destination), overwrite)
                    .await?;
            } else if outcome == TransferIoOutcome::Canceled {
                let _ = tokio::fs::remove_file(partial_path).await;
            }
            Ok(outcome)
        }
    }
}

async fn finalize_local_download(
    partial: &PathBuf,
    destination: &PathBuf,
    overwrite: bool,
) -> anyhow::Result<()> {
    if tokio::fs::try_exists(destination).await? {
        if !overwrite {
            anyhow::bail!(
                "local destination already exists: {}",
                destination.display()
            );
        }
        let backup = destination.with_file_name(format!(
            ".{}.fileterm-backup-{}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("download"),
            uuid::Uuid::new_v4()
        ));
        tokio::fs::rename(destination, &backup).await?;
        if let Err(error) = tokio::fs::rename(partial, destination).await {
            let _ = tokio::fs::rename(&backup, destination).await;
            return Err(error.into());
        }
        let _ = tokio::fs::remove_file(backup).await;
        return Ok(());
    }
    tokio::fs::rename(partial, destination).await?;
    Ok(())
}

fn status_label(status: TransferTaskStatus) -> &'static str {
    match status {
        TransferTaskStatus::Queued => "等待中",
        TransferTaskStatus::Running => "传输中",
        TransferTaskStatus::Paused => "已暂停",
        TransferTaskStatus::Verifying => "校验中",
        TransferTaskStatus::Finalizing => "完成中",
        TransferTaskStatus::Done => "已完成",
        TransferTaskStatus::Failed => "失败",
        TransferTaskStatus::Canceled => "已取消",
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
