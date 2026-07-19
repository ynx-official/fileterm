use std::sync::Arc;

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::{
    backend::{ConnectedStreamSession, FileTermDesktopApi},
    state::{AppState, TabStatus},
    term::{stream::StreamSessionEvent, StreamController, TermView},
    theme::ThemePalette,
};

pub struct StreamSessionWorkspace {
    tab_id: String,
    profile_id: String,
    protocol: String,
    endpoint: String,
    api: Arc<dyn FileTermDesktopApi>,
    controller: Arc<StreamController>,
    terminal: Option<Entity<TermView>>,
    app_state: Entity<AppState>,
    status: TabStatus,
    error: Option<String>,
    generation: u64,
}

impl StreamSessionWorkspace {
    pub fn new(
        tab_id: String,
        profile_id: String,
        api: Arc<dyn FileTermDesktopApi>,
        session: ConnectedStreamSession,
        app_state: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let ConnectedStreamSession {
            controller,
            protocol,
            endpoint,
        } = session;
        let transport: Arc<dyn crate::term::TerminalTransport> = controller.clone();
        let terminal = cx.new(|cx| TermView::from_transport(cx, transport, 80, 24));
        let mut workspace = Self {
            tab_id,
            profile_id,
            protocol,
            endpoint,
            api,
            controller,
            terminal: Some(terminal),
            app_state,
            status: TabStatus::Connected,
            error: None,
            generation: 0,
        };
        workspace.spawn_events(cx);
        workspace
    }

    fn spawn_events(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let mut events = self.controller.subscribe_events();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let terminal = matches!(
                    event,
                    StreamSessionEvent::Closed | StreamSessionEvent::Error(_)
                );
                let _ = this.update(cx, |workspace, cx| {
                    if workspace.generation != generation {
                        return;
                    }
                    workspace.status = match event {
                        StreamSessionEvent::Connected => TabStatus::Connected,
                        StreamSessionEvent::Closed => TabStatus::Closed,
                        StreamSessionEvent::Error(error) => {
                            workspace.error = Some(error);
                            TabStatus::Error
                        }
                    };
                    workspace.app_state.update(cx, |state, cx| {
                        state.set_tab_status(&workspace.tab_id, workspace.status);
                        cx.notify();
                    });
                    cx.notify();
                });
                if terminal {
                    break;
                }
            }
        })
        .detach();
    }

    fn reconnect(&mut self, cx: &mut Context<Self>) {
        if self.status == TabStatus::Connecting {
            return;
        }

        self.controller.shutdown();
        self.generation = self.generation.wrapping_add(1);
        self.status = TabStatus::Connecting;
        self.error = None;
        self.terminal = None;
        self.app_state.update(cx, |state, cx| {
            state.set_tab_status(&self.tab_id, TabStatus::Connecting);
            cx.notify();
        });

        let api = self.api.clone();
        let profile_id = self.profile_id.clone();
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = api.stream_connect(&profile_id).await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.generation != generation {
                    if let Ok(session) = result {
                        session.controller.shutdown();
                    }
                    return;
                }

                match result {
                    Ok(session) => {
                        workspace.protocol = session.protocol;
                        workspace.endpoint = session.endpoint;
                        workspace.controller = session.controller;
                        let transport: Arc<dyn crate::term::TerminalTransport> =
                            workspace.controller.clone();
                        workspace.terminal =
                            Some(cx.new(|cx| TermView::from_transport(cx, transport, 80, 24)));
                        workspace.status = TabStatus::Connected;
                        workspace.error = None;
                        workspace.spawn_events(cx);
                    }
                    Err(error) => {
                        workspace.status = TabStatus::Error;
                        workspace.error = Some(error.to_string());
                    }
                }
                workspace.app_state.update(cx, |state, cx| {
                    state.set_tab_status(&workspace.tab_id, workspace.status);
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn send_command(&self, command: &str, append_carriage_return: bool) -> anyhow::Result<()> {
        crate::term::TerminalTransport::write_input(self.controller.as_ref(), command.as_bytes())?;
        if append_carriage_return {
            crate::term::TerminalTransport::write_input(self.controller.as_ref(), b"\r")?;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.controller.shutdown();
        self.terminal = None;
    }
}

impl Drop for StreamSessionWorkspace {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

impl Render for StreamSessionWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let terminal = self.terminal.clone();
        let (status_label, status_color) = match self.status {
            TabStatus::Connected => ("已连接", palette.success),
            TabStatus::Connecting => ("连接中", palette.warning),
            TabStatus::Error => ("连接错误", palette.danger),
            TabStatus::Closed | TabStatus::Idle => ("已断开", palette.text_soft),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
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
                            .child(format!(
                                "{} · {}",
                                self.protocol.to_uppercase(),
                                self.endpoint
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().text_color(status_color).child(status_label))
                            .when(
                                matches!(self.status, TabStatus::Error | TabStatus::Closed),
                                |actions| {
                                    actions.child(
                                        div()
                                            .id("stream-reconnect")
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .border_1()
                                            .border_color(palette.border)
                                            .text_xs()
                                            .text_color(palette.accent)
                                            .hover(move |style| style.bg(palette.accent_surface))
                                            .on_click(cx.listener(|workspace, _, _, cx| {
                                                workspace.reconnect(cx)
                                            }))
                                            .child("重新连接"),
                                    )
                                },
                            ),
                    ),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_3()
                        .py_2()
                        .bg(palette.surface)
                        .text_xs()
                        .text_color(palette.danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .min_h(px(0.0))
                    .flex_1()
                    .when_some(terminal, |view, terminal| view.child(terminal)),
            )
    }
}
