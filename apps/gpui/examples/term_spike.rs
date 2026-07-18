//! Phase G-1.4 spike: spawn a real bash PTY, render it via `TermView`.
//!
//! Run with:
//!   cargo run -p fileterm-gpui --example term_spike
//!
//! Acceptance (per `docs/plans/active/gpui-spike.md` G-1.4):
//!   * Window opens at 1024x720 with a bash prompt visible.
//!   * Typing `ls` / `echo hello` produces visible output.
//!   * `ls --color=always` paints red/green/blue text.
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
//!
//! G-1.4 wiring: the example spawns `bash` via `PtyHandle::spawn` with an
//! initial 80x24 grid, then wraps the resulting `PtyHandle` in an
//! `Arc<PtyHandle>` and constructs a `TermView` inside the window's root
//! entity. The view subscribes to PTY output, renders the grid via a
//! `Canvas`, and forwards keystrokes back to the PTY.
//!
//! G-1.5 fix: a multi-threaded tokio runtime must be started and
//! `enter()`ed on the main thread before `application().run`, because
//! `TermView::new` → `spawn_term_feed` uses `tokio::spawn` to drive the
//! broadcast receiver + interval pump on a tokio worker thread. Without
//! the runtime, the pump panics with "there is no reactor running". See
//! `src/term/spawn.rs` for the full architecture rationale.

use std::sync::Arc;

use gpui::{
    App, AppContext, Bounds, Context, TitlebarOptions, WindowBounds, WindowDecorations, WindowKind,
    WindowOptions, size, px,
};

use fileterm_gpui::term::{PtyHandle, TermView};

fn main() {
    // Start a multi-threaded tokio runtime so `spawn_term_feed`'s
    // `tokio::spawn` (broadcast recv + interval tick) has a reactor. See
    // the G-1.5 note in the module doc above and `src/term/spawn.rs`.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _rt_guard = rt.enter();

    // Use bash for the spike; fall back to sh if bash isn't installed.
    let shell = which_shell();

    // Spawn the PTY before opening the window so we can hand it to the view
    // in the window-builder closure. 80x24 matches the model's default and
    // is what bash configures against on startup.
    let (pty, _rx) = PtyHandle::spawn(shell, 80, 24).expect("pty spawn");
    // `PtyHandle` wraps a portable-pty master, which is not `Send`/`Sync`.
    // We keep it on the main thread for the spike (gpui runs single-threaded
    // in foreground anyway), so the `Arc` is fine despite clippy's warning.
    #[allow(clippy::arc_with_non_send_sync)]
    let pty: Arc<PtyHandle> = Arc::new(pty);

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1024.0), px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FileTerm GPUI Spike — G-1.4".into()),
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
            |_window, cx| {
                cx.new(|cx| TermView::new(cx, pty.clone(), 80, 24))
            },
        )
        .unwrap();
    });
}

/// Pick a shell for the spike. `bash` is preferred (most Linux distros ship
/// it and enable color prompts by default); fall back to `sh` if `bash` is
/// missing so the example still runs on minimal containers.
fn which_shell() -> &'static str {
    if std::path::Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else if std::path::Path::new("/usr/bin/bash").exists() {
        "/usr/bin/bash"
    } else {
        "sh"
    }
}

// Touch `Context` so the import stays meaningful even if future refactors
// drop the explicit use site. (`cx.new` is a `Context`-trait method; we
// don't reference the type by name in `main`.)
#[allow(dead_code)]
fn _ctx_marker<T: 'static>(_: &Context<T>) {}
