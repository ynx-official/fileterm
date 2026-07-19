use gpui::{div, prelude::*, px, AnyElement, Context, KeyDownEvent};
use zeroize::Zeroize;

use super::RootView;
use crate::{
    services::ssh_keys::{SshKeyFileSelection, SshKeyFolder, SshKeyLayout, SshKeyMetadata},
    state::AppState,
    theme::ThemePalette,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SshKeyEditorMode {
    Import,
    Edit(String),
    CreateFolder,
    RenameFolder(String),
}

#[derive(Clone)]
pub(super) struct PendingSshKeyEditor {
    pub(super) mode: SshKeyEditorMode,
    pub(super) name: String,
    pub(super) note: String,
    pub(super) source: Option<SshKeyFileSelection>,
    pub(super) active_field: usize,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
}

impl Drop for PendingSshKeyEditor {
    fn drop(&mut self) {
        self.note.zeroize();
        if let Some(source) = self.source.as_mut() {
            source.source_path.zeroize();
        }
    }
}

impl PendingSshKeyEditor {
    fn import() -> Self {
        Self {
            mode: SshKeyEditorMode::Import,
            name: String::new(),
            note: String::new(),
            source: None,
            active_field: 0,
            busy: false,
            error: None,
        }
    }

    fn edit(key: &SshKeyMetadata) -> Self {
        Self {
            mode: SshKeyEditorMode::Edit(key.id.clone()),
            name: key.name.clone(),
            note: key.note.clone().unwrap_or_default(),
            source: None,
            active_field: 0,
            busy: false,
            error: None,
        }
    }

    fn folder(mode: SshKeyEditorMode, name: String) -> Self {
        Self {
            mode,
            name,
            note: String::new(),
            source: None,
            active_field: 0,
            busy: false,
            error: None,
        }
    }

    fn active_value(&mut self) -> &mut String {
        match self.mode {
            SshKeyEditorMode::Import => &mut self.note,
            SshKeyEditorMode::Edit(_) if self.active_field == 0 => &mut self.name,
            SshKeyEditorMode::Edit(_) => &mut self.note,
            SshKeyEditorMode::CreateFolder | SshKeyEditorMode::RenameFolder(_) => &mut self.name,
        }
    }

    fn field_count(&self) -> usize {
        if matches!(self.mode, SshKeyEditorMode::Edit(_)) {
            2
        } else {
            1
        }
    }
}

impl RootView {
    pub(super) fn reload_ssh_key_library(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio::try_join!(api.ssh_keys_list(), api.ssh_keys_get_layout());
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok((keys, layout)) => root.state.update(cx, |state, cx| {
                        state.apply_ssh_key_library(keys, layout);
                        state.data_error = None;
                        cx.notify();
                    }),
                    Err(error) => {
                        root.update_state(cx, |state| state.data_error = Some(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn begin_ssh_key_import(&mut self, cx: &mut Context<Self>) {
        self.pending_ssh_key_editor = Some(PendingSshKeyEditor::import());
        cx.notify();
    }

    fn begin_ssh_key_edit(&mut self, key_id: String, cx: &mut Context<Self>) {
        let key = self
            .state
            .read(cx)
            .ssh_keys
            .iter()
            .find(|key| key.id == key_id)
            .cloned();
        if let Some(key) = key {
            self.pending_ssh_key_editor = Some(PendingSshKeyEditor::edit(&key));
        }
        cx.notify();
    }

    fn begin_create_key_folder(&mut self, cx: &mut Context<Self>) {
        self.pending_ssh_key_editor = Some(PendingSshKeyEditor::folder(
            SshKeyEditorMode::CreateFolder,
            String::new(),
        ));
        cx.notify();
    }

    fn begin_rename_key_folder(&mut self, folder_id: String, cx: &mut Context<Self>) {
        let folder = self
            .state
            .read(cx)
            .ssh_key_layout
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .cloned();
        if let Some(folder) = folder {
            self.pending_ssh_key_editor = Some(PendingSshKeyEditor::folder(
                SshKeyEditorMode::RenameFolder(folder.id),
                folder.name,
            ));
        }
        cx.notify();
    }

    fn select_ssh_key_file(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pending_ssh_key_editor.as_mut() else {
            return;
        };
        editor.busy = true;
        editor.error = None;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ssh_keys_select_file().await;
            let _ = this.update(cx, |root, cx| {
                if let Some(editor) = root.pending_ssh_key_editor.as_mut() {
                    editor.busy = false;
                    match result {
                        Ok(selection) => editor.source = selection,
                        Err(error) => editor.error = Some(error.to_string()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn handle_ssh_key_editor_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.pending_ssh_key_editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                self.pending_ssh_key_editor = None;
                cx.notify();
            }
            "tab" => {
                editor.active_field = (editor.active_field + 1) % editor.field_count();
                cx.notify();
            }
            "enter" | "return" => self.save_ssh_key_editor(cx),
            "backspace" => {
                editor.active_value().pop();
                cx.notify();
            }
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    editor.active_value().push_str(text);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn save_ssh_key_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pending_ssh_key_editor.as_mut() else {
            return;
        };
        let mode = editor.mode.clone();
        let name = editor.name.trim().to_string();
        let note = editor.note.trim().to_string();
        let source_path = editor
            .source
            .as_ref()
            .map(|source| source.source_path.clone());
        editor.busy = true;
        editor.error = None;
        let api = self.api.clone();
        let current_layout = self.state.read(cx).ssh_key_layout.clone();
        cx.spawn(async move |this, cx| {
            let result = match mode {
                SshKeyEditorMode::Import => match source_path {
                    Some(path) => api.ssh_keys_import(path, note).await.map(|_| ()),
                    None => Err(crate::error::AppError::Command(
                        "请选择 SSH 私钥文件。".to_string(),
                    )),
                },
                SshKeyEditorMode::Edit(key_id) => {
                    if name.is_empty() || note.is_empty() {
                        Err(crate::error::AppError::Command(
                            "密钥名称和备注不能为空。".to_string(),
                        ))
                    } else {
                        match api.ssh_keys_rename(key_id.clone(), name).await {
                            Ok(_) => api.ssh_keys_update_note(key_id, note).await.map(|_| ()),
                            Err(error) => Err(error),
                        }
                    }
                }
                SshKeyEditorMode::CreateFolder => {
                    let mut layout = current_layout;
                    layout.folders.push(SshKeyFolder {
                        id: uuid::Uuid::new_v4().to_string(),
                        name,
                    });
                    api.ssh_keys_save_layout(layout).await.map(|_| ())
                }
                SshKeyEditorMode::RenameFolder(folder_id) => {
                    let mut layout = current_layout;
                    if let Some(folder) = layout
                        .folders
                        .iter_mut()
                        .find(|folder| folder.id == folder_id)
                    {
                        folder.name = name;
                    }
                    api.ssh_keys_save_layout(layout).await.map(|_| ())
                }
            };
            let _ = this.update(cx, |root, cx| match result {
                Ok(()) => {
                    root.pending_ssh_key_editor = None;
                    root.reload_ssh_key_library(cx);
                }
                Err(error) => {
                    if let Some(editor) = root.pending_ssh_key_editor.as_mut() {
                        editor.busy = false;
                        editor.error = Some(error.to_string());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn request_delete_ssh_key(&mut self, key_id: String, name: String, cx: &mut Context<Self>) {
        self.pending_ssh_key_delete = Some((key_id, name));
        cx.notify();
    }

    fn confirm_delete_ssh_key(&mut self, cx: &mut Context<Self>) {
        let Some((key_id, _)) = self.pending_ssh_key_delete.clone() else {
            return;
        };
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ssh_keys_delete(key_id).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(()) => {
                    root.pending_ssh_key_delete = None;
                    root.reload_ssh_key_library(cx);
                }
                Err(error) => {
                    root.pending_ssh_key_delete = None;
                    root.update_state(cx, |state| state.data_error = Some(error.to_string()));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn delete_ssh_key_folder(&mut self, folder_id: String, cx: &mut Context<Self>) {
        let mut layout = self.state.read(cx).ssh_key_layout.clone();
        layout.folders.retain(|folder| folder.id != folder_id);
        layout
            .assignments
            .retain(|_, assigned| assigned != &folder_id);
        layout.item_order.remove(&folder_id);
        self.persist_ssh_key_layout(layout, cx);
    }

    fn cycle_ssh_key_folder(&mut self, key_id: String, cx: &mut Context<Self>) {
        let mut layout = self.state.read(cx).ssh_key_layout.clone();
        let next_folder = match layout.assignments.get(&key_id) {
            None => layout.folders.first().map(|folder| folder.id.clone()),
            Some(current) => layout
                .folders
                .iter()
                .position(|folder| &folder.id == current)
                .and_then(|index| layout.folders.get(index + 1))
                .map(|folder| folder.id.clone()),
        };
        if let Some(folder_id) = next_folder {
            layout.assignments.insert(key_id, folder_id);
        } else {
            layout.assignments.remove(&key_id);
        }
        self.persist_ssh_key_layout(layout, cx);
    }

    fn persist_ssh_key_layout(&mut self, layout: SshKeyLayout, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ssh_keys_save_layout(layout).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(layout) => root.state.update(cx, |state, cx| {
                    state.ssh_key_layout = layout;
                    cx.notify();
                }),
                Err(error) => {
                    root.update_state(cx, |state| state.data_error = Some(error.to_string()))
                }
            });
        })
        .detach();
    }

    pub(super) fn render_ssh_key_manager(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_folder = self.active_ssh_key_folder.clone();
        let query = self.ssh_key_query.trim().to_lowercase();
        let mut keys = state
            .ssh_keys
            .iter()
            .filter(|key| {
                active_folder.as_ref().is_none_or(|folder_id| {
                    state.ssh_key_layout.assignments.get(&key.id) == Some(folder_id)
                })
            })
            .filter(|key| {
                query.is_empty()
                    || key.name.to_lowercase().contains(&query)
                    || key
                        .note
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
                    || key.algorithm.to_lowercase().contains(&query)
                    || key.fingerprint.to_lowercase().contains(&query)
            })
            .cloned()
            .collect::<Vec<_>>();
        keys.sort_by_key(|key| {
            state
                .ssh_key_layout
                .item_order
                .get(&key.id)
                .copied()
                .unwrap_or(key.imported_at)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .text_color(palette.text)
                                    .child("密钥管理器"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .child(format!(
                                        "{} 个密钥，{} 个文件夹",
                                        state.ssh_keys.len(),
                                        state.ssh_key_layout.folders.len()
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(key_action_button(
                                "刷新",
                                "refresh-ssh-keys",
                                false,
                                palette,
                                cx,
                                |this, cx| this.reload_ssh_key_library(cx),
                            ))
                            .child(key_action_button(
                                "新建文件夹",
                                "create-ssh-key-folder",
                                false,
                                palette,
                                cx,
                                |this, cx| this.begin_create_key_folder(cx),
                            ))
                            .child(key_action_button(
                                "导入私钥",
                                "import-ssh-key",
                                true,
                                palette,
                                cx,
                                |this, cx| this.begin_ssh_key_import(cx),
                            )),
                    ),
            )
            .child(
                div()
                    .id("ssh-key-search")
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .px_3()
                    .rounded_md()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(if self.ssh_key_search_focused {
                        palette.accent
                    } else {
                        palette.border
                    })
                    .cursor_pointer()
                    .text_sm()
                    .text_color(if self.ssh_key_query.is_empty() {
                        palette.text_soft
                    } else {
                        palette.text
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ssh_key_search_focused = true;
                        cx.notify();
                    }))
                    .child(if self.ssh_key_query.is_empty() {
                        "筛选名称、备注、算法或指纹…".to_string()
                    } else {
                        self.ssh_key_query.clone()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .child(self.render_ssh_key_folders(state, palette, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(px(42.0))
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .px_4()
                                    .border_b_1()
                                    .border_color(palette.border)
                                    .text_xs()
                                    .text_color(palette.text_soft)
                                    .child(div().flex_1().child("名称 / 状态"))
                                    .child(div().w(px(180.0)).child("算法 / 指纹"))
                                    .child(div().w(px(140.0)).child("备注"))
                                    .child(div().w(px(100.0)).child("导入时间"))
                                    .child(div().w(px(40.0)).child("引用"))
                                    .child(div().w(px(150.0)).child("操作")),
                            )
                            .when(keys.is_empty(), |view| {
                                view.flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .child(if query.is_empty() {
                                        "尚未导入私钥。导入后可在 SSH 连接中复用。"
                                    } else {
                                        "没有匹配的密钥。"
                                    })
                            })
                            .when(!keys.is_empty(), |view| {
                                view.children(keys.into_iter().enumerate().map(|(index, key)| {
                                    self.render_ssh_key_row(index, key, state, palette, cx)
                                }))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_ssh_key_folders(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all_active = self.active_ssh_key_folder.is_none();
        div()
            .w(px(190.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(palette.background)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .id("all-ssh-keys")
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if all_active {
                        palette.surface_active
                    } else {
                        palette.background
                    })
                    .text_sm()
                    .text_color(if all_active {
                        palette.text
                    } else {
                        palette.text_muted
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active_ssh_key_folder = None;
                        cx.notify();
                    }))
                    .child("全部密钥")
                    .child(state.ssh_keys.len().to_string()),
            )
            .children(
                state
                    .ssh_key_layout
                    .folders
                    .iter()
                    .enumerate()
                    .map(|(index, folder)| {
                        let folder_id = folder.id.clone();
                        let rename_id = folder.id.clone();
                        let delete_id = folder.id.clone();
                        let active = self.active_ssh_key_folder.as_deref() == Some(&folder.id);
                        let count = state
                            .ssh_key_layout
                            .assignments
                            .values()
                            .filter(|assigned| *assigned == &folder.id)
                            .count();
                        div()
                            .id(("ssh-key-folder", index))
                            .h(px(38.0))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded_md()
                            .bg(if active {
                                palette.surface_active
                            } else {
                                palette.background
                            })
                            .text_xs()
                            .text_color(if active {
                                palette.text
                            } else {
                                palette.text_muted
                            })
                            .child(
                                div()
                                    .id(("select-ssh-key-folder", index))
                                    .flex_1()
                                    .truncate()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.active_ssh_key_folder = Some(folder_id.clone());
                                        cx.notify();
                                    }))
                                    .child(format!("{} ({count})", folder.name)),
                            )
                            .child(
                                div()
                                    .id(("rename-ssh-key-folder", index))
                                    .cursor_pointer()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.begin_rename_key_folder(rename_id.clone(), cx)
                                    }))
                                    .child("改"),
                            )
                            .child(
                                div()
                                    .id(("delete-ssh-key-folder", index))
                                    .cursor_pointer()
                                    .text_color(palette.danger)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_ssh_key_folder(delete_id.clone(), cx)
                                    }))
                                    .child("删"),
                            )
                    }),
            )
            .into_any_element()
    }

    fn render_ssh_key_row(
        &self,
        index: usize,
        key: SshKeyMetadata,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let edit_id = key.id.clone();
        let move_id = key.id.clone();
        let delete_id = key.id.clone();
        let delete_name = key.name.clone();
        let folder_name = state
            .ssh_key_layout
            .assignments
            .get(&key.id)
            .and_then(|folder_id| {
                state
                    .ssh_key_layout
                    .folders
                    .iter()
                    .find(|folder| &folder.id == folder_id)
            })
            .map(|folder| folder.name.as_str())
            .unwrap_or("全部密钥");
        div()
            .id(("ssh-key-row", index))
            .min_h(px(72.0))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .border_b_1()
            .border_color(palette.border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(palette.text)
                            .child(key.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if key.encrypted {
                                palette.warning
                            } else {
                                palette.text_soft
                            })
                            .child(if key.encrypted {
                                "已加密"
                            } else {
                                "未加密"
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(180.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.accent)
                            .child(key.algorithm.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child(short_fingerprint(&key.fingerprint)),
                    ),
            )
            .child(
                div()
                    .w(px(140.0))
                    .truncate()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(key.note.clone().unwrap_or_else(|| "—".to_string())),
            )
            .child(
                div()
                    .w(px(100.0))
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child(format_imported_at(key.imported_at)),
            )
            .child(
                div()
                    .w(px(40.0))
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(key.usage_count.to_string()),
            )
            .child(
                div()
                    .w(px(150.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id(("move-ssh-key", index))
                            .max_w(px(78.0))
                            .truncate()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cycle_ssh_key_folder(move_id.clone(), cx)
                            }))
                            .child(folder_name.to_string()),
                    )
                    .child(
                        div()
                            .id(("edit-ssh-key", index))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.begin_ssh_key_edit(edit_id.clone(), cx)
                            }))
                            .child("编辑"),
                    )
                    .child(
                        div()
                            .id(("delete-ssh-key", index))
                            .text_xs()
                            .text_color(if key.usage_count == 0 {
                                palette.danger
                            } else {
                                palette.text_soft
                            })
                            .when(key.usage_count == 0, |button| {
                                button.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_delete_ssh_key(
                                            delete_id.clone(),
                                            delete_name.clone(),
                                            cx,
                                        )
                                    },
                                ))
                            })
                            .child(if key.usage_count == 0 {
                                "删除"
                            } else {
                                "使用中"
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_ssh_key_editor(
        &self,
        editor: PendingSshKeyEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = match editor.mode {
            SshKeyEditorMode::Import => "导入 SSH 私钥",
            SshKeyEditorMode::Edit(_) => "编辑密钥",
            SshKeyEditorMode::CreateFolder => "新建密钥文件夹",
            SshKeyEditorMode::RenameFolder(_) => "重命名密钥文件夹",
        };
        let is_import = matches!(editor.mode, SshKeyEditorMode::Import);
        let is_edit = matches!(editor.mode, SshKeyEditorMode::Edit(_));
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .child(
                div()
                    .w(px(600.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(div().text_lg().text_color(palette.text).child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child("Tab 切换输入项，Enter 保存，Esc 取消。"),
                    )
                    .when(is_edit, |view| {
                        view.child(key_input(
                            "名称",
                            editor.name.clone(),
                            0,
                            editor.active_field,
                            palette,
                            cx,
                        ))
                    })
                    .when(!is_import && !is_edit, |view| {
                        view.child(key_input(
                            "文件夹名称",
                            editor.name.clone(),
                            0,
                            editor.active_field,
                            palette,
                            cx,
                        ))
                    })
                    .when(is_import || is_edit, |view| {
                        let field = usize::from(is_edit);
                        view.child(key_input(
                            "备注信息",
                            editor.note.clone(),
                            field,
                            editor.active_field,
                            palette,
                            cx,
                        ))
                    })
                    .when(is_import, |view| {
                        let file_label = editor
                            .source
                            .as_ref()
                            .map(|source| source.file_name.clone())
                            .unwrap_or_else(|| "尚未选择私钥文件".to_string());
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .p_3()
                                .rounded_md()
                                .bg(palette.background)
                                .border_1()
                                .border_color(palette.border)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(palette.text)
                                                .child(file_label),
                                        )
                                        .when_some(
                                            editor
                                                .source
                                                .as_ref()
                                                .and_then(|source| source.existing_key.as_ref()),
                                            |view, existing| {
                                                view.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(palette.warning)
                                                        .child(format!(
                                                            "该私钥已导入：{}",
                                                            existing.name
                                                        )),
                                                )
                                            },
                                        ),
                                )
                                .child(key_action_button(
                                    if editor.source.is_some() {
                                        "重新选择"
                                    } else {
                                        "选择文件"
                                    },
                                    "select-ssh-key-file",
                                    false,
                                    palette,
                                    cx,
                                    |this, cx| this.select_ssh_key_file(cx),
                                )),
                        )
                    })
                    .when_some(editor.error.clone(), |view, error| {
                        view.child(div().text_xs().text_color(palette.danger).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(key_action_button(
                                "取消",
                                "cancel-ssh-key-editor",
                                false,
                                palette,
                                cx,
                                |this, cx| {
                                    this.pending_ssh_key_editor = None;
                                    cx.notify();
                                },
                            ))
                            .child(key_action_button(
                                if editor.busy { "保存中" } else { "保存" },
                                "save-ssh-key-editor",
                                true,
                                palette,
                                cx,
                                |this, cx| this.save_ssh_key_editor(cx),
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_ssh_key_delete_confirmation(
        &self,
        key_id: String,
        key_name: String,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .child(
                div()
                    .w(px(480.0))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(div().text_lg().text_color(palette.text).child("删除密钥"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(format!(
                                "确定删除 {key_name} 吗？此操作只删除 FileTerm 管理的副本，不会删除原始文件。"
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(key_action_button(
                                "取消",
                                "cancel-delete-ssh-key",
                                false,
                                palette,
                                cx,
                                |this, cx| {
                                    this.pending_ssh_key_delete = None;
                                    cx.notify();
                                },
                            ))
                            .child(
                                div()
                                    .id("confirm-delete-ssh-key")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.danger)
                                    .text_xs()
                                    .text_color(palette.background)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.pending_ssh_key_delete.as_ref().map(|value| &value.0) == Some(&key_id) {
                                            this.confirm_delete_ssh_key(cx);
                                        }
                                    }))
                                    .child("删除"),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn key_input(
    label: &'static str,
    value: String,
    field: usize,
    active_field: usize,
    palette: ThemePalette,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let empty = value.is_empty();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(palette.text_muted).child(label))
        .child(
            div()
                .id(("ssh-key-input", field))
                .h(px(38.0))
                .flex()
                .items_center()
                .px_3()
                .rounded_md()
                .cursor_pointer()
                .bg(palette.background)
                .border_1()
                .border_color(if active_field == field {
                    palette.accent
                } else {
                    palette.border
                })
                .text_sm()
                .text_color(if empty {
                    palette.text_soft
                } else {
                    palette.text
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.pending_ssh_key_editor.as_mut() {
                        editor.active_field = field;
                    }
                    cx.notify();
                }))
                .child(if empty {
                    "输入内容".to_string()
                } else {
                    value
                }),
        )
}

fn key_action_button(
    label: &'static str,
    id: impl Into<gpui::ElementId>,
    primary: bool,
    palette: ThemePalette,
    cx: &mut Context<RootView>,
    callback: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .bg(if primary {
            palette.accent
        } else {
            palette.background
        })
        .border_1()
        .border_color(if primary {
            palette.accent
        } else {
            palette.border
        })
        .text_xs()
        .text_color(if primary {
            palette.background
        } else {
            palette.accent
        })
        .hover(move |style| style.bg(palette.surface_hover))
        .on_click(cx.listener(move |this, _, _, cx| callback(this, cx)))
        .child(label)
}

fn format_imported_at(imported_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let elapsed = now.saturating_sub(imported_at);
    match elapsed {
        0..=59_999 => "刚刚".to_string(),
        60_000..=3_599_999 => format!("{} 分钟前", elapsed / 60_000),
        3_600_000..=86_399_999 => format!("{} 小时前", elapsed / 3_600_000),
        _ => format!("{} 天前", elapsed / 86_400_000),
    }
}

fn short_fingerprint(fingerprint: &str) -> String {
    if fingerprint.chars().count() > 34 {
        format!(
            "{}…{}",
            &fingerprint[..18],
            &fingerprint[fingerprint.len() - 12..]
        )
    } else {
        fingerprint.to_string()
    }
}
