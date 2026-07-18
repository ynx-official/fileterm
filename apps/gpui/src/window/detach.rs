//! Detach orchestration — high-level "move tab to new window" logic.
//!
//! G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
//!
//! This module is the **pure-logic** half of detach. It owns:
//! * generating the new window id (`detached-session-{uuid}`),
//! * updating the [`WindowRegistry`] bookkeeping,
//! * returning the updated placement list so the caller can broadcast it.
//!
//! It does **not** open the GPUI window — that requires `&mut App` and
//! a root view, which the bridge layer doesn't have. The caller (a
//! `cx.open_window` site in the future `view::workspace` module) is
//! responsible for:
//! 1. calling [`detach_tab_to_new_window`] to reserve the window id +
//!    update the registry,
//! 2. calling `cx.open_window(...)` with that id encoded in the title,
//! 3. calling [`WindowRegistry::register_handle`] from the window-open
//!    callback so subsequent placement broadcasts can route to it.
//!
//! Splitting logic from GPUI API keeps this module unit-testable without
//! spinning up a real `App` / window.

use uuid::Uuid;

use super::registry::{SharedWindowRegistry, WorkspaceTabPlacement};

/// Result of a detach operation. Returned to the caller so it can:
/// * open a GPUI window with `new_window_id` encoded (e.g. in the title),
/// * broadcast `placements` to all windows via the placements-changed
///   channel (refactor.md 4.2.2 event routing table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachResult {
    /// The freshly-minted `detached-session-{uuid}` id. Caller uses this
    /// as the key when calling `WindowRegistry::register_handle`.
    pub new_window_id: String,
    /// The tab that was detached. Echoed back for caller convenience
    /// (the caller already knows it, but having it in the result makes
    /// the broadcast payload self-contained).
    pub tab_id: String,
    /// Full placement snapshot after the detach. Broadcast this to every
    /// open window so their tab strips re-render.
    pub placements: Vec<WorkspaceTabPlacement>,
}

/// Detach `tab_id` into a freshly-minted `detached-session-{uuid}` window.
///
/// Updates the registry bookkeeping (forward + reverse maps) and returns
/// the new window id + updated placement list. The caller is responsible
/// for actually opening the GPUI window and registering its handle.
///
/// # Id format
///
/// `detached-session-{uuid}` where `{uuid}` is a v4 UUID. Matches the
/// format documented in refactor.md 4.2.1 (`detached-session-{id}`) so
/// the renderer can parse the kind back out of the window id if needed.
pub fn detach_tab_to_new_window(registry: &SharedWindowRegistry, tab_id: &str) -> DetachResult {
    let new_window_id = format!("detached-session-{}", Uuid::new_v4());
    registry.detach_tab(tab_id, &new_window_id);
    DetachResult {
        new_window_id,
        tab_id: tab_id.to_string(),
        placements: registry.list_placements(),
    }
}

/// Move a tab back to the main window. Symmetric inverse of
/// [`detach_tab_to_new_window`].
///
/// Called when:
/// * the user picks "Move to main window" from the tab context menu,
/// * a detached-session window closes (refactor.md 4.2.4 close chain),
/// * crash recovery on startup finds a window id in `detached_tabs` with
///   no live handle (refactor.md 4.2.4 recovery path).
///
/// Returns the list of tab ids that were in the window (always ≥0; the
/// caller may have already removed the specific tab it cares about).
pub fn return_window_to_main(registry: &SharedWindowRegistry, window_id: &str) -> Vec<String> {
    registry.return_tabs_to_main(window_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::WindowRegistry;
    use std::sync::Arc;

    fn fresh_registry() -> SharedWindowRegistry {
        Arc::new(WindowRegistry::new())
    }

    #[test]
    fn detach_assigns_new_window_id_and_updates_placements() {
        let reg = fresh_registry();
        let result = detach_tab_to_new_window(&reg, "tab-A");
        // Window id follows the documented format.
        assert!(result.new_window_id.starts_with("detached-session-"));
        assert_eq!(result.tab_id, "tab-A");
        // Placement list reflects the new ownership.
        assert_eq!(result.placements.len(), 1);
        assert_eq!(result.placements[0].tab_id, "tab-A");
        assert_eq!(result.placements[0].window_id, result.new_window_id);
        // Registry reverse-lookup agrees.
        assert_eq!(
            reg.window_for_tab("tab-A").as_deref(),
            Some(result.new_window_id.as_str())
        );
    }

    #[test]
    fn detach_twice_produces_distinct_window_ids() {
        let reg = fresh_registry();
        let r1 = detach_tab_to_new_window(&reg, "tab-1");
        let r2 = detach_tab_to_new_window(&reg, "tab-2");
        assert_ne!(r1.new_window_id, r2.new_window_id);
        // Both tabs are tracked.
        assert_eq!(reg.list_placements().len(), 2);
    }

    #[test]
    fn detach_then_return_clears_bookkeeping() {
        let reg = fresh_registry();
        let detach = detach_tab_to_new_window(&reg, "tab-X");
        let returned = return_window_to_main(&reg, &detach.new_window_id);
        assert_eq!(returned, vec!["tab-X".to_string()]);
        // After return, the tab is no longer in any detached window.
        assert!(reg.window_for_tab("tab-X").is_none());
        assert!(reg.list_placements().is_empty());
    }

    #[test]
    fn return_unknown_window_is_noop_returns_empty() {
        let reg = fresh_registry();
        let returned = return_window_to_main(&reg, "detached-session-nonexistent");
        assert!(returned.is_empty());
    }

    #[test]
    fn detach_moving_tab_between_detached_windows_clears_old_owner() {
        let reg = fresh_registry();
        let first = detach_tab_to_new_window(&reg, "tab-M");
        // Detach the same tab into a second new window — the first
        // window should no longer own it.
        let second = detach_tab_to_new_window(&reg, "tab-M");
        assert_ne!(first.new_window_id, second.new_window_id);
        assert_eq!(
            reg.window_for_tab("tab-M").as_deref(),
            Some(second.new_window_id.as_str())
        );
        // The first window's tab list should be empty now.
        let placements: Vec<_> = reg
            .list_placements()
            .into_iter()
            .filter(|p| p.window_id == first.new_window_id)
            .collect();
        assert!(placements.is_empty(), "first window should have no tabs");
    }
}
