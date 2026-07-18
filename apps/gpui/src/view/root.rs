use std::{collections::HashMap, sync::Arc};

use gpui::{
    div, prelude::*, px, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    Subscription, Window,
};

use crate::{
    backend::{FileTermDesktopApi, SshConnectOptions},
    state::{AppState, DataLoadState, NavigationSection, TabStatus},
    theme::{ThemeMode, ThemePalette},
    view::SessionWorkspace,
    window::menu::ToggleTheme,
};

pub struct RootView {
    api: Arc<dyn FileTermDesktopApi>,
    state: Entity<AppState>,
    focus: FocusHandle,
    sessions: HashMap<String, Entity<SessionWorkspace>>,
    pending_host_verification: Option<PendingHostVerification>,
    _state_subscription: Subscription,
}

#[derive(Clone)]
struct PendingHostVerification {
    profile_id: String,
    title: String,
    host: String,
    port: u16,
    fingerprint: String,
    changed: bool,
}

impl RootView {
    pub fn new(api: Arc<dyn FileTermDesktopApi>, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| AppState::default());
        let state_subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let state_for_load = state.downgrade();
        let api_for_load = api.clone();
        cx.spawn(async move |_, cx| {
            let result = api_for_load.app_get_connection_library().await;
            let _ = state_for_load.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_connection_library(library),
                    Err(error) => state.fail_data_load(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();

        Self {
            api,
            state,
            focus: cx.focus_handle(),
            sessions: HashMap::new(),
            pending_host_verification: None,
            _state_subscription: state_subscription,
        }
    }

    fn update_state(&self, cx: &mut Context<Self>, update: impl FnOnce(&mut AppState)) {
        self.state.update(cx, |state, cx| {
            update(state);
            cx.notify();
        });
    }

    fn select_navigation(&mut self, section: NavigationSection, cx: &mut Context<Self>) {
        self.update_state(cx, |state| state.select_navigation(section));
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| {
            state.sidebar_collapsed = !state.sidebar_collapsed
        });
    }

    fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| state.workspace_focus = !state.workspace_focus);
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.update_state(cx, |state| state.theme = state.theme.toggled());
    }

    fn reload_connection_library(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| {
            state.data_load_state = DataLoadState::Loading;
            state.data_error = None;
        });
        let state = self.state.downgrade();
        let api = self.api.clone();
        cx.spawn(async move |_, cx| {
            let result = api.app_get_connection_library().await;
            let _ = state.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_connection_library(library),
                    Err(error) => state.fail_data_load(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_ssh_profile(&mut self, profile_id: String, title: String, cx: &mut Context<Self>) {
        self.connect_ssh_profile(profile_id, title, SshConnectOptions::default(), cx);
    }

    fn connect_ssh_profile(
        &mut self,
        profile_id: String,
        title: String,
        options: SshConnectOptions,
        cx: &mut Context<Self>,
    ) {
        let tab_id = format!("ssh:{profile_id}");
        if self.sessions.contains_key(&tab_id) {
            self.update_state(cx, |state| state.activate_tab(&tab_id));
            return;
        }

        self.update_state(cx, |state| {
            state.open_session_tab(tab_id.clone(), title.clone())
        });
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ssh_connect(&profile_id, 80, 24, options).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(session) => {
                    let workspace = cx.new(|cx| {
                        SessionWorkspace::new(
                            tab_id.clone(),
                            session.controller,
                            session.output,
                            root.state.clone(),
                            cx,
                        )
                    });
                    root.sessions.insert(tab_id.clone(), workspace);
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Connected)
                    });
                }
                Err(crate::error::AppError::SshHostVerification {
                    host,
                    port,
                    fingerprint,
                    changed,
                }) => {
                    root.pending_host_verification = Some(PendingHostVerification {
                        profile_id: profile_id.clone(),
                        title: title.clone(),
                        host,
                        port,
                        fingerprint,
                        changed,
                    });
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Error);
                        state.data_error = None;
                    });
                    cx.notify();
                }
                Err(error) => {
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Error);
                        state.data_error = Some(error.to_string());
                    });
                }
            });
        })
        .detach();
    }

    fn close_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        self.sessions.remove(tab_id);
        self.update_state(cx, |state| state.close_tab(tab_id));
    }

    fn accept_host_key(&mut self, save: bool, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_host_verification.take() else {
            return;
        };
        self.connect_ssh_profile(
            pending.profile_id,
            pending.title,
            SshConnectOptions {
                accepted_host_fingerprint: Some(pending.fingerprint),
                save_host_fingerprint: save,
            },
            cx,
        );
    }

    fn reject_host_key(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_host_verification.take() {
            self.close_tab(&format!("ssh:{}", pending.profile_id), cx);
        }
        cx.notify();
    }

    fn render_host_verification(
        &self,
        pending: &PendingHostVerification,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.65))
            .child(
                div()
                    .w(px(520.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(if pending.changed {
                        palette.danger
                    } else {
                        palette.border_strong
                    })
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette.text)
                            .child(if pending.changed {
                                "远端主机密钥已变化"
                            } else {
                                "验证远端主机密钥"
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(format!("{}:{}", pending.host, pending.port)),
                    )
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(palette.background)
                            .text_xs()
                            .text_color(palette.text)
                            .child(pending.fingerprint.clone()),
                    )
                    .when(pending.changed, |view| {
                        view.child(div().text_sm().text_color(palette.danger).child(
                            "保存的指纹与当前服务器不一致。继续前请确认服务器密钥确实已更换。",
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("reject-host-key")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .hover(move |style| style.bg(palette.surface_hover))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.reject_host_key(cx)),
                                    )
                                    .child("拒绝"),
                            )
                            .child(
                                div()
                                    .id("accept-host-key-once")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.accent)
                                    .hover(move |style| style.bg(palette.accent_surface))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.accept_host_key(false, cx)
                                        }),
                                    )
                                    .child("仅本次接受"),
                            )
                            .child(
                                div()
                                    .id("accept-host-key-save")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.accept_host_key(true, cx)
                                        }),
                                    )
                                    .child("接受并保存"),
                            ),
                    ),
            )
    }

    fn render_sidebar(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collapsed = state.sidebar_collapsed || state.workspace_focus;
        div()
            .w(px(if collapsed { 64.0 } else { 220.0 }))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .h(px(58.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(palette.accent_surface)
                            .text_color(palette.accent)
                            .text_sm()
                            .child("FT"),
                    )
                    .when(!collapsed, |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_sm().text_color(palette.text).child("FileTerm"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.text_soft)
                                        .child("Remote Workstation"),
                                ),
                        )
                    }),
            )
            .child(
                div().flex_1().flex().flex_col().gap_1().p_2().children(
                    NavigationSection::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, section)| {
                            let active = state.navigation == section;
                            div()
                                .id(("nav", index))
                                .h(px(40.0))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(if active {
                                    palette.surface_active
                                } else {
                                    palette.sidebar
                                })
                                .hover(move |style| style.bg(palette.surface_hover))
                                .text_color(if active {
                                    palette.text
                                } else {
                                    palette.text_muted
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_navigation(section, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(28.0))
                                        .text_center()
                                        .text_xs()
                                        .text_color(if active {
                                            palette.accent
                                        } else {
                                            palette.text_soft
                                        })
                                        .child(section.glyph()),
                                )
                                .when(!collapsed, |view| view.child(section.label()))
                        }),
                ),
            )
            .child(
                div()
                    .h(px(48.0))
                    .flex()
                    .items_center()
                    .px_3()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .id("toggle-sidebar")
                            .w_full()
                            .h(px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(palette.text_muted)
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
                            .child(if collapsed { ">" } else { "收起侧栏" }),
                    ),
            )
    }

    fn render_tabbar(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .h(px(48.0))
            .w_full()
            .flex()
            .items_end()
            .justify_between()
            .bg(palette.surface)
            .border_b_1()
            .border_color(palette.border)
            .child(div().h_full().flex().items_end().gap_1().px_2().children(
                state.tabs.iter().enumerate().map(|(index, tab)| {
                    let tab_id = tab.id.clone();
                    let close_id = tab.id.clone();
                    let active = state.active_tab_id == tab.id;
                    let status_color = match tab.status {
                        TabStatus::Connected => palette.success,
                        TabStatus::Connecting => palette.warning,
                        TabStatus::Error => palette.danger,
                        TabStatus::Idle | TabStatus::Closed => palette.text_soft,
                    };
                    div()
                        .id(("tab", index))
                        .h(px(38.0))
                        .min_w(px(120.0))
                        .max_w(px(220.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .rounded_t_md()
                        .cursor_pointer()
                        .bg(if active {
                            palette.background
                        } else {
                            palette.surface
                        })
                        .border_1()
                        .border_b_0()
                        .border_color(if active {
                            palette.border_strong
                        } else {
                            palette.border
                        })
                        .text_color(if active {
                            palette.text
                        } else {
                            palette.text_muted
                        })
                        .hover(move |style| style.bg(palette.surface_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_state(cx, |state| state.activate_tab(&tab_id));
                        }))
                        .child(div().size(px(7.0)).rounded_full().bg(status_color))
                        .child(div().flex_1().truncate().text_sm().child(tab.title.clone()))
                        .when(tab.id != "overview", |view| {
                            view.child(
                                div()
                                    .id(("close-tab", index))
                                    .size(px(22.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .hover(move |style| style.bg(palette.surface_active))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_tab(&close_id, cx);
                                    }))
                                    .child("×"),
                            )
                        })
                }),
            ))
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .child(
                        div()
                            .id("focus-mode")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(if state.workspace_focus {
                                palette.accent
                            } else {
                                palette.text_muted
                            })
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_focus_mode(cx)))
                            .child(if state.workspace_focus {
                                "退出专注"
                            } else {
                                "专注模式"
                            }),
                    )
                    .child(
                        div()
                            .id("theme-toggle")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_theme(&ToggleTheme, window, cx)
                            }))
                            .child(match state.theme {
                                ThemeMode::Dark => "浅色",
                                ThemeMode::Light => "深色",
                            }),
                    ),
            )
    }

    fn render_connection_library_content(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match state.data_load_state {
            DataLoadState::Loading => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(palette.text_muted)
                .child("正在读取连接库…")
                .into_any_element(),
            DataLoadState::Error => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div().text_sm().text_color(palette.danger).child(
                        state
                            .data_error
                            .clone()
                            .unwrap_or_else(|| "连接库读取失败".to_string()),
                    ),
                )
                .child(
                    div()
                        .id("retry-connection-library")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(palette.accent_surface)
                        .text_sm()
                        .text_color(palette.accent)
                        .on_click(cx.listener(|this, _, _, cx| this.reload_connection_library(cx)))
                        .child("重新读取"),
                )
                .into_any_element(),
            DataLoadState::Ready if state.connections.is_empty() => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(div().text_lg().text_color(palette.text).child("连接库为空"))
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text_muted)
                        .child("创建第一个 SSH、FTP、Telnet 或 Serial 连接。"),
                )
                .into_any_element(),
            DataLoadState::Ready => div()
                .size_full()
                .flex()
                .flex_col()
                .children(state.connections.iter().take(8).enumerate().map(
                    |(index, connection)| {
                        let profile_id = connection.id.clone();
                        let title = connection.name.clone();
                        let is_ssh = connection.protocol == "ssh";
                        div()
                            .id(("connection-summary", index))
                            .when(is_ssh, |view| {
                                view.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_ssh_profile(
                                            profile_id.clone(),
                                            title.clone(),
                                            cx,
                                        );
                                    },
                                ))
                            })
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .w(px(54.0))
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
                                            .child(connection.endpoint.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child(connection.group.clone()),
                            )
                    },
                ))
                .into_any_element(),
        }
    }

    fn render_overview(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let connection_count = state.connections.len().to_string();
        let active_session_count = state
            .tabs
            .iter()
            .filter(|tab| matches!(tab.status, TabStatus::Connecting | TabStatus::Connected))
            .count()
            .to_string();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_end()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .text_color(palette.text)
                                    .child("远程工作台"),
                            )
                            .child(
                                div().text_sm().text_color(palette.text_muted).child(
                                    "连接、终端、文件与传输将在同一个原生 GPU 工作区中协作。",
                                ),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(palette.accent_surface)
                            .text_color(palette.accent)
                            .text_xs()
                            .child("GPUI Runtime"),
                    ),
            )
            .child(
                div().flex().gap_4().children(
                    [
                        ("连接", connection_count, "共享连接库"),
                        ("活动会话", active_session_count, "当前工作区"),
                        ("传输任务", "0".to_string(), "队列当前为空"),
                    ]
                    .into_iter()
                    .map(|(title, value, description)| {
                        div()
                            .flex_1()
                            .min_h(px(132.0))
                            .flex()
                            .flex_col()
                            .justify_between()
                            .p_4()
                            .rounded_lg()
                            .bg(palette.surface)
                            .border_1()
                            .border_color(palette.border)
                            .child(div().text_sm().text_color(palette.text_muted).child(title))
                            .child(div().text_3xl().text_color(palette.text).child(value))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_soft)
                                    .child(description),
                            )
                    }),
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
                    .child(self.render_connection_library_content(state, palette, cx)),
            )
    }

    fn render_section(
        &self,
        state: &AppState,
        section: NavigationSection,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if section == NavigationSection::Overview {
            return self.render_overview(state, palette, cx).into_any_element();
        }

        if section == NavigationSection::Connections {
            return div()
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
                                .child(div().text_sm().text_color(palette.text_muted).child(
                                    format!(
                                        "{} 个连接，{} 个文件夹",
                                        state.connections.len(),
                                        state.connection_folders.len()
                                    ),
                                )),
                        )
                        .child(
                            div()
                                .id("refresh-connection-library")
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .text_sm()
                                .text_color(palette.accent)
                                .hover(move |style| style.bg(palette.surface_hover))
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.reload_connection_library(cx)
                                    }),
                                )
                                .child("刷新"),
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
                        .child(self.render_connection_library_content(state, palette, cx)),
                )
                .into_any_element();
        }

        let (title, description) = match section {
            NavigationSection::Commands => (
                "命令管理器",
                "命令模板与多会话发送将在工作区状态接通后启用。",
            ),
            NavigationSection::Settings => {
                ("设置", "主题切换已可用；语言、终端与同步设置继续迁移。")
            }
            NavigationSection::Connections | NavigationSection::Overview => unreachable!(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .child(div().text_2xl().text_color(palette.text).child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(palette.text_muted)
                    .child(description),
            )
            .child(
                div()
                    .flex_1()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border),
            )
            .into_any_element()
    }
}

impl Focusable for RootView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let active_session = self.sessions.get(&state.active_tab_id).cloned();
        let pending_host_verification = self.pending_host_verification.clone();
        let palette = ThemePalette::for_mode(state.theme);

        div()
            .id("fileterm-root")
            .key_context("FileTerm")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::toggle_theme))
            .size_full()
            .relative()
            .flex()
            .bg(palette.background)
            .text_color(palette.text)
            .child(self.render_sidebar(&state, palette, cx))
            .child(
                div()
                    .min_w(px(0.0))
                    .h_full()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(self.render_tabbar(&state, palette, cx))
                    .child(
                        div().min_h(px(0.0)).flex_1().overflow_hidden().child(
                            match active_session {
                                Some(session) => session.into_any_element(),
                                None => self
                                    .render_section(&state, state.navigation, palette, cx)
                                    .into_any_element(),
                            },
                        ),
                    ),
            )
            .when_some(pending_host_verification, |view, pending| {
                view.child(self.render_host_verification(&pending, palette, cx))
            })
    }
}
