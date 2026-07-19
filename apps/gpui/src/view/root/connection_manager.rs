use gpui::{div, prelude::*, px, AnyElement, Context, KeyDownEvent};
use serde_json::{json, Value};
use zeroize::Zeroize;

use super::RootView;
use crate::{state::AppState, theme::ThemePalette};

#[derive(Clone)]
pub(super) struct PendingConnectionEditor {
    profile_id: Option<String>,
    protocol: String,
    name: String,
    host: String,
    port: String,
    username: String,
    password: String,
    group: String,
    remote_path: String,
    auth_type: String,
    private_key_id: String,
    private_key_path: String,
    passphrase: String,
    security_mode: String,
    device_path: String,
    baud_rate: String,
    data_bits: String,
    stop_bits: String,
    parity: String,
    flow_control: String,
    active_field: usize,
    busy: bool,
    error: Option<String>,
    delete_confirmation: bool,
}

impl Drop for PendingConnectionEditor {
    fn drop(&mut self) {
        self.password.zeroize();
        self.passphrase.zeroize();
        self.private_key_path.zeroize();
    }
}

impl PendingConnectionEditor {
    pub(super) fn new() -> Self {
        Self {
            profile_id: None,
            protocol: "ssh".to_string(),
            name: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            password: String::new(),
            group: "默认".to_string(),
            remote_path: "/".to_string(),
            auth_type: "password".to_string(),
            private_key_id: String::new(),
            private_key_path: String::new(),
            passphrase: String::new(),
            security_mode: "none".to_string(),
            device_path: String::new(),
            baud_rate: "115200".to_string(),
            data_bits: "8".to_string(),
            stop_bits: "1".to_string(),
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            active_field: 0,
            busy: false,
            error: None,
            delete_confirmation: false,
        }
    }

