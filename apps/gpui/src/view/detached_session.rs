use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Focusable,
    IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, Render, TitlebarOptions, Window,
    WindowBounds, WindowDecorations, WindowHandle, WindowKind, WindowOptions,
};

use crate::{
    state::AppState,
    theme::ThemePalette,
    view::{FtpWorkspace, LocalSessionWorkspace, SessionWorkspace, StreamSessionWorkspace},
    window::{
        detach_tab_to_new_window, DragDropTarget, ScreenBounds, SharedWindowRegistry, TabDragState,
    },
};

#[derive(Clone)]
pub enum DetachedSessionContent {
    Ssh(Entity<SessionWorkspace>),
    Ftp(Entity<FtpWorkspace>),
    Local(Entity<LocalSessionWorkspace>),
    Stream(Entity<StreamSessionWorkspace>),
}

#[derive(Clone)]
pub struct DetachedSessionTab {
    pub tab_id: String,
    pub title: String,
    pub content: DetachedSessionContent,
}

pub struct DetachedSessionWindow {
    window_id: String,
    state: Entity<AppState>,
    registry: SharedWindowRegistry,
    tabs: Vec<DetachedSessionTab>,
    active_tab_id: String,
    drag: TabDragState,
    focus: FocusHandle,
}

impl DetachedSessionWindow {
    pub fn new(
        window_id: String,
        tab_id: String,
        title: String,
        state: Entity<AppState>,
        registry: SharedWindowRegistry,
        content: DetachedSessionContent,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            window_id,
            state,
            registry,
            tabs: vec![DetachedSessionTab {
                tab_id: tab_id.clone(),
                title,
                content,
            }],
            active_tab_id: tab_id,
            drag: TabDragState::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn add_tab(&mut self, tab: DetachedSessionTab, cx: &mut Context<Self>) {
        self.tabs.retain(|existing| existing.tab_id != tab.tab_id);
        self.active_tab_id = tab.tab_id.clone();
        self.tabs.push(tab);
        cx.notify();
    }

    fn remove_tab(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.retain(|tab| tab.tab_id != tab_id);
        if self.active_tab_id == tab_id {
            self.active_tab_id = self
                .tabs
                .last()
                .map(|tab| tab.tab_id.clone())
                .unwrap_or_default();
        }
        if self.tabs.is_empty() {
            window.remove_window();
        } else {
            cx.notify();
        }
    }

    fn tab(&self, tab_id: &str) -> Option<DetachedSessionTab> {
        self.tabs.iter().find(|tab| tab.tab_id == tab_id).cloned()
    }

    fn start_drag(&mut self, tab_id: &str, _: &MouseDownEvent, cx: &mut Context<Self>) {
        self.drag.cancel();
        self.drag.start(tab_id, &self.window_id);
        self.active_tab_id = tab_id.to_string();
        cx.notify();
    }

    fn finish_drag(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.drag.active_tab_id().map(str::to_string) else {
            return;
        };
        let screen = screen_position(window, event.position);
        let Some(target) = self
            .drag
            .finish(screen.0, screen.1, &self.registry.bounds_snapshot())
        else {
            return;
        };
        match target {
            DragDropTarget::SameWindow => {}
            DragDropTarget::OtherWindow(target_window) if target_window == "main" => {
                if self.registry.return_tab_to_main(&tab_id) {
                    self.state.update(cx, |state, cx| {
                        state.activate_tab(&tab_id);
                        cx.notify();
                    });
                    self.remove_tab(&tab_id, window, cx);
                }
            }
            DragDropTarget::OtherWindow(target_window) => {
                let Some(tab) = self.tab(&tab_id) else { return };
                let Some(handle_id) = self.registry.handle_for(&target_window) else {
                    return;
                };
                let handle = WindowHandle::<DetachedSessionWindow>::new(handle_id);
                if handle
                    .update(cx, |target, _, cx| target.add_tab(tab, cx))
                    .is_ok()
                {
                    self.registry.detach_tab(&tab_id, &target_window);
                    self.remove_tab(&tab_id, window, cx);
                }
            }
            DragDropTarget::NewWindow => self.move_tab_to_new_window(&tab_id, screen, window, cx),
        }
    }

    fn move_tab_to_new_window(
        &mut self,
        tab_id: &str,
        screen: (i32, i32),
        source_window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab(tab_id) else { return };
        let result = detach_tab_to_new_window(&self.registry, tab_id);
        let new_window_id = result.new_window_id.clone();
        let registry = self.registry.clone();
        let state = self.state.clone();
        let new_tab = tab.clone();
        let bounds = Bounds::new(
            point(px(screen.0 as f32 - 80.0), px(screen.1 as f32 - 20.0)),
            size(px(1040.0), px(720.0)),
        );
        let open_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(tab.title.clone().into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: Some(WindowDecorations::Server),
                kind: WindowKind::Normal,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| {
                    DetachedSessionWindow::new(
                        new_window_id,
                        new_tab.tab_id,
                        new_tab.title,
                        state,
                        registry.clone(),
                        new_tab.content,
                        cx,
                    )
                });
                view.focus_handle(cx).focus(window, cx);
                view
            },
        );
        match open_result {
            Ok(handle) => {
                self.registry
                    .register_handle(&result.new_window_id, handle.window_id());
                self.remove_tab(tab_id, source_window, cx);
            }
            Err(_) => self.registry.detach_tab(tab_id, &self.window_id),
        }
    }

    fn return_active_to_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_id = self.active_tab_id.clone();
        if self.registry.return_tab_to_main(&tab_id) {
            self.state.update(cx, |state, cx| {
                state.activate_tab(&tab_id);
                cx.notify();
            });
            self.remove_tab(&tab_id, window, cx);
        }
    }
}

