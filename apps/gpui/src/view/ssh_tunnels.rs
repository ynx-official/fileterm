use std::sync::Arc;

use gpui::{
    div, prelude::*, px, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render, Window,
};

use crate::{
    ssh::{SshController, SshTunnelRule, SshTunnelSnapshot},
    state::AppState,
    theme::ThemePalette,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TunnelField {
    Name,
    Kind,
    BindHost,
    BindPort,
    TargetHost,
    TargetPort,
}

impl TunnelField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Kind,
            Self::Kind => Self::BindHost,
            Self::BindHost => Self::BindPort,
            Self::BindPort => Self::TargetHost,
            Self::TargetHost => Self::TargetPort,
            Self::TargetPort => Self::Name,
        }
    }
}

#[derive(Clone)]
struct TunnelForm {
    active: TunnelField,
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
            active: TunnelField::Name,
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
    fn active_value_mut(&mut self) -> Option<&mut String> {
        match self.active {
            TunnelField::Name => Some(&mut self.name),
            TunnelField::Kind => None,
            TunnelField::BindHost => Some(&mut self.bind_host),
            TunnelField::BindPort => Some(&mut self.bind_port),
            TunnelField::TargetHost => Some(&mut self.target_host),
            TunnelField::TargetPort => Some(&mut self.target_port),
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

    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => self.form = None,
            "tab" => form.active = form.active.next(),
            "enter" if form.active == TunnelField::Kind => form.cycle_kind(),
            "enter" => {
                self.submit(cx);
                return;
            }
            "backspace" => {
                if let Some(value) = form.active_value_mut() {
                    value.pop();
                }
            }
            "space" if form.active == TunnelField::Kind => form.cycle_kind(),
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let (Some(value), Some(text)) =
                    (form.active_value_mut(), event.keystroke.key_char.as_deref())
                {
                    value.push_str(text);
                }
            }
            _ => {}
        }
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
        value: String,
        field: TunnelField,
        palette: ThemePalette,
    ) -> impl IntoElement {
        let active = self.form.as_ref().is_some_and(|form| form.active == field);
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
            .child(
                div()
                    .min_w(px(0.0))
                    .flex_1()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(if active {
                        palette.accent
                    } else {
                        palette.border
                    })
                    .rounded_sm()
                    .text_xs()
                    .text_color(palette.text)
                    .child(if value.is_empty() {
                        " ".to_string()
                    } else {
                        value
                    }),
            )
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
            .on_key_down(cx.listener(Self::handle_key))
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.form = Some(TunnelForm::default());
                                this.error = None;
                                this.focus.focus(window, cx);
                                cx.notify();
                            }))
                            .child("新增"),
                    ),
            )
            .children(rows)
            .when_some(form, |view, form| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(palette.border)
                        .child(self.render_field("名称", form.name, TunnelField::Name, palette))
                        .child(self.render_field(
                            "类型",
                            form.kind.clone(),
                            TunnelField::Kind,
                            palette,
                        ))
                        .child(self.render_field(
                            "监听地址",
                            form.bind_host,
                            TunnelField::BindHost,
                            palette,
                        ))
                        .child(self.render_field(
                            "监听端口",
                            form.bind_port,
                            TunnelField::BindPort,
                            palette,
                        ))
                        .when(form.kind != "dynamic", |form_view| {
                            form_view
                                .child(self.render_field(
                                    "目标地址",
                                    form.target_host,
                                    TunnelField::TargetHost,
                                    palette,
                                ))
                                .child(self.render_field(
                                    "目标端口",
                                    form.target_port,
                                    TunnelField::TargetPort,
                                    palette,
                                ))
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.text_soft)
                                .child("Tab 切换字段，类型字段按空格切换，Enter 创建，Esc 取消"),
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
