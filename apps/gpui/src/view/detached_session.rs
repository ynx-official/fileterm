use gpui::{
    div, prelude::*, px, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window,
};

use crate::{
    state::AppState,
    theme::ThemePalette,
    view::{LocalSessionWorkspace, SessionWorkspace, StreamSessionWorkspace},
};

#[derive(Clone)]
pub enum DetachedSessionContent {
    Ssh(Entity<SessionWorkspace>),
    Local(Entity<LocalSessionWorkspace>),
    Stream(Entity<StreamSessionWorkspace>),
}

pub struct DetachedSessionWindow {
    tab_id: String,
    title: String,
    state: Entity<AppState>,
    content: DetachedSessionContent,
    focus: FocusHandle,
}

impl DetachedSessionWindow {
    pub fn new(
        tab_id: String,
        title: String,
        state: Entity<AppState>,
        content: DetachedSessionContent,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            tab_id,
            title,
            state,
            content,
            focus: cx.focus_handle(),
        }
    }
}

impl Focusable for DetachedSessionWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DetachedSessionWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.state.read(cx).theme);
        let tab_id = self.tab_id.clone();
        let content = match &self.content {
            DetachedSessionContent::Ssh(session) => session.clone().into_any_element(),
            DetachedSessionContent::Local(session) => session.clone().into_any_element(),
            DetachedSessionContent::Stream(session) => session.clone().into_any_element(),
        };

        div()
            .id(tab_id)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .text_color(palette.text)
            .child(
                div()
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .id("return-detached-session")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .hover(move |style| style.bg(palette.accent_surface))
                            .on_click(|_, window, _| window.remove_window())
                            .child("返回主窗口"),
                    ),
            )
            .child(div().min_h(px(0.0)).flex_1().child(content))
    }
}