impl Focusable for DetachedSessionWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DetachedSessionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.state.read(cx).theme);
        let global_bounds = window.window_bounds().get_bounds();
        self.registry.update_bounds(
            &self.window_id,
            ScreenBounds::new(
                f32::from(global_bounds.origin.x) as i32,
                f32::from(global_bounds.origin.y) as i32,
                f32::from(global_bounds.size.width) as i32,
                f32::from(global_bounds.size.height) as i32,
            ),
        );
        let content = self
            .tabs
            .iter()
            .find(|tab| tab.tab_id == self.active_tab_id)
            .or_else(|| self.tabs.first())
            .map(|tab| match &tab.content {
                DetachedSessionContent::Ssh(session) => session.clone().into_any_element(),
                DetachedSessionContent::Ftp(session) => session.clone().into_any_element(),
                DetachedSessionContent::Local(session) => session.clone().into_any_element(),
                DetachedSessionContent::Stream(session) => session.clone().into_any_element(),
            });

        div()
            .id(self.window_id.clone())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .text_color(palette.text)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.finish_drag(event, window, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.finish_drag(event, window, cx)),
            )
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_end()
                    .justify_between()
                    .px_2()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().h_full().flex().items_end().gap_1().children(
                        self.tabs.iter().enumerate().map(|(index, tab)| {
                            let tab_id = tab.tab_id.clone();
                            let click_id = tab_id.clone();
                            let drag_id = tab_id.clone();
                            let active = self.active_tab_id == tab_id;
                            div()
                                .id(("detached-tab", index))
                                .h(px(34.0))
                                .min_w(px(120.0))
                                .max_w(px(220.0))
                                .px_3()
                                .flex()
                                .items_center()
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
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event, _, cx| {
                                        this.start_drag(&drag_id, event, cx)
                                    }),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.active_tab_id = click_id.clone();
                                    cx.notify();
                                }))
                                .child(div().truncate().text_sm().child(tab.title.clone()))
                        }),
                    ))
                    .child(
                        div()
                            .id("return-detached-session")
                            .mb_1()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .hover(move |style| style.bg(palette.accent_surface))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.return_active_to_main(window, cx)
                            }))
                            .child("返回主窗口"),
                    ),
            )
            .child(div().min_h(px(0.0)).flex_1().children(content))
    }
}

fn screen_position(window: &Window, local: gpui::Point<gpui::Pixels>) -> (i32, i32) {
    let bounds = window.window_bounds().get_bounds();
    (
        (f32::from(bounds.origin.x) + f32::from(local.x)) as i32,
        (f32::from(bounds.origin.y) + f32::from(local.y)) as i32,
    )
}
