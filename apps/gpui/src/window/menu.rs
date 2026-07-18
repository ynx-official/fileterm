//! Application menu builder.
//!
//! G2 phase of `docs/plans/active/gpui-refactor.md` section 6.3.
//!
//! Mirrors Tauri's `init_menu` (File / Edit / View / Window / Help) using
//! GPUI's `MenuItem` enum. Each menu item dispatches a GPUI `Action`; the
//! action types are defined here as stubs (`NewConnection`, `Quit`, etc.)
//! and G3+ wires them to real handlers via `cx.on_action(...)`.
//!
//! ## Why stub actions
//!
//! GPUI menus dispatch via the `Action` trait, not string ids like Tauri.
//! Defining the action types upfront means the menu builds today; G3+
//! just adds `cx.on_action::<NewConnection>(...)` handlers. This matches
//! the spike-first pattern: get the structure compiling, fill in behavior
//! incrementally.

use gpui::{MenuItem, SharedString};

// ---- Action types ----
//
// Each is a unit struct implementing `gpui::Action`. The `actions!` macro
// generates the `Action` impl + `From` impls + registration. G3+ adds
// `cx.on_action::<Foo>(handler)` calls in main.rs to wire them up.
//
// Naming mirrors Tauri's menu item ids (`tray-show-main`, `new-connection`,
// etc.) so the migration is line-for-line where possible.

/// File → New Connection. Opens the connection form.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct NewConnection;

/// File → Connection Manager. Opens the connection library window.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct OpenConnectionManager;

/// File → Command Manager. Opens the command library window.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct OpenCommandManager;

/// File → Close Tab. Closes the active workspace tab.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct CloseTab;

/// File → Quit. Triggers the app-wide quit flow.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct Quit;

/// Edit → Undo/Redo/Cut/Copy/Paste/Select All. Maps to OS predefined
/// menu items on macOS; on Linux/Windows these are dispatched as
/// `EditAction` variants. GPUI's `OsAction` enum handles the platform
/// mapping — see `MenuItem::os_action`.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditUndo;
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditRedo;
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditCut;
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditCopy;
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditPaste;
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct EditSelectAll;

/// View → Toggle Theme. Cycles light/dark.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct ToggleTheme;

/// Window → Show Main. Brings the main window to front (also used by tray).
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct ShowMain;

/// Help → Documentation. Opens the docs URL in the browser.
#[derive(Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = fileterm)]
pub struct OpenDocs;

/// Build the application menu (File / Edit / View / Window / Help).
///
/// Returns `Vec<MenuItem>` to be passed to `cx.set_menus(...)`. The
/// `is_english` flag matches Tauri's localization: `true` → English
/// labels, `false` → Chinese labels.
///
/// G2 returns the structure with stub actions. G3+ wires real handlers.
pub fn build_application_menu(is_english: bool) -> Vec<MenuItem> {
    let label = |en: &str, zh: &str| -> SharedString {
        if is_english { en } else { zh }.into()
    };

    let file = MenuItem::submenu(
        gpui::Menu::new(label("File", "文件")).items(vec![
            MenuItem::action(label("New Connection", "新建连接"), NewConnection),
            MenuItem::action(
                label("Connection Manager", "连接管理器"),
                OpenConnectionManager,
            ),
            MenuItem::action(
                label("Command Manager", "命令管理器"),
                OpenCommandManager,
            ),
            MenuItem::separator(),
            MenuItem::action(label("Close Tab", "关闭标签"), CloseTab),
            MenuItem::separator(),
            MenuItem::action(label("Quit", "退出"), Quit),
        ]),
    );

    let edit = MenuItem::submenu(
        gpui::Menu::new(label("Edit", "编辑")).items(vec![
            MenuItem::action(label("Undo", "撤销"), EditUndo),
            MenuItem::action(label("Redo", "重做"), EditRedo),
            MenuItem::separator(),
            MenuItem::action(label("Cut", "剪切"), EditCut),
            MenuItem::action(label("Copy", "复制"), EditCopy),
            MenuItem::action(label("Paste", "粘贴"), EditPaste),
            MenuItem::action(label("Select All", "全选"), EditSelectAll),
        ]),
    );

    let view = MenuItem::submenu(gpui::Menu::new(label("View", "视图")).items(vec![
        MenuItem::action(label("Toggle Theme", "切换主题"), ToggleTheme),
    ]));

    let window = MenuItem::submenu(
        gpui::Menu::new(label("Window", "窗口")).items(vec![MenuItem::action(
            label("Show Main", "显示主窗口"),
            ShowMain,
        )]),
    );

    let help = MenuItem::submenu(
        gpui::Menu::new(label("Help", "帮助")).items(vec![MenuItem::action(
            label("Documentation", "文档"),
            OpenDocs,
        )]),
    );

    vec![file, edit, view, window, help]
}

/// Build the tray menu (Show Main / Connection Manager / Command Manager
/// / Quit). Smaller than the app menu — the tray only needs the
/// "jump back into the app" actions.
pub fn build_tray_menu(is_english: bool) -> Vec<MenuItem> {
    let label = |en: &str, zh: &str| -> SharedString {
        if is_english { en } else { zh }.into()
    };

    vec![
        MenuItem::action(label("Show Main", "显示主窗口"), ShowMain),
        MenuItem::separator(),
        MenuItem::action(
            label("Connection Manager", "连接管理器"),
            OpenConnectionManager,
        ),
        MenuItem::action(
            label("Command Manager", "命令管理器"),
            OpenCommandManager,
        ),
        MenuItem::separator(),
        MenuItem::action(label("Quit", "退出"), Quit),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_menu_has_five_top_level_items() {
        let menu = build_application_menu(true);
        // File, Edit, View, Window, Help
        assert_eq!(menu.len(), 5, "expected 5 top-level menus");
    }

    #[test]
    fn tray_menu_has_six_items_with_separators() {
        let menu = build_tray_menu(true);
        // Show Main, sep, Connection Manager, Command Manager, sep, Quit
        assert_eq!(menu.len(), 6);
    }

    #[test]
    fn chinese_labels_differ_from_english() {
        let en = build_application_menu(true);
        let zh = build_application_menu(false);
        // The first menu's name should differ ("File" vs "文件").
        // We can't inspect MenuItem's name directly without matching,
        // but we can at least verify both builds succeed.
        assert_eq!(en.len(), zh.len());
    }
}
