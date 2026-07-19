use gpui::{div, prelude::*, px, AnyElement, Context, Entity, FocusHandle, Focusable, MouseButton};
use zeroize::Zeroize;

use super::RootView;
use crate::{
    services::ssh_keys::{SshKeyFileSelection, SshKeyFolder, SshKeyLayout, SshKeyMetadata},
    state::AppState,
    theme::ThemePalette,
    view::text_editor::{TextInput, TextInputEvent, TextInputMode},
};

mod layout;
use layout::{
    assign_key_folder, delete_folder_from_layout, next_root_order, normalize_layout,
    reorder_relative, root_items,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SshKeyDeleteTarget {
    Key { id: String, name: String },
    Folder { id: String, name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SshKeyDragItem {
    Key(String),
    Folder(String),
}

impl SshKeyDragItem {
    fn id(&self) -> &str {
        match self {
            Self::Key(id) | Self::Folder(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SshKeyDropPosition {
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SshKeyEditorMode {
    Import,
    Edit(String),
    CreateFolder,
    RenameFolder(String),
}

#[derive(Clone)]
pub(super) struct PendingSshKeyEditor {
    form_id: uuid::Uuid,
    input: Option<Entity<TextInput>>,
    auto_focus: bool,
    pub(super) mode: SshKeyEditorMode,
    pub(super) name: String,
    pub(super) note: String,
    pub(super) folder_id: Option<String>,
    pub(super) source: Option<SshKeyFileSelection>,
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
    fn import(folder_id: Option<String>) -> Self {
        Self {
            form_id: uuid::Uuid::new_v4(),
            input: None,
            auto_focus: true,
            mode: SshKeyEditorMode::Import,
            name: String::new(),
            note: String::new(),
            folder_id,
            source: None,
            busy: false,
            error: None,
        }
    }

    fn edit(key: &SshKeyMetadata, folder_id: Option<String>) -> Self {
        Self {
            form_id: uuid::Uuid::new_v4(),
            input: None,
            auto_focus: true,
            mode: SshKeyEditorMode::Edit(key.id.clone()),
            name: key.name.clone(),
            note: key.note.clone().unwrap_or_default(),
            folder_id,
            source: None,
            busy: false,
            error: None,
        }
    }

    fn folder(mode: SshKeyEditorMode, name: String) -> Self {
        Self {
            form_id: uuid::Uuid::new_v4(),
            input: None,
            auto_focus: true,
            mode,
            name,
            note: String::new(),
            folder_id: None,
            source: None,
            busy: false,
            error: None,
        }
    }

    fn set_value(&mut self, value: String) {
        match self.mode {
            SshKeyEditorMode::Import | SshKeyEditorMode::Edit(_) => self.note = value,
            SshKeyEditorMode::CreateFolder | SshKeyEditorMode::RenameFolder(_) => self.name = value,
        }
        self.error = None;
    }

    pub(super) fn take_auto_focus_handle(&mut self, cx: &gpui::App) -> Option<FocusHandle> {
        if !self.auto_focus {
            return None;
        }
        self.auto_focus = false;
        self.input.as_ref().map(|input| input.focus_handle(cx))
    }
}

impl RootView {
    fn install_ssh_key_editor(&mut self, mut editor: PendingSshKeyEditor, cx: &mut Context<Self>) {
        let form_id = editor.form_id;
        let palette = ThemePalette::for_mode(self.state.read(cx).theme);
        let (value, placeholder, mode) = match editor.mode {
            SshKeyEditorMode::Import | SshKeyEditorMode::Edit(_) => {
                (editor.note.clone(), "备注信息", TextInputMode::MultiLine)
            }
            SshKeyEditorMode::CreateFolder | SshKeyEditorMode::RenameFolder(_) => {
                (editor.name.clone(), "文件夹名称", TextInputMode::SingleLine)
            }
        };
        let input = cx.new(|cx| TextInput::new(value, placeholder, mode, false, palette, cx));
        cx.subscribe(&input, move |root, _, event, cx| {
            let Some(editor) = root.pending_ssh_key_editor.as_mut() else {
                return;
            };
            if editor.form_id != form_id || editor.busy {
                return;
            }
            match event {
                TextInputEvent::Changed(value) => editor.set_value(value.clone()),
                TextInputEvent::Submit => root.save_ssh_key_editor(cx),
                TextInputEvent::Cancel => {
                    root.pending_ssh_key_editor = None;
                    cx.notify();
                }
            }
        })
        .detach();
        editor.input = Some(input);
        self.pending_ssh_key_editor = Some(editor);
        cx.notify();
    }

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
        self.install_ssh_key_editor(
            PendingSshKeyEditor::import(self.active_ssh_key_folder.clone()),
            cx,
        );
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
            let folder_id = self
                .state
                .read(cx)
                .ssh_key_layout
                .assignments
                .get(&key.id)
                .cloned();
            self.install_ssh_key_editor(PendingSshKeyEditor::edit(&key, folder_id), cx);
        }
        cx.notify();
    }

    fn begin_create_key_folder(&mut self, cx: &mut Context<Self>) {
        self.install_ssh_key_editor(
            PendingSshKeyEditor::folder(SshKeyEditorMode::CreateFolder, String::new()),
            cx,
        );
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
            self.install_ssh_key_editor(
                PendingSshKeyEditor::folder(SshKeyEditorMode::RenameFolder(folder.id), folder.name),
                cx,
            );
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

    fn save_ssh_key_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pending_ssh_key_editor.as_mut() else {
            return;
        };
        let mode = editor.mode.clone();
        let name = editor.name.trim().to_string();
        let note = editor.note.trim().to_string();
        let folder_id = editor.folder_id.clone();
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
                    Some(path) => match api.ssh_keys_import(path, note).await {
                        Ok(imported) => {
                            let mut layout = current_layout;
                            assign_key_folder(&mut layout, &imported.key.id, folder_id);
                            api.ssh_keys_save_layout(layout).await.map(|_| ())
                        }
                        Err(error) => Err(error),
                    },
                    None => Err(crate::error::AppError::Command(
                        "请选择 SSH 私钥文件。".to_string(),
                    )),
                },
                SshKeyEditorMode::Edit(key_id) => {
                    if note.is_empty() {
                        Err(crate::error::AppError::Command(
                            "密钥备注不能为空。".to_string(),
                        ))
                    } else {
                        match api.ssh_keys_update_note(key_id.clone(), note).await {
                            Ok(_) => {
                                let mut layout = current_layout;
                                assign_key_folder(&mut layout, &key_id, folder_id);
                                api.ssh_keys_save_layout(layout).await.map(|_| ())
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
                SshKeyEditorMode::CreateFolder => {
                    let mut layout = current_layout;
                    let folder_id = uuid::Uuid::new_v4().to_string();
                    let next_order = next_root_order(&layout);
                    layout.folders.push(SshKeyFolder {
                        id: folder_id.clone(),
                        name,
                    });
                    layout.item_order.insert(folder_id, next_order);
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
        self.pending_ssh_key_delete = Some(SshKeyDeleteTarget::Key { id: key_id, name });
        cx.notify();
    }

    fn request_delete_ssh_key_folder(
        &mut self,
        folder_id: String,
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_ssh_key_delete = Some(SshKeyDeleteTarget::Folder {
            id: folder_id,
            name,
        });
        cx.notify();
    }

    fn confirm_delete_ssh_key(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pending_ssh_key_delete.clone() else {
            return;
        };
        let api = self.api.clone();
        let current_layout = self.state.read(cx).ssh_key_layout.clone();
        cx.spawn(async move |this, cx| {
            let result = match target {
                SshKeyDeleteTarget::Key { id, .. } => api.ssh_keys_delete(id).await,
                SshKeyDeleteTarget::Folder { id, .. } => {
                    let mut layout = current_layout;
                    delete_folder_from_layout(&mut layout, &id);
                    api.ssh_keys_save_layout(layout).await.map(|_| ())
                }
            };
            let _ = this.update(cx, |root, cx| match result {
                Ok(()) => {
                    if let Some(SshKeyDeleteTarget::Folder { id, .. }) =
                        root.pending_ssh_key_delete.as_ref()
                    {
                        root.expanded_ssh_key_folders.remove(id);
                        if root.active_ssh_key_folder.as_ref() == Some(id) {
                            root.active_ssh_key_folder = None;
                        }
                    }
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

    fn start_ssh_key_drag(&mut self, item: SshKeyDragItem, cx: &mut Context<Self>) {
        self.ssh_key_dragging = Some(item);
        cx.notify();
    }

    fn drop_ssh_key_on(
        &mut self,
        target: SshKeyDragItem,
        position: SshKeyDropPosition,
        cx: &mut Context<Self>,
    ) {
        let Some(dragged) = self.ssh_key_dragging.take() else {
            return;
        };
        if dragged == target {
            cx.notify();
            return;
        }
        let state = self.state.read(cx);
        let mut layout = state.ssh_key_layout.clone();
        reorder_relative(&mut layout, &state.ssh_keys, &dragged, &target, position);
        self.persist_ssh_key_layout(layout, cx);
    }

    fn drop_ssh_key_into_folder(&mut self, folder_id: String, cx: &mut Context<Self>) {
        let Some(dragged) = self.ssh_key_dragging.take() else {
            return;
        };
        let SshKeyDragItem::Key(key_id) = dragged else {
            cx.notify();
            return;
        };
        let state = self.state.read(cx);
        let mut layout = state.ssh_key_layout.clone();
        assign_key_folder(&mut layout, &key_id, Some(folder_id.clone()));
        self.expanded_ssh_key_folders.insert(folder_id);
        self.persist_ssh_key_layout(layout, cx);
    }

    fn drop_ssh_key_on_root(&mut self, cx: &mut Context<Self>) {
        let Some(SshKeyDragItem::Key(key_id)) = self.ssh_key_dragging.take() else {
            self.ssh_key_dragging = None;
            cx.notify();
            return;
        };
        let state = self.state.read(cx);
        let mut layout = state.ssh_key_layout.clone();
        layout.assignments.remove(&key_id);
        let next_order = root_items(&layout, &state.ssh_keys)
            .into_iter()
            .filter(|id| id != &key_id)
            .enumerate()
            .map(|(index, id)| {
                layout.item_order.insert(id, ((index + 1) * 1000) as u64);
                index
            })
            .last()
            .map_or(1000, |index| ((index + 2) * 1000) as u64);
        layout.item_order.insert(key_id, next_order);
        self.persist_ssh_key_layout(layout, cx);
    }

    pub(super) fn cancel_ssh_key_drag(&mut self, cx: &mut Context<Self>) {
        if self.ssh_key_dragging.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_ssh_key_folder(&mut self, folder_id: String, cx: &mut Context<Self>) {
        if !self.expanded_ssh_key_folders.remove(&folder_id) {
            self.expanded_ssh_key_folders.insert(folder_id);
        }
        cx.notify();
    }

    fn persist_ssh_key_layout(&mut self, mut layout: SshKeyLayout, cx: &mut Context<Self>) {
        normalize_layout(&mut layout, &self.state.read(cx).ssh_keys);
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
        let rows = if active_folder.is_some() {
            keys.into_iter()
                .enumerate()
                .map(|(index, key)| self.render_ssh_key_row(index, key, state, palette, cx))
                .collect::<Vec<_>>()
        } else {
            let visible_key_ids = keys
                .iter()
                .map(|key| key.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            let visible_folder_ids = state
                .ssh_key_layout
                .folders
                .iter()
                .filter(|folder| query.is_empty() || folder.name.to_lowercase().contains(&query))
                .map(|folder| folder.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            let mut rows = Vec::new();
            let mut row_index = 0usize;
            for item_id in root_items(&state.ssh_key_layout, &state.ssh_keys) {
                if let Some(folder) = state
                    .ssh_key_layout
                    .folders
                    .iter()
                    .find(|folder| folder.id == item_id)
                {
                    if !visible_folder_ids.contains(folder.id.as_str()) {
                        continue;
                    }
                    rows.push(self.render_ssh_key_folder_row(
                        row_index,
                        folder.clone(),
                        state,
                        palette,
                        cx,
                    ));
                    row_index += 1;
                    if self.expanded_ssh_key_folders.contains(&folder.id) {
                        let mut folder_keys = keys
                            .iter()
                            .filter(|key| {
                                state.ssh_key_layout.assignments.get(&key.id) == Some(&folder.id)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        folder_keys.sort_by_key(|key| {
                            state
                                .ssh_key_layout
                                .item_order
                                .get(&key.id)
                                .copied()
                                .unwrap_or(key.imported_at)
                        });
                        if folder_keys.is_empty() {
                            rows.push(
                                div()
                                    .min_h(px(42.0))
                                    .flex()
                                    .items_center()
                                    .pl_8()
                                    .border_b_1()
                                    .border_color(palette.border)
                                    .text_xs()
                                    .text_color(palette.text_soft)
                                    .child("空文件夹")
                                    .into_any_element(),
                            );
                            row_index += 1;
                        }
                        for key in folder_keys {
                            rows.push(self.render_ssh_key_row(row_index, key, state, palette, cx));
                            row_index += 1;
                        }
                    }
                } else if visible_key_ids.contains(item_id.as_str()) {
                    if let Some(key) = keys.iter().find(|key| key.id == item_id).cloned() {
                        rows.push(self.render_ssh_key_row(row_index, key, state, palette, cx));
                        row_index += 1;
                    }
                }
            }
            rows
        };
        let has_rows = !rows.is_empty();

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
            .when_some(state.data_error.clone(), |view, error| {
                view.child(
                    div()
                        .id("dismiss-ssh-key-error")
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(palette.background)
                        .border_1()
                        .border_color(palette.danger)
                        .text_xs()
                        .text_color(palette.danger)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_state(cx, |state| state.data_error = None);
                        }))
                        .child(error)
                        .child("关闭"),
                )
            })
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
                            .when(!has_rows, |view| {
                                view.flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .child(if query.is_empty() {
                                        "尚未导入私钥。导入后可在 SSH 连接中复用。"
                                    } else {
                                        "没有匹配的密钥或文件夹。"
                                    })
                            })
                            .when(has_rows, |view| view.children(rows)),
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
        let mut folders = state.ssh_key_layout.folders.clone();
        folders.sort_by_key(|folder| {
            state
                .ssh_key_layout
                .item_order
                .get(&folder.id)
                .copied()
                .unwrap_or(u64::MAX)
        });
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
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.drop_ssh_key_on_root(cx)),
                    )
                    .child("全部密钥")
                    .child(state.ssh_keys.len().to_string()),
            )
            .children(folders.into_iter().enumerate().map(|(index, folder)| {
                let folder_id = folder.id.clone();
                let drop_folder_id = folder.id.clone();
                let rename_id = folder.id.clone();
                let delete_id = folder.id.clone();
                let delete_name = folder.name.clone();
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
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.drop_ssh_key_into_folder(drop_folder_id.clone(), cx)
                        }),
                    )
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
                                this.request_delete_ssh_key_folder(
                                    delete_id.clone(),
                                    delete_name.clone(),
                                    cx,
                                )
                            }))
                            .child("删"),
                    )
            }))
            .into_any_element()
    }

    fn render_ssh_key_folder_row(
        &self,
        index: usize,
        folder: SshKeyFolder,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded_ssh_key_folders.contains(&folder.id);
        let count = state
            .ssh_key_layout
            .assignments
            .values()
            .filter(|assigned| *assigned == &folder.id)
            .count();
        let drag_id = folder.id.clone();
        let before_id = folder.id.clone();
        let drop_id = folder.id.clone();
        let inside_id = folder.id.clone();
        let toggle_id = folder.id.clone();
        let rename_id = folder.id.clone();
        let delete_id = folder.id.clone();
        let delete_name = folder.name.clone();

        div()
            .id(("ssh-key-folder-row", index))
            .min_h(px(54.0))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_ssh_key_drag(SshKeyDragItem::Folder(drag_id.clone()), cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.drop_ssh_key_on(
                        SshKeyDragItem::Folder(drop_id.clone()),
                        SshKeyDropPosition::After,
                        cx,
                    )
                }),
            )
            .child(
                div()
                    .id(("before-ssh-key-folder", index))
                    .w(px(14.0))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(palette.text_soft)
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.drop_ssh_key_on(
                                SshKeyDragItem::Folder(before_id.clone()),
                                SshKeyDropPosition::Before,
                                cx,
                            )
                        }),
                    )
                    .child("⋮"),
            )
            .child(
                div()
                    .id(("toggle-ssh-key-folder", index))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.drop_ssh_key_into_folder(inside_id.clone(), cx)
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_ssh_key_folder(toggle_id.clone(), cx)
                    }))
                    .child(
                        div()
                            .w(px(16.0))
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child(if expanded { "▼" } else { "▶" }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(palette.text)
                            .child(folder.name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(format!("{count} 个密钥")),
                    ),
            )
            .child(
                div()
                    .w(px(180.0))
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child("—"),
            )
            .child(
                div()
                    .w(px(140.0))
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child("—"),
            )
            .child(
                div()
                    .w(px(100.0))
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child("—"),
            )
            .child(
                div()
                    .w(px(40.0))
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child("—"),
            )
            .child(
                div()
                    .w(px(150.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id(("rename-inline-ssh-key-folder", index))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.begin_rename_key_folder(rename_id.clone(), cx)
                            }))
                            .child("重命名"),
                    )
                    .child(
                        div()
                            .id(("delete-inline-ssh-key-folder", index))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.danger)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_delete_ssh_key_folder(
                                    delete_id.clone(),
                                    delete_name.clone(),
                                    cx,
                                )
                            }))
                            .child("删除"),
                    ),
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
        let nested = self.active_ssh_key_folder.is_none()
            && state.ssh_key_layout.assignments.contains_key(&key.id);
        let edit_id = key.id.clone();
        let drag_id = key.id.clone();
        let before_id = key.id.clone();
        let drop_id = key.id.clone();
        let delete_id = key.id.clone();
        let delete_name = key.name.clone();
        div()
            .id(("ssh-key-row", index))
            .min_h(px(72.0))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .when(nested, |row| row.pl_8())
            .border_b_1()
            .border_color(palette.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_ssh_key_drag(SshKeyDragItem::Key(drag_id.clone()), cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.drop_ssh_key_on(
                        SshKeyDragItem::Key(drop_id.clone()),
                        SshKeyDropPosition::After,
                        cx,
                    )
                }),
            )
            .child(
                div()
                    .id(("before-ssh-key", index))
                    .w(px(14.0))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(palette.text_soft)
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.drop_ssh_key_on(
                                SshKeyDragItem::Key(before_id.clone()),
                                SshKeyDropPosition::Before,
                                cx,
                            )
                        }),
                    )
                    .child("⋮"),
            )
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
        let input = editor
            .input
            .clone()
            .expect("SSH key editor input initialized");
        input.update(cx, |input, _| input.set_palette(palette));
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
                    .child(div().text_xs().text_color(palette.text_soft).child(
                        if is_import || is_edit {
                            "支持中文输入；Enter 换行，Cmd/Ctrl+Enter 保存，Esc 取消。"
                        } else {
                            "支持中文输入；Enter 保存，Esc 取消。"
                        },
                    ))
                    .when(is_edit, |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_xs().text_color(palette.text_muted).child("名称"))
                                .child(
                                    div()
                                        .h(px(38.0))
                                        .flex()
                                        .items_center()
                                        .px_3()
                                        .rounded_md()
                                        .bg(palette.background)
                                        .border_1()
                                        .border_color(palette.border)
                                        .text_sm()
                                        .text_color(palette.text_soft)
                                        .child(editor.name.clone()),
                                ),
                        )
                    })
                    .when(!is_import && !is_edit, |view| {
                        view.child(key_input("文件夹名称", input.clone(), palette))
                    })
                    .when(is_import || is_edit, |view| {
                        view.child(key_input("备注信息", input.clone(), palette))
                    })
                    .when(is_import || is_edit, |view| {
                        let folders = self.state.read(cx).ssh_key_layout.folders.clone();
                        let selected_folder = editor.folder_id.clone();
                        view.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.text_muted)
                                        .child("所属文件夹"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .child(folder_option(
                                            "根目录（未分类）".to_string(),
                                            None,
                                            selected_folder.is_none(),
                                            0,
                                            palette,
                                            cx,
                                        ))
                                        .children(folders.into_iter().enumerate().map(
                                            |(index, folder)| {
                                                let selected =
                                                    selected_folder.as_ref() == Some(&folder.id);
                                                folder_option(
                                                    folder.name,
                                                    Some(folder.id),
                                                    selected,
                                                    index + 1,
                                                    palette,
                                                    cx,
                                                )
                                            },
                                        )),
                                ),
                        )
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
        target: SshKeyDeleteTarget,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, description) = match &target {
            SshKeyDeleteTarget::Key { name, .. } => (
                "删除密钥",
                format!("确定删除 {name} 吗？此操作只删除 FileTerm 管理的副本，不会删除原始文件。"),
            ),
            SshKeyDeleteTarget::Folder { name, .. } => (
                "删除文件夹",
                format!("确定删除 {name} 吗？文件夹内的密钥会移回根目录，不会被删除。"),
            ),
        };
        let expected_target = target.clone();
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
                    .child(div().text_lg().text_color(palette.text).child(title))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(description),
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
                                        if this.pending_ssh_key_delete.as_ref()
                                            == Some(&expected_target)
                                        {
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

fn folder_option(
    label: String,
    folder_id: Option<String>,
    selected: bool,
    index: usize,
    palette: ThemePalette,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    div()
        .id(("ssh-key-folder-option", index))
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .bg(if selected {
            palette.surface_active
        } else {
            palette.background
        })
        .border_1()
        .border_color(if selected {
            palette.accent
        } else {
            palette.border
        })
        .text_xs()
        .text_color(if selected {
            palette.text
        } else {
            palette.text_muted
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(editor) = this.pending_ssh_key_editor.as_mut() {
                editor.folder_id = folder_id.clone();
                editor.error = None;
            }
            cx.notify();
        }))
        .child(label)
}

fn key_input(
    label: &'static str,
    input: Entity<TextInput>,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(palette.text_muted).child(label))
        .child(input)
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
