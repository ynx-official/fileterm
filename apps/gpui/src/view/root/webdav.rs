use gpui::{div, prelude::*, px, Context, IntoElement, KeyDownEvent};
use serde_json::Value;
use zeroize::Zeroize;

use super::RootView;
use crate::theme::ThemePalette;

#[derive(Clone)]
pub(super) struct PendingWebDavEditor {
    pub(super) enabled: bool,
    pub(super) allow_insecure_tls: bool,
    pub(super) url: String,
    pub(super) username: String,
    pub(super) password: String,
    pub(super) remote_path: String,
    pub(super) active_field: usize,
}

impl Drop for PendingWebDavEditor {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

impl RootView {
    pub(super) fn open_webdav_editor(&mut self, cx: &mut Context<Self>) {
        let config = self
            .webdav_config
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        self.pending_webdav_editor = Some(PendingWebDavEditor {
            enabled: config
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            allow_insecure_tls: config
                .get("allowInsecureTls")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            url: config
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            username: config
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            password: String::new(),
            remote_path: config
                .get("remotePath")
                .and_then(Value::as_str)
                .unwrap_or("fileterm-connections.json")
                .to_string(),
            active_field: 0,
        });
        cx.notify();
    }

    pub(super) fn handle_webdav_editor_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.pending_webdav_editor.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.pending_webdav_editor = None;
                cx.notify();
            }
            "tab" => {
                editor.active_field = (editor.active_field + 1) % 4;
                cx.notify();
            }
            "enter" | "return" => self.save_webdav_config(cx),
            "backspace" => {
                active_value(editor).pop();
                cx.notify();
            }
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    active_value(editor).push_str(text);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn save_webdav_config(&mut self, cx: &mut Context<Self>) {
        let Some(mut editor) = self.pending_webdav_editor.take() else {
            return;
        };
        self.webdav_busy = true;
        self.webdav_message = None;
        let mut input = serde_json::json!({
            "enabled": editor.enabled,
            "allowInsecureTls": editor.allow_insecure_tls,
            "url": editor.url,
            "username": editor.username,
            "remotePath": editor.remote_path,
        });
        if !editor.password.is_empty() {
            input["password"] = Value::String(std::mem::take(&mut editor.password));
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.webdav_save_config(input).await;
            let _ = this.update(cx, |root, cx| {
                root.webdav_busy = false;
                match result {
                    Ok(config) => {
                        root.webdav_config = Some(config);
                        root.webdav_message = Some("WebDAV 配置已保存".to_string());
                    }
                    Err(error) => root.webdav_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_webdav_sync(&mut self, upload: bool, cx: &mut Context<Self>) {
        if self.webdav_busy {
            return;
        }
        self.webdav_busy = true;
        self.webdav_message = None;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = if upload {
                api.webdav_upload().await
            } else {
                api.webdav_download().await
            };
            let succeeded = result.is_ok();
            let config = if succeeded {
                api.webdav_get_config().await.ok()
            } else {
                None
            };
            let _ = this.update(cx, |root, cx| {
                root.webdav_busy = false;
                if let Some(config) = config {
                    root.webdav_config = Some(config);
                }
                root.webdav_message = Some(match result {
                    Ok(value) => value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("WebDAV 同步完成")
                        .to_string(),
                    Err(error) => error.to_string(),
                });
                if !upload && succeeded {
                    root.reload_connection_library(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_webdav_settings(
        &self,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let config = self
            .webdav_config
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let enabled = config
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let url = config
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("未配置");
        let remote_path = config
            .get("remotePath")
            .and_then(Value::as_str)
            .unwrap_or("fileterm-connections.json");
        let last_synced = config
            .get("lastSyncedAt")
            .and_then(Value::as_str)
            .unwrap_or("尚未同步");
        let busy = self.webdav_busy;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_5()
            .rounded_lg()
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .child(
                div()
                    .text_sm()
                    .text_color(palette.text)
                    .child("WebDAV 配置同步"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_soft)
                    .child("手动同步完整连接配置包；远端写入使用 ETag 冲突保护。"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(format!(
                        "状态：{}",
                        if enabled { "已启用" } else { "未启用" }
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(format!("地址：{url}")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(format!("远端文件：{remote_path}")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(format!("上次同步：{last_synced}")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(action(
                        "编辑配置",
                        "webdav-edit",
                        false,
                        palette,
                        cx,
                        |this, cx| this.open_webdav_editor(cx),
                    ))
                    .child(action(
                        "上传",
                        "webdav-upload",
                        busy || !enabled,
                        palette,
                        cx,
                        |this, cx| this.run_webdav_sync(true, cx),
                    ))
                    .child(action(
                        "下载",
                        "webdav-download",
                        busy || !enabled,
                        palette,
                        cx,
                        |this, cx| this.run_webdav_sync(false, cx),
                    )),
            )
            .when_some(self.webdav_message.clone(), |view, message| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(
                            if message.contains("失败")
                                || message.contains("error")
                                || message.contains("冲突")
                            {
                                palette.danger
                            } else {
                                palette.text_soft
                            },
                        )
                        .child(message),
                )
            })
    }

    pub(super) fn render_webdav_editor(
        &self,
        editor: PendingWebDavEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let enabled = editor.enabled;
        let insecure = editor.allow_insecure_tls;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .child(
                div()
                    .w(px(620.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette.text)
                            .child("WebDAV 配置"),
                    )
                    .child(
                        div().text_xs().text_color(palette.text_soft).child(
                            "Tab 切换输入项，Enter 保存，Esc 取消。密码留空会保留已保存凭据。",
                        ),
                    )
                    .child(input(
                        "地址",
                        &editor.url,
                        0,
                        editor.active_field,
                        false,
                        palette,
                        cx,
                    ))
                    .child(input(
                        "用户名",
                        &editor.username,
                        1,
                        editor.active_field,
                        false,
                        palette,
                        cx,
                    ))
                    .child(input(
                        "密码",
                        &editor.password,
                        2,
                        editor.active_field,
                        true,
                        palette,
                        cx,
                    ))
                    .child(input(
                        "远端路径",
                        &editor.remote_path,
                        3,
                        editor.active_field,
                        false,
                        palette,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(action(
                                if enabled {
                                    "同步已启用"
                                } else {
                                    "同步未启用"
                                },
                                "webdav-toggle-enabled",
                                false,
                                palette,
                                cx,
                                |this, cx| {
                                    if let Some(editor) = this.pending_webdav_editor.as_mut() {
                                        editor.enabled = !editor.enabled;
                                    }
                                    cx.notify();
                                },
                            ))
                            .child(action(
                                if insecure { "允许 HTTP" } else { "仅 HTTPS" },
                                "webdav-toggle-insecure",
                                false,
                                palette,
                                cx,
                                |this, cx| {
                                    if let Some(editor) = this.pending_webdav_editor.as_mut() {
                                        editor.allow_insecure_tls = !editor.allow_insecure_tls;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(action(
                                "取消",
                                "webdav-cancel",
                                false,
                                palette,
                                cx,
                                |this, cx| {
                                    this.pending_webdav_editor = None;
                                    cx.notify();
                                },
                            ))
                            .child(action(
                                "保存",
                                "webdav-save",
                                false,
                                palette,
                                cx,
                                |this, cx| this.save_webdav_config(cx),
                            )),
                    ),
            )
    }
}

fn active_value(editor: &mut PendingWebDavEditor) -> &mut String {
    match editor.active_field {
        0 => &mut editor.url,
        1 => &mut editor.username,
        2 => &mut editor.password,
        _ => &mut editor.remote_path,
    }
}

fn input(
    label: &'static str,
    value: &str,
    field: usize,
    active_field: usize,
    secret: bool,
    palette: ThemePalette,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let display = if secret && !value.is_empty() {
        "•".repeat(value.chars().count())
    } else if value.is_empty() {
        "输入内容".to_string()
    } else {
        value.to_string()
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(palette.text_muted).child(label))
        .child(
            div()
                .id(("webdav-input", field))
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
                .text_color(if display == "输入内容" {
                    palette.text_soft
                } else {
                    palette.text
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.pending_webdav_editor.as_mut() {
                        editor.active_field = field;
                    }
                    cx.notify();
                }))
                .child(display),
        )
}

fn action(
    label: &'static str,
    id: &'static str,
    disabled: bool,
    palette: ThemePalette,
    cx: &mut Context<RootView>,
    callback: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .text_xs()
        .text_color(if disabled {
            palette.text_soft
        } else {
            palette.accent
        })
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(move |style| style.bg(palette.accent_surface))
                .on_click(cx.listener(move |this, _, _, cx| callback(this, cx)))
        })
        .child(label)
}
