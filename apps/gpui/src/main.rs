//! FileTerm GPUI runtime entry point.
//!
//! The binary only owns runtime bootstrapping and dependency assembly.
//! Product state lives in `state`, visual composition in `view`, and system
//! capabilities behind `FileTermDesktopApi`.

use std::sync::Arc;

use gpui::{
    px, size, App, AppContext, Bounds, Focusable, KeyBinding, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowKind, WindowOptions,
};

use fileterm_gpui::{
    backend::{AppHandle, FileTermDesktopApi, GpuiDesktopApi},
    view::RootView,
    window::{
        menu::{build_application_menu, Quit, ToggleTheme},
        WindowRegistry,
    },
};

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _runtime_guard = runtime.enter();

    let app_handle = Arc::new(
        AppHandle::platform_default().expect("resolve FileTerm application data directory"),
    );
    let desktop_api: Arc<dyn FileTermDesktopApi> = Arc::new(GpuiDesktopApi::new(app_handle));

    gpui_platform::application().run(move |cx: &mut App| {
        cx.activate(true);
        cx.set_menus(build_application_menu(false));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-shift-l", ToggleTheme, Some("FileTerm")),
            KeyBinding::new("ctrl-shift-l", ToggleTheme, Some("FileTerm")),
            KeyBinding::new("cmd-q", Quit, None),
        ]);

        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let api = desktop_api.clone();
        let window_registry = Arc::new(WindowRegistry::new());
        let registry_for_root = window_registry.clone();
        let main_handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("FileTerm".into()),
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
                move |window, cx| {
                    let root =
                        cx.new(|cx| RootView::new(api.clone(), registry_for_root.clone(), cx));
                    root.focus_handle(cx).focus(window, cx);
                    root
                },
            )
            .expect("open FileTerm main window");
        let main_window_id = main_handle.window_id();
        cx.on_window_closed(move |cx, window_id| {
            if window_id == main_window_id {
                cx.quit();
                return;
            }
            let returned_tabs = window_registry.return_closed_window_to_main(window_id);
            if !returned_tabs.is_empty() {
                let _ = main_handle.update(cx, |root, _, cx| {
                    root.restore_detached_tabs(returned_tabs, cx);
                });
            }
        })
        .detach();
    });
}