    fn from_profile(profile: &Value) -> Self {
        let mut editor = Self::new();
        editor.profile_id = profile
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        editor.protocol = text(profile, "type", "ssh");
        editor.name = text(profile, "name", "");
        editor.host = text(profile, "host", "");
        editor.port = profile
            .get("port")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| default_port(&editor.protocol))
            .to_string();
        editor.username = text(profile, "username", "");
        editor.group = text(profile, "group", "默认");
        editor.remote_path = text(profile, "remotePath", "/");
        editor.auth_type = text(profile, "authType", "password");
        editor.private_key_id = text(profile, "privateKeyId", "");
        editor.security_mode = text(profile, "securityMode", "none");
        editor.device_path = text(profile, "devicePath", "");
        editor.baud_rate = profile
            .get("baudRate")
            .and_then(Value::as_u64)
            .unwrap_or(115_200)
            .to_string();
        editor.data_bits = profile
            .get("dataBits")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .to_string();
        editor.stop_bits = profile
            .get("stopBits")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .to_string();
        editor.parity = text(profile, "parity", "none");
        editor.flow_control = text(profile, "flowControl", "none");
        editor
    }

    fn field_count(&self) -> usize {
        match self.protocol.as_str() {
            "serial" => 8,
            "ssh" if self.auth_type == "privateKey" => 9,
            _ => 7,
        }
    }

    fn active_value(&mut self) -> &mut String {
        match self.protocol.as_str() {
            "serial" => match self.active_field {
                0 => &mut self.name,
                1 => &mut self.device_path,
                2 => &mut self.baud_rate,
                3 => &mut self.data_bits,
                4 => &mut self.stop_bits,
                5 => &mut self.parity,
                6 => &mut self.flow_control,
                _ => &mut self.group,
            },
            "ssh" if self.auth_type == "privateKey" => match self.active_field {
                0 => &mut self.name,
                1 => &mut self.host,
                2 => &mut self.port,
                3 => &mut self.username,
                4 => &mut self.private_key_path,
                5 => &mut self.passphrase,
                6 => &mut self.group,
                7 => &mut self.remote_path,
                _ => &mut self.password,
            },
            _ => match self.active_field {
                0 => &mut self.name,
                1 => &mut self.host,
                2 => &mut self.port,
                3 => &mut self.username,
                4 => &mut self.password,
                5 => &mut self.group,
                _ => &mut self.remote_path,
            },
        }
    }

    fn to_input(&self) -> Result<Value, String> {
        let mut input = json!({
            "type": self.protocol,
            "name": self.name.trim(),
            "group": self.group.trim(),
        });
        if self.protocol == "serial" {
            let baud_rate = self
                .baud_rate
                .parse::<u32>()
                .map_err(|_| "串口波特率无效".to_string())?;
            let data_bits = self
                .data_bits
                .parse::<u8>()
                .ok()
                .filter(|value| matches!(value, 5..=8))
                .ok_or_else(|| "串口数据位必须是 5、6、7 或 8".to_string())?;
            let stop_bits = self
                .stop_bits
                .parse::<u8>()
                .ok()
                .filter(|value| matches!(value, 1 | 2))
                .ok_or_else(|| "串口停止位必须是 1 或 2".to_string())?;
            if !matches!(self.parity.as_str(), "none" | "odd" | "even") {
                return Err("串口校验位必须是 none、odd 或 even".to_string());
            }
            if !matches!(self.flow_control.as_str(), "none" | "software" | "hardware") {
                return Err("串口流控必须是 none、software 或 hardware".to_string());
            }
            input["devicePath"] = Value::String(self.device_path.trim().to_string());
            input["baudRate"] = Value::Number(baud_rate.into());
            input["dataBits"] = Value::Number(data_bits.into());
            input["stopBits"] = Value::Number(stop_bits.into());
            input["parity"] = Value::String(self.parity.clone());
            input["flowControl"] = Value::String(self.flow_control.clone());
            return Ok(input);
        }

        let port = self
            .port
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| "端口必须在 1 到 65535 之间".to_string())?;
        input["host"] = Value::String(self.host.trim().to_string());
        input["port"] = Value::Number(port.into());
        input["username"] = Value::String(self.username.trim().to_string());
        input["password"] = Value::String(self.password.clone());
        input["remotePath"] = Value::String(self.remote_path.trim().to_string());
        if self.protocol == "ssh" {
            if self.auth_type == "privateKey"
                && self.private_key_id.is_empty()
                && self.private_key_path.trim().is_empty()
            {
                return Err("请选择托管密钥或填写私钥路径".to_string());
            }
            input["authType"] = Value::String(self.auth_type.clone());
            input["privateKeyId"] = if self.private_key_id.is_empty() {
                Value::Null
            } else {
                Value::String(self.private_key_id.clone())
            };
            input["privateKeyPath"] = Value::String(self.private_key_path.clone());
            input["passphrase"] = Value::String(self.passphrase.clone());
        } else if self.protocol == "ftp" {
            input["securityMode"] = Value::String(self.security_mode.clone());
        }
        Ok(input)
    }
}

impl RootView {
    pub(super) fn begin_new_connection(&mut self, cx: &mut Context<Self>) {
        self.pending_connection_editor = Some(PendingConnectionEditor::new());
        cx.notify();
    }

    fn edit_connection(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let profile = self
            .state
            .read(cx)
            .connection_profiles
            .iter()
            .find(|profile| profile.get("id").and_then(Value::as_str) == Some(&profile_id))
            .cloned();
        match profile {
            Some(profile) => {
                self.pending_connection_editor =
                    Some(PendingConnectionEditor::from_profile(&profile))
            }
            None => self.update_state(cx, |state| {
                state.data_error = Some("连接配置不存在，请刷新后重试".to_string());
            }),
        }
        cx.notify();
    }

