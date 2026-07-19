use std::sync::Arc;

use gpui::{
    div, prelude::*, px, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window,
};

use crate::{
    ssh::{SshController, SshTunnelRule, SshTunnelSnapshot},
    state::AppState,
    theme::ThemePalette,
    view::text_editor::{TextInput, TextInputEvent, TextInputMode},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TunnelField {
    Name,
    BindHost,
    BindPort,
    TargetHost,
    TargetPort,
}

#[derive(Clone)]
struct TunnelInputs {
    name: Entity<TextInput>,
    bind_host: Entity<TextInput>,
    bind_port: Entity<TextInput>,
    target_host: Entity<TextInput>,
    target_port: Entity<TextInput>,
}

#[derive(Clone)]
struct TunnelForm {
    inputs: Option<TunnelInputs>,
    name: String,
    kind: String,
    bind_host: String,
    bind_port: String,
    target_host: String,
    target_port: String,
}

impl Default for TunnelForm {
    fn default() -> Self {
        Self {
            inputs: None,
            name: String::new(),
            kind: "local".into(),
            bind_host: "127.0.0.1".into(),
            bind_port: String::new(),
            target_host: String::new(),
            target_port: String::new(),
        }
    }
}

impl TunnelForm {
    fn set_field(&mut self, field: TunnelField, value: String) {
        match field {
            TunnelField::Name => self.name = value,
            TunnelField::BindHost => self.bind_host = value,
            TunnelField::BindPort => self.bind_port = value,
            TunnelField::TargetHost => self.target_host = value,
            TunnelField::TargetPort => self.target_port = value,
        }
    }

    fn cycle_kind(&mut self) {
        self.kind = match self.kind.as_str() {
            "local" => "dynamic",
            "dynamic" => "remote",
            _ => "local",
        }
        .to_string();
        if self.kind == "dynamic" {
            self.target_host.clear();
            self.target_port.clear();
        }
    }

    fn into_rule(self) -> anyhow::Result<SshTunnelRule> {
        let bind_port = self
            .bind_port
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("监听端口必须是 1-65535 的整数"))?;
        let target_port = if self.kind == "dynamic" {
            None
        } else {
            Some(
                self.target_port
                    .parse::<u16>()
                    .map_err(|_| anyhow::anyhow!("目标端口必须是 1-65535 的整数"))?,
            )
        };
        Ok(SshTunnelRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: if self.name.trim().is_empty() {
                format!("{}:{}", self.bind_host.trim(), bind_port)
            } else {
                self.name.trim().to_string()
            },
            kind: self.kind,
            bind_host: self.bind_host.trim().to_string(),
            bind_port,
            target_host: (target_port.is_some()).then(|| self.target_host.trim().to_string()),
            target_port,
            auto_start: false,
        })
    }
}

