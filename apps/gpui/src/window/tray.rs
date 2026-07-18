//! System tray handler.
//!
//! G2 phase of `docs/plans/active/gpui-refactor.md` section 6.3.
//!
//! ## What G2 delivers
//!
//! The `TrayHandler` struct + platform branches. Real platform glue
//! (native icon, click → show/hide) needs platform-specific APIs that
//! GPUI 1.8.2 exposes via `cx.os_actions()` / `Window::activate_window`;
//! G2 ships the structure with a `setup` method that G3+ calls from
//! `main.rs`.
//!
//! ## Platform differences (mirroring Tauri's tray.rs)
//!
//! * **macOS**: tray icon in the menu bar; left-click toggles dock
//!   visibility. `Window::show` / `Window::hide` are no-ops when the app
//!   is in the background — must call `NSApplication::activateIgnoringOtherApps`.
//! * **Windows**: tray icon in the system tray; left-click restores the
//!   window. Right-click shows the context menu.
//! * **Linux**: tray icon depends on the desktop environment (StatusNotifierItem
//!   on modern KDE/GNOME; legacy XEmbed on older WMs). GPUI's `TrayHandle`
//!   abstracts this; we just provide the menu + click handler.

use gpui::App;

use super::menu::build_tray_menu;

/// Handles the system tray icon + menu.
///
/// Held as `Option<TrayHandler>` in the app — on platforms without tray
/// support (or when the user disabled it), this stays `None`.
pub struct TrayHandler {
    /// Whether the tray menu uses English labels. Matches the app menu
    /// locale so a Chinese user sees Chinese tray items.
    is_english: bool,
}

impl TrayHandler {
    /// Construct with the given locale. Doesn't actually create the
    /// tray icon yet — call [`setup`](Self::setup) to do that.
    pub fn new(is_english: bool) -> Self {
        Self { is_english }
    }

    /// Create the tray icon + attach the menu.
    ///
    /// G2 stub: logs the intent but doesn't call GPUI's tray API yet
    /// (which is `cx.add_tray_icon(...)` and needs an image asset we
    /// don't have in the spike). G3+ fills this in with a real icon.
    pub fn setup(&self, _cx: &mut App) {
        // G3+: cx.add_tray_icon(TrayIconBuilder::new()
        //     .icon(default_icon())
        //     .menu(build_tray_menu(self.is_english))
        //     .on_tray_icon_event(|event| match event.click {
        //         TrayClick::Left => { show_main_window(cx); }
        //         _ => {}
        //     })
        //     .build(cx)?);
        let _menu = build_tray_menu(self.is_english);
        // Intentionally no-op for G2.
    }

    /// Toggle the main window's visibility (macOS dock show/hide
    /// semantics). Called from the tray's left-click handler.
    ///
    /// G2 stub — G3+ wires to `Window::show` / `Window::hide` via the
    /// `WindowRegistry`'s main-window handle.
    pub fn toggle_main_window(&self, _cx: &mut App) {
        // G3+: let handle = registry.handle_for("main")?;
        //         handle.update(cx, |_, window| {
        //             if window.is_window_visible() { window.hide(); }
        //             else { window.show(); }
        //         });
    }

    /// Update the locale of the tray menu at runtime. Called when the
    /// user changes the UI language preference.
    ///
    /// G2 stub — G3+ calls `tray_icon.set_menu(...)` with the rebuilt menu.
    pub fn set_locale(&mut self, is_english: bool, _cx: &mut App) {
        self.is_english = is_english;
        // G3+: rebuild + reattach menu.
    }
}

/// Default tray icon as raw bytes (PNG). G2 uses a 1x1 transparent
/// placeholder — G3+ replaces with the real FileTerm brand icon.
///
/// Embedded via `include_bytes!` so the binary is self-contained (per
/// the workspace rule "离线资源就地化：严禁运行时动态拉取外部 CDN 资源").
#[cfg(not(target_os = "macos"))]
const DEFAULT_TRAY_ICON: &[u8] = &[
    // Minimal 1x1 transparent PNG (67 bytes). Real icon lands in G3.
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[cfg(target_os = "macos")]
const DEFAULT_TRAY_ICON: &[u8] = DEFAULT_TRAY_ICON_PLACEHOLDER;
#[cfg(target_os = "macos")]
const DEFAULT_TRAY_ICON_PLACEHOLDER: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// Get the default tray icon bytes. G3+ replaces with the real brand icon.
pub fn default_tray_icon() -> &'static [u8] {
    DEFAULT_TRAY_ICON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_handler_construction() {
        let handler = TrayHandler::new(true);
        // Just verify construction doesn't panic; setup is a no-op in G2.
        assert!(handler.is_english);
    }

    #[test]
    fn default_icon_is_non_empty_png() {
        let icon = default_tray_icon();
        // PNG magic bytes.
        assert_eq!(
            &icon[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert!(icon.len() > 50);
    }
}
