//! Phase G-1.1 spike: open a 1024x720 dark grey window.
//!
//! Run with:
//!   cargo run -p fileterm-gpui --example term_spike
//!
//! Acceptance: a 1024x720 dark grey (#181818) window opens; closing it exits
//! the application cleanly.
//!
//! Deviations from `docs/plans/active/gpui-spike.md` G-1.1 skeleton, all
//! forced by the real `gpui-unofficial` 1.8.2 API surface:
//!   * Entry point is `gpui_platform::application()` then `.run()`, not
//!     `Application::run()` directly. Without `gpui_platform::application()`
//!     the platform windowing/text drivers never initialize.
//!   * `Bounds::centered` requires a `&App` context argument.
//!   * `cx.open_window`'s view-builder closure is `|window, cx|`, not `|cx|`.
//!   * `div().size_full()` needs `use gpui::Styled;` in scope (trait method).
//!   * Entity construction is `cx.new(|cx| ...)`, not `cx.new_view(...)`.
//!     `new` is a trait method on `AppContext` (impl'd by `App`), so
//!     `use gpui::AppContext;` is required.

use gpui::{
    App, AppContext, Bounds, Context, IntoElement, Render, Styled, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowKind, WindowOptions, div, px, rgb, size,
};

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1024.0), px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FileTerm GPUI Spike".into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: if cfg!(target_os = "windows") {
                    Some(WindowDecorations::Client)
                } else {
                    Some(WindowDecorations::Server)
                },
                kind: WindowKind::Normal,
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| SpikeView),
        )
        .unwrap();
    });
}

struct SpikeView;

impl Render for SpikeView {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x181818))
    }
}