pub struct SshTunnelPanel {
    controller: Arc<SshController>,
    app_state: gpui::Entity<AppState>,
    tunnels: Vec<SshTunnelSnapshot>,
    form: Option<TunnelForm>,
    busy: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl SshTunnelPanel {
    pub fn new(
        controller: Arc<SshController>,
        app_state: gpui::Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            controller,
            app_state,
            tunnels: Vec::new(),
            form: None,
            busy: false,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    fn begin_form(&mut self, cx: &mut Context<Self>) {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let create_input = |value: &str, placeholder: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                TextInput::new(
                    value,
                    placeholder,
                    TextInputMode::SingleLine,
                    false,
                    palette,
                    cx,
                )
            })
        };
        let form = TunnelForm::default();
        let inputs = TunnelInputs {
            name: create_input(&form.name, "隧道名称", cx),
            bind_host: create_input(&form.bind_host, "127.0.0.1", cx),
            bind_port: create_input(&form.bind_port, "监听端口", cx),
            target_host: create_input(&form.target_host, "目标地址", cx),
            target_port: create_input(&form.target_port, "目标端口", cx),
        };
        for (field, input) in [
            (TunnelField::Name, inputs.name.clone()),
            (TunnelField::BindHost, inputs.bind_host.clone()),
            (TunnelField::BindPort, inputs.bind_port.clone()),
            (TunnelField::TargetHost, inputs.target_host.clone()),
            (TunnelField::TargetPort, inputs.target_port.clone()),
        ] {
            cx.subscribe(&input, move |panel, _, event, cx| match event {
                TextInputEvent::Changed(value) => {
                    if let Some(form) = panel.form.as_mut() {
                        form.set_field(field, value.clone());
                    }
                }
                TextInputEvent::Submit | TextInputEvent::Save => panel.submit(cx),
                TextInputEvent::Cancel => {
                    panel.form = None;
                    cx.notify();
                }
            })
            .detach();
        }
        inputs.name.update(cx, |input, cx| input.request_focus(cx));
        self.form = Some(TunnelForm {
            inputs: Some(inputs),
            ..form
        });
        self.error = None;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(form) = self.form.clone() else {
            return;
        };
        let rule = match form.into_rule() {
            Ok(rule) => rule,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.error = None;
        let controller = self.controller.clone();
        cx.spawn(async move |this, cx| {
            let result = controller.create_tunnel(rule).await;
            let _ = this.update(cx, |panel, cx| {
                panel.busy = false;
                match result {
                    Ok(tunnels) => {
                        panel.tunnels = tunnels;
                        panel.form = None;
                    }
                    Err(error) => panel.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mutate_tunnel(&mut self, rule_id: String, operation: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let controller = self.controller.clone();
        cx.spawn(async move |this, cx| {
            let result = match operation {
                "start" => controller.start_tunnel(&rule_id).await,
                "stop" => controller.stop_tunnel(&rule_id).await,
                "delete" => controller.delete_tunnel(&rule_id).await,
                _ => unreachable!(),
            };
            let _ = this.update(cx, |panel, cx| {
                panel.busy = false;
                match result {
                    Ok(tunnels) => panel.tunnels = tunnels,
                    Err(error) => panel.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_field(
        &self,
        label: &'static str,
        input: Entity<TextInput>,
        palette: ThemePalette,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(68.0))
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(label),
            )
            .child(div().min_w(px(0.0)).flex_1().child(input))
    }
}

impl Focusable for SshTunnelPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SshTunnelPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let rows = self
            .tunnels
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, tunnel)| {
                let toggle_id = tunnel.rule.id.clone();
                let delete_id = tunnel.rule.id.clone();
                let running = tunnel.status == "running";
                let endpoint = if tunnel.rule.kind == "dynamic" {
                    format!(
                        "{}:{} · SOCKS5",
                        tunnel.rule.bind_host, tunnel.rule.bind_port
                    )
                } else {
                    format!(
                        "{}:{} → {}:{}",
                        tunnel.rule.bind_host,
                        tunnel.rule.bind_port,
                        tunnel.rule.target_host.as_deref().unwrap_or(""),
                        tunnel.rule.target_port.unwrap_or_default()
                    )
                };
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text)
                                    .child(tunnel.rule.name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if running {
                                        palette.success
                                    } else {
                                        palette.text_soft
                                    })
                                    .child(tunnel.status),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(endpoint),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id(("toggle-ssh-tunnel", index))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mutate_tunnel(
                                            toggle_id.clone(),
                                            if running { "stop" } else { "start" },
                                            cx,
                                        );
                                    }))
                                    .child(if running { "停止" } else { "启动" }),
                            )
                            .child(
                                div()
                                    .id(("delete-ssh-tunnel", index))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(palette.danger)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mutate_tunnel(delete_id.clone(), "delete", cx);
                                    }))
                                    .child("删除"),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        let form = self.form.clone();
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(palette.border_strong)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .child(div().text_sm().text_color(palette.text).child("SSH 隧道"))
                    .child(
                        div()
                            .id("new-ssh-tunnel")
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .on_click(cx.listener(|this, _, _, cx| this.begin_form(cx)))
                            .child("新增"),
                    ),
            )
            .children(rows)
            .when_some(form, |view, form| {
                let inputs = form.inputs.expect("SSH tunnel inputs initialized");
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(palette.border)
                        .child(self.render_field("名称", inputs.name, palette))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(68.0))
                                        .text_xs()
                                        .text_color(palette.text_muted)
                                        .child("类型"),
                                )
                                .child(
                                    div()
                                        .id("ssh-tunnel-kind")
                                        .flex_1()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(palette.background)
                                        .border_1()
                                        .border_color(palette.border)
                                        .text_xs()
                                        .text_color(palette.text)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(form) = this.form.as_mut() {
                                                form.cycle_kind();
                                            }
                                            cx.notify();
                                        }))
                                        .child(form.kind.clone()),
                                ),
                        )
                        .child(self.render_field("监听地址", inputs.bind_host, palette))
                        .child(self.render_field("监听端口", inputs.bind_port, palette))
                        .when(form.kind != "dynamic", |form_view| {
                            form_view
                                .child(self.render_field("目标地址", inputs.target_host, palette))
                                .child(self.render_field("目标端口", inputs.target_port, palette))
                        })
                        .child(
                            div().text_xs().text_color(palette.text_soft).child(
                                "点击类型切换；Tab/Shift+Tab 切换输入项，Enter 创建，Esc 取消",
                            ),
                        ),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(palette.danger)
                        .child(error),
                )
            })
    }
}
