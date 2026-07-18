use std::sync::Arc;

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::{
    state::AppState,
    term::{StreamController, TermView},
    theme::ThemePalette,
};

pub struct StreamSessionWorkspace {
    protocol: String,
    endpoint: String,
    controller: Arc<StreamController>,
    terminal: Option<Entity<TermView>>,
    app_state: Entity<AppState>,
}

impl StreamSessionWorkspace {
    pub fn new(
        protocol: String,
        endpoint: String,
        controller: Arc<StreamController>,
        app_state: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let transport: Arc<dyn crate::term::TerminalTransport> = controller.clone();
        let terminal = cx.new(|cx| TermView::from_transport(cx, transport, 80, 24));
        Self {
            protocol,
            endpoint,
            controller,
            terminal: Some(terminal),
            app_state,
        }
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
                    .child(div().text_xs().text_color(palette.success).child("已连接")),
            )
            .child(
                div()
                    .min_h(px(0.0))
                    .flex_1()
                    .when_some(terminal, |view, terminal| view.child(terminal)),
            )
    }
}