    pub(super) fn handle_connection_editor_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.pending_connection_editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                self.pending_connection_editor = None;
                cx.notify();
            }
            "tab" => {
                editor.active_field = (editor.active_field + 1) % editor.field_count();
                cx.notify();
            }
            "enter" | "return" => self.save_connection(cx),
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

    fn save_connection(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pending_connection_editor.as_mut() else {
            return;
        };
        let input = match editor.to_input() {
            Ok(input) => input,
            Err(error) => {
                editor.error = Some(error);
                cx.notify();
                return;
            }
        };
        editor.busy = true;
        editor.error = None;
        let profile_id = editor.profile_id.clone();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = match profile_id {
                Some(profile_id) => api.app_update_connection_profile(profile_id, input).await,
                None => api.app_create_connection_profile(input).await,
            };
            let _ = this.update(cx, |root, cx| match result {
                Ok(_) => {
                    root.pending_connection_editor = None;
                    root.reload_connection_library(cx);
                }
                Err(error) => {
                    if let Some(editor) = root.pending_connection_editor.as_mut() {
                        editor.busy = false;
                        editor.error = Some(error.to_string());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn delete_connection(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pending_connection_editor.as_mut() else {
            return;
        };
        let Some(profile_id) = editor.profile_id.clone() else {
            return;
        };
        if !editor.delete_confirmation {
            editor.delete_confirmation = true;
            cx.notify();
            return;
        }
        editor.busy = true;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.app_delete_connection_profile(profile_id).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(()) => {
                    root.pending_connection_editor = None;
                    root.reload_connection_library(cx);
                }
                Err(error) => {
                    if let Some(editor) = root.pending_connection_editor.as_mut() {
                        editor.busy = false;
                        editor.delete_confirmation = false;
                        editor.error = Some(error.to_string());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn render_connection_manager(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                                    .child("连接管理器"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .child(format!(
                                        "{} 个连接，{} 个文件夹",
                                        state.connections.len(),
                                        state.connection_folders.len()
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(action_button(
                                "刷新",
                                "refresh-connections",
                                false,
                                palette,
                                cx,
                                |this, cx| this.reload_connection_library(cx),
                            ))
                            .child(action_button(
                                "新建连接",
                                "create-connection",
                                true,
                                palette,
                                cx,
                                |this, cx| this.begin_new_connection(cx),
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .when(state.connections.is_empty(), |view| {
                        view.flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child("连接库为空，使用“新建连接”创建第一个配置。")
                    })
                    .when(!state.connections.is_empty(), |view| {
                        view.flex()
                            .flex_col()
                            .children(state.connections.iter().enumerate().map(
                                |(index, connection)| {
                                    let edit_id = connection.id.clone();
                                    let connect_id = connection.id.clone();
                                    let title = connection.name.clone();
                                    let protocol = connection.protocol.clone();
                                    div()
                                        .id(("managed-connection", index))
                                        .min_h(px(64.0))
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .px_4()
                                        .border_b_1()
                                        .border_color(palette.border)
                                        .child(
                                            div()
                                                .w(px(58.0))
                                                .text_xs()
                                                .text_color(palette.accent)
                                                .child(connection.protocol.to_uppercase()),
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
                                                        .child(connection.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(palette.text_soft)
                                                        .child(format!(
                                                            "{} · {}",
                                                            connection.endpoint, connection.group
                                                        )),
                                                ),
                                        )
                                        .child(action_button(
                                            "连接",
                                            ("connect-profile", index),
                                            false,
                                            palette,
                                            cx,
                                            move |this, cx| match protocol.as_str() {
                                                "ssh" => this.open_ssh_profile(
                                                    connect_id.clone(),
                                                    title.clone(),
                                                    cx,
                                                ),
                                                "ftp" => this.open_ftp_profile(
                                                    connect_id.clone(),
                                                    title.clone(),
                                                    cx,
                                                ),
                                                "telnet" | "serial" => this.open_stream_profile(
                                                    connect_id.clone(),
                                                    protocol.clone(),
                                                    title.clone(),
                                                    cx,
                                                ),
                                                _ => this.update_state(cx, |state| {
                                                    state.data_error = Some(format!(
                                                        "不支持的连接类型: {protocol}"
                                                    ))
                                                }),
                                            },
                                        ))
                                        .child(action_button(
                                            "编辑",
                                            ("edit-profile", index),
                                            false,
                                            palette,
                                            cx,
                                            move |this, cx| {
                                                this.edit_connection(edit_id.clone(), cx)
                                            },
                                        ))
                                },
                            ))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_connection_editor(
        &self,
        editor: PendingConnectionEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = editor.profile_id.is_some();
        let mut fields = vec![("名称", editor.name.clone(), false)];
        if editor.protocol == "serial" {
            fields.extend([
                ("设备路径", editor.device_path.clone(), false),
                ("波特率", editor.baud_rate.clone(), false),
                ("数据位（5/6/7/8）", editor.data_bits.clone(), false),
                ("停止位（1/2）", editor.stop_bits.clone(), false),
                ("校验位（none/odd/even）", editor.parity.clone(), false),
                (
                    "流控（none/software/hardware）",
                    editor.flow_control.clone(),
                    false,
                ),
                ("分组", editor.group.clone(), false),
            ]);
        } else {
            fields.extend([
                ("主机", editor.host.clone(), false),
                ("端口", editor.port.clone(), false),
                ("用户名", editor.username.clone(), false),
            ]);
            if editor.protocol == "ssh" && editor.auth_type == "privateKey" {
                fields.extend([
                    ("私钥路径", editor.private_key_path.clone(), false),
                    ("私钥口令", editor.passphrase.clone(), true),
                    ("分组", editor.group.clone(), false),
                    ("远端路径", editor.remote_path.clone(), false),
                    ("备用密码", editor.password.clone(), true),
                ]);
            } else {
                fields.extend([
                    ("密码", editor.password.clone(), true),
                    ("分组", editor.group.clone(), false),
                    ("远端路径", editor.remote_path.clone(), false),
                ]);
            }
        }

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .child(
                div()
                    .w(px(680.0))
                    .max_h(px(660.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(div().text_lg().text_color(palette.text).child(if editing {
                        "编辑连接"
                    } else {
                        "新建连接"
                    }))
                    .child(
                        div().text_xs().text_color(palette.text_soft).child(
                            "Tab 切换输入项，Enter 保存，Esc 取消。密码留空会保留已保存凭据。",
                        ),
                    )
                    .child(
                        div().flex().gap_2().children(
                            ["ssh", "ftp", "telnet", "serial"]
                                .into_iter()
                                .enumerate()
                                .map(|(index, protocol)| {
                                    let active = editor.protocol == protocol;
                                    div()
                                        .id(("connection-protocol", index))
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(if active {
                                            palette.accent_surface
                                        } else {
                                            palette.background
                                        })
                                        .border_1()
                                        .border_color(if active {
                                            palette.accent
                                        } else {
                                            palette.border
                                        })
                                        .text_xs()
                                        .text_color(if active {
                                            palette.accent
                                        } else {
                                            palette.text_muted
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(editor) =
                                                this.pending_connection_editor.as_mut()
                                            {
                                                editor.protocol = protocol.to_string();
                                                editor.port = default_port(protocol).to_string();
                                                editor.active_field = 0;
                                                editor.error = None;
                                            }
                                            cx.notify();
                                        }))
                                        .child(protocol.to_uppercase())
                                }),
                        ),
                    )
                    .when(editor.protocol == "ssh", |view| {
                        view.child(
                            div().flex().gap_2().children(
                                [("password", "密码认证"), ("privateKey", "私钥认证")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, (auth, label))| {
                                        let active = editor.auth_type == auth;
                                        div()
                                            .id(("connection-auth", index))
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .border_1()
                                            .border_color(if active {
                                                palette.accent
                                            } else {
                                                palette.border
                                            })
                                            .text_xs()
                                            .text_color(if active {
                                                palette.accent
                                            } else {
                                                palette.text_muted
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(editor) =
                                                    this.pending_connection_editor.as_mut()
                                                {
                                                    editor.auth_type = auth.to_string();
                                                    editor.active_field = 0;
                                                }
                                                cx.notify();
                                            }))
                                            .child(label)
                                    }),
                            ),
                        )
                    })
                    .when(
                        editor.protocol == "ssh" && editor.auth_type == "privateKey",
                        |view| {
                            view.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div().text_xs().text_color(palette.text_muted).child(
                                            "托管密钥（点击选择；也可填写下方路径作为回退）",
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .id("connection-managed-key-none")
                                                    .px_3()
                                                    .py_1()
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .border_1()
                                                    .border_color(
                                                        if editor.private_key_id.is_empty() {
                                                            palette.accent
                                                        } else {
                                                            palette.border
                                                        },
                                                    )
                                                    .text_xs()
                                                    .text_color(palette.text_muted)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        if let Some(editor) =
                                                            this.pending_connection_editor.as_mut()
                                                        {
                                                            editor.private_key_id.clear();
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child("使用文件路径"),
                                            )
                                            .children(
                                                self.state
                                                    .read(cx)
                                                    .ssh_keys
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(index, key)| {
                                                        let key_id = key.id.clone();
                                                        let active =
                                                            editor.private_key_id == key.id;
                                                        div()
                                                            .id(("connection-managed-key", index))
                                                            .px_3()
                                                            .py_1()
                                                            .rounded_md()
                                                            .cursor_pointer()
                                                            .border_1()
                                                            .border_color(if active {
                                                                palette.accent
                                                            } else {
                                                                palette.border
                                                            })
                                                            .text_xs()
                                                            .text_color(if active {
                                                                palette.accent
                                                            } else {
                                                                palette.text_muted
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    if let Some(editor) = this
                                                                        .pending_connection_editor
                                                                        .as_mut()
                                                                    {
                                                                        editor.private_key_id =
                                                                            key_id.clone();
                                                                        editor
                                                                            .private_key_path
                                                                            .clear();
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            ))
                                                            .child(key.note.clone().unwrap_or_else(
                                                                || key.name.clone(),
                                                            ))
                                                    }),
                                            ),
                                    ),
                            )
                        },
                    )
                    .when(editor.protocol == "ftp", |view| {
                        view.child(
                            div().flex().gap_2().children(
                                [
                                    ("none", "FTP"),
                                    ("explicit", "显式 FTPS"),
                                    ("implicit", "隐式 FTPS"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (mode, label))| {
                                    let active = editor.security_mode == mode;
                                    div()
                                        .id(("ftp-security", index))
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .border_1()
                                        .border_color(if active {
                                            palette.accent
                                        } else {
                                            palette.border
                                        })
                                        .text_xs()
                                        .text_color(if active {
                                            palette.accent
                                        } else {
                                            palette.text_muted
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(editor) =
                                                this.pending_connection_editor.as_mut()
                                            {
                                                editor.security_mode = mode.to_string();
                                            }
                                            cx.notify();
                                        }))
                                        .child(label)
                                }),
                            ),
                        )
                    })
                    .child(
                        div().flex().flex_col().gap_2().children(
                            fields.into_iter().enumerate().map(
                                |(index, (label, value, secret))| {
                                    connection_input(
                                        label,
                                        value,
                                        index,
                                        editor.active_field,
                                        secret,
                                        palette,
                                        cx,
                                    )
                                },
                            ),
                        ),
                    )
                    .when_some(editor.error.clone(), |view, error| {
                        view.child(div().text_xs().text_color(palette.danger).child(error))
                    })
                    .when(editor.delete_confirmation, |view| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(palette.danger)
                                .child("再次点击“确认删除”将永久删除此连接配置和对应凭据。"),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(div().when(editing, |view| {
                                view.child(action_button(
                                    if editor.delete_confirmation {
                                        "确认删除"
                                    } else {
                                        "删除"
                                    },
                                    "delete-connection",
                                    false,
                                    palette,
                                    cx,
                                    |this, cx| this.delete_connection(cx),
                                ))
                            }))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(action_button(
                                        "取消",
                                        "cancel-connection",
                                        false,
                                        palette,
                                        cx,
                                        |this, cx| {
                                            this.pending_connection_editor = None;
                                            cx.notify();
                                        },
                                    ))
                                    .child(action_button(
                                        if editor.busy { "保存中" } else { "保存" },
                                        "save-connection",
                                        true,
                                        palette,
                                        cx,
                                        |this, cx| this.save_connection(cx),
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn text(profile: &Value, key: &str, fallback: &str) -> String {
    profile
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn default_port(protocol: &str) -> u64 {
    match protocol {
        "ftp" => 21,
        "telnet" => 23,
        _ => 22,
    }
}

fn connection_input(
    label: &'static str,
    value: String,
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
        value
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(palette.text_muted).child(label))
        .child(
            div()
                .id(("connection-input", field))
                .h(px(36.0))
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
                    if let Some(editor) = this.pending_connection_editor.as_mut() {
                        editor.active_field = field;
                    }
                    cx.notify();
                }))
                .child(display),
        )
}

fn action_button(
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
