use gpui::{div, prelude::*, px, Context, Entity, IntoElement};
use serde_json::Value;
use zeroize::Zeroize;

use super::RootView;
use crate::{
    theme::ThemePalette,
    view::text_editor::{TextInput, TextInputEvent, TextInputMode},
};

#[derive(Clone)]
struct WebDavInputs {
    url: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    remote_path: Entity<TextInput>,
}

#[derive(Clone)]
pub(super) struct PendingWebDavEditor {
    pub(super) enabled: bool,
    pub(super) allow_insecure_tls: bool,
    pub(super) url: String,
    pub(super) username: String,
    pub(super) password: String,
    pub(super) remote_path: String,
    inputs: WebDavInputs,
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
        let enabled = config
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let allow_insecure_tls = config
            .get("allowInsecureTls")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let url = config
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let username = config
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let remote_path = config
            .get("remotePath")
            .and_then(Value::as_str)
            .unwrap_or("fileterm-connections.json")
            .to_string();
        let palette = ThemePalette::for_mode(self.state.read(cx).theme);
        let create_input =
            |value: &str, placeholder: &'static str, secret: bool, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    TextInput::new(
                        value,
                        placeholder,
                        TextInputMode::SingleLine,
                        secret,
                        palette,
                        cx,
                    )
                })
            };
        let inputs = WebDavInputs {
            url: create_input(&url, "https://example.com/dav/", false, cx),
            username: create_input(&username, "用户名", false, cx),
            password: create_input("", "留空保留已保存密码", true, cx),
            remote_path: create_input(&remote_path, "fileterm-connections.json", false, cx),
        };
        for (field, input) in [
            (0usize, inputs.url.clone()),
            (1, inputs.username.clone()),
            (2, inputs.password.clone()),
            (3, inputs.remote_path.clone()),
        ] {
            cx.subscribe(&input, move |root, _, event, cx| match event {
                TextInputEvent::Changed(value) => {
                    if let Some(editor) = root.pending_webdav_editor.as_mut() {
                        match field {
                            0 => editor.url = value.clone(),
                            1 => editor.username = value.clone(),
                            2 => editor.password = value.clone(),
                            _ => editor.remote_path = value.clone(),
                        }
                    }
                }
                TextInputEvent::Submit | TextInputEvent::Save => root.save_webdav_config(cx),
                TextInputEvent::Cancel => {
                    root.pending_webdav_editor = None;
                    cx.notify();
                }
            })
            .detach();
        }
        inputs.url.update(cx, |input, cx| input.request_focus(cx));
        self.pending_webdav_editor = Some(PendingWebDavEditor {
            enabled,
            allow_insecure_tls,
            url,
            username,
            password: String::new(),
            remote_path,
            inputs,
        });
        cx.notify();
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
                    .child(input("地址", editor.inputs.url.clone(), palette))
                    .child(input("用户名", editor.inputs.username.clone(), palette))
                    .child(input("密码", editor.inputs.password.clone(), palette))
                    .child(input(
                        "远端路径",
                        editor.inputs.remote_path.clone(),
                        palette,
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

fn input(label: &'static str, input: Entity<TextInput>, palette: ThemePalette) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(palette.text_muted).child(label))
        .child(input)
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
