//! FileTerm GPUI runtime — binary entry point.
//!
//! G0 phase of `docs/plans/active/gpui-refactor.md` section 6.1.
//!
//! Opens a single empty main window. The window title advertises the
//! current phase so it's visually obvious during migration that you're
//! running the GPUI runtime, not Tauri. No tabs, no sidebar, no terminal
//! — those land in G2 (desktop shell) and G3 (SSH terminal).
//!
//! ## Tokio runtime
//!
//! Same setup as `examples/term_spike.rs`: a multi-threaded tokio runtime
//! is started and `enter()`ed before `application().run`, so any
//! `cx.spawn` future that calls `tokio::spawn` (e.g. the G-5
//! `spawn_term_feed` pump) finds a reactor. Without this the pump panics
//! with "there is no reactor running" — see `src/term/spawn.rs` for the
//! full architecture rationale.

use std::sync::Arc;

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, SharedString, Styled, TitlebarOptions, Window, WindowBounds, WindowDecorations,
    WindowKind, WindowOptions, div, px, rgb, size,
};

use fileterm_gpui::backend::{FileTermDesktopApi, GpuiDesktopApi};

fn main() {
    // See module doc — must precede `application().run`.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _rt_guard = rt.enter();

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FileTerm — GPUI runtime (G0 scaffold)".into()),
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
            |_window, cx| cx.new(RootView::new),
        )
        .unwrap();
    });
}

/// Root view for the G0 main window.
///
/// G0 just renders a centered placeholder string. The bridge handle is
/// stored here so G1+ view layers have a place to grab it from; G0 itself
/// doesn't call any bridge methods (they all return `Unsupported` anyway).
///
/// As real features land, this struct grows into the workspace shell:
/// sidebar (G2) → tabs (G2) → terminal dock (G3) → file manager (G4) →
/// detach support (G5). For now it's a single div with a label.
pub struct RootView {
    /// Held but unused in G0. G1+ call sites will do
    /// `self.api.app_get_platform().await` etc. Storing it as
    /// `Arc<dyn FileTermDesktopApi>` (rather than the concrete
    /// `GpuiDesktopApi`) means swapping in a mock for tests is a one-line
    /// change at the construction site.
    #[allow(dead_code)]
    api: Arc<dyn FileTermDesktopApi>,
    focus: FocusHandle,
    /// Cached label so we don't format the phase string every frame.
    /// Updates only when the phase changes (which in G0 is never — it's
    /// a compile-time constant).
    label: SharedString,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            api: Arc::new(GpuiDesktopApi),
            focus: cx.focus_handle(),
            label: "FileTerm GPUI runtime — G0 scaffold\n\n\
                    Bridge trait + stub impl ready.\n\
                    Storage / window / SSH / SFTP / transfer land in G1–G5."
                .into(),
        }
    }
}

impl Focusable for RootView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // G0 has no interactive elements — just a centered label on a
        // neutral background. The bg color is a placeholder; the real
        // theme system (token → theme vars → component skins) lands in
        // G2 along with the sidebar.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .child(
                div()
                    .child(self.label.clone())
                    .line_height(px(21.0))
                    .text_size(px(14.0)),
            )
    }
}

// Touch `Entity` so the import stays meaningful even though G0's `RootView`
// is constructed via `cx.new` (which is an `AppContext` trait method, not
// a `Context`-typed one). Future phases will likely hold child entities
// here.
#[allow(dead_code)]
fn _entity_marker<T: 'static>(_: &Entity<T>) {}
