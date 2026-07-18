//! `WindowRegistry` — tracks which tab is in which window.
//!
//! G2 phase of `docs/plans/active/gpui-refactor.md` section 4.2.3 + 6.3.
//!
//! ## Why a registry
//!
//! GPUI windows are opened via `cx.open_window(...)` which returns a
//! `WindowHandle<V>`. There's no built-in way to look up "which window
//! has tab X" or "list all detached-session windows". The registry fills
//! that gap with three `RwLock<HashMap<...>>` maps:
//!
//! * `detached_tabs`: window_id → Vec<tab_id> (forward lookup)
//! * `tab_owner`: tab_id → window_id (reverse lookup, for "find my window")
//! * `handles`: window_id → WindowHandle (for `window.update(cx, ...)`)
//!
//! `main` window's tabs are tracked by `WorkspaceState` (G3), not here —
//! the registry only tracks *detached* tab ownership, because that's the
//! only case where a tab's window isn't implicitly "main".
//!
//! ## Concurrency
//!
//! `parking_lot::RwLock` because reads (window_for_tab, list_placements)
//! vastly outnumber writes (detach/return), and readers shouldn't block
//! each other. Writes are short (HashMap insert/remove) so the write
//! lock is held for microseconds at most.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

/// Placeholder for the root view type. G3 will replace this with the
/// real `WorkspaceView` / `RootView` type; for G2 we just need *a* type
/// parameter so `WindowHandle` compiles. Using `()` would work but
/// `WindowHandle<()>` is misleading — `RootViewPlaceholder` makes the
/// "this is temporary" intent explicit.
///
/// When G3 lands, find-and-replace `RootViewPlaceholder` with the real
/// view type across this module.
pub struct RootViewPlaceholder;

/// Where a tab currently lives.
///
/// Serialized to JSON and broadcast to all windows on every change so
/// the UI can render the correct tab strip (tabs in main window vs.
/// tabs in their own windows). Matches Tauri's
/// `WorkspaceTabPlacement` shape line-for-line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkspaceTabPlacement {
    /// The tab's id (same as `tab_id` in `WorkspaceState`).
    pub tab_id: String,
    /// The window id the tab currently lives in. `"main"` for the main
    /// window, `"detached-session-{id}"` for a detached window.
    pub window_id: String,
}

/// Tracks detached tab → window ownership.
///
/// See module docs for the three-map design. Held as `Arc<WindowRegistry>`
/// inside `GpuiDesktopApi` so all views share one registry.
#[derive(Default)]
pub struct WindowRegistry {
    /// window_id → list of tab_ids in that window. Only contains
    /// detached-session windows; main window's tabs are owned by
    /// `WorkspaceState` and not tracked here.
    detached_tabs: RwLock<HashMap<String, Vec<String>>>,
    /// tab_id → window_id. Reverse lookup for "which window has this
    /// tab". Lets `window_for_tab` be O(1) instead of scanning every
    /// window's tab list.
    tab_owner: RwLock<HashMap<String, String>>,
    /// window_id → WindowHandle. Stored as `gpui::WindowHandle` so we
    /// can `window.update(cx, |view, cx| ...)` from anywhere with just
    /// the window_id. G2 stores handles opaquely (we don't call them
    /// yet); G3+ uses them for close-request routing.
    ///
    /// The handle is `Option<...>` because we register a window's label
    /// in `detached_tabs` *before* its handle is known (the window-open
    /// callback runs after `cx.open_window` returns). `None` means
    /// "registered but not yet opened".
    handles: RwLock<HashMap<String, gpui::WindowHandle<RootViewPlaceholder>>>,
}

impl WindowRegistry {
    /// Create an empty registry. The main window is implicitly present
    /// (label `"main"`) but not tracked here — only detached windows
    /// appear in the registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a window's handle after `cx.open_window` returns.
    ///
    /// Called from the window-open callback. The window's label may
    /// already be in `detached_tabs` (if tabs were pre-detached before
    /// the window opened); this just fills in the handle.
    pub fn register_handle(
        &self,
        window_id: &str,
        handle: gpui::WindowHandle<RootViewPlaceholder>,
    ) {
        self.handles.write().insert(window_id.to_string(), handle);
    }

    /// Detach a tab from its current window into a target window.
    ///
    /// `tab_id` is moved from its current owner (found via `tab_owner`)
    /// to `target_window_id`. If the tab wasn't tracked (i.e. it was in
    /// the main window, which isn't in `detached_tabs`), this just
    /// records the new ownership. The actual view movement (closing the
    /// tab in the old window, opening it in the new one) is G5's job;
    /// this method only updates the bookkeeping.
    pub fn detach_tab(&self, tab_id: &str, target_window_id: &str) {
        let mut detached = self.detached_tabs.write();
        let mut owner = self.tab_owner.write();

        // Remove from old window's tab list if it was already detached.
        if let Some(old_window) = owner.get(tab_id).cloned() {
            if let Some(tabs) = detached.get_mut(&old_window) {
                tabs.retain(|t| t != tab_id);
                // Don't remove the window entry even if empty — the
                // window might still be open with zero tabs (user
                // dragged the last tab out). Window removal is handled
                // by `return_tabs_to_main` on close.
            }
        }

        // Add to new window.
        detached
            .entry(target_window_id.to_string())
            .or_default()
            .push(tab_id.to_string());
        owner.insert(tab_id.to_string(), target_window_id.to_string());
    }

    /// Return all tabs from a window to the main window.
    ///
    /// Called when a detached-session window closes. Returns the list
    /// of tab_ids that were in it so the caller (G5 close handler) can
    /// re-open them in the main window. Also removes the window from
    /// `detached_tabs` and `handles`.
    pub fn return_tabs_to_main(&self, window_id: &str) -> Vec<String> {
        let mut detached = self.detached_tabs.write();
        let mut owner = self.tab_owner.write();
        let mut handles = self.handles.write();

        let tabs = detached.remove(window_id).unwrap_or_default();
        for tab_id in &tabs {
            owner.remove(tab_id);
        }
        handles.remove(window_id);
        tabs
    }

    /// Return a *single* tab to the main window.
    ///
    /// G5 addition — `return_tabs_to_main` is window-scoped (returns
    /// every tab in the window), but the "Move to main window" tab
    /// context-menu action moves only one tab while leaving any sibling
    /// tabs in the detached window. This method removes just `tab_id`
    /// from its current owner's list + the reverse-lookup map, leaving
    /// the rest of the window intact.
    ///
    /// Returns `true` if the tab was tracked (and is now back in main),
    /// `false` if it wasn't detached in the first place.
    pub fn return_tab_to_main(&self, tab_id: &str) -> bool {
        let mut detached = self.detached_tabs.write();
        let mut owner = self.tab_owner.write();

        let Some(old_window) = owner.remove(tab_id) else {
            return false;
        };
        if let Some(tabs) = detached.get_mut(&old_window) {
            tabs.retain(|t| t != tab_id);
            // Intentionally keep the window entry even if now empty —
            // matches `detach_tab`'s policy (window may still be open
            // with zero tabs; cleanup happens on window close).
        }
        true
    }

    /// Find which window a tab is currently in.
    ///
    /// Returns `None` if the tab is in the main window (not tracked) or
    /// unknown. Used by event routers to send tab-targeted events to
    /// the right window.
    pub fn window_for_tab(&self, tab_id: &str) -> Option<String> {
        self.tab_owner.read().get(tab_id).cloned()
    }

    /// Get the `WindowHandle` for a window id.
    ///
    /// Returns `None` if the window isn't open yet (registered in
    /// `detached_tabs` but `register_handle` hasn't been called) or
    /// already closed.
    pub fn handle_for(&self, window_id: &str) -> Option<gpui::WindowHandle<RootViewPlaceholder>> {
        self.handles.read().get(window_id).copied()
    }

    /// List all tab placements (tab_id + window_id pairs).
    ///
    /// Broadcast to all windows on every detach/return so the UI can
    /// render the correct tab strip. Only includes detached tabs — main
    /// window tabs are tracked by `WorkspaceState` and merged in by the
    /// caller.
    pub fn list_placements(&self) -> Vec<WorkspaceTabPlacement> {
        let detached = self.detached_tabs.read();
        let mut out = Vec::new();
        for (window_id, tabs) in detached.iter() {
            for tab_id in tabs {
                out.push(WorkspaceTabPlacement {
                    tab_id: tab_id.clone(),
                    window_id: window_id.clone(),
                });
            }
        }
        out
    }

    /// List all open detached-session window ids.
    ///
    /// Used by the crash-recovery path (refactor.md 4.2.4): on startup,
    /// scan `detached_tabs` for window_ids that have no handle and
    /// return their tabs to main.
    pub fn detached_window_ids(&self) -> Vec<String> {
        self.detached_tabs.read().keys().cloned().collect()
    }

    /// Debug consistency check.
    ///
    /// Per refactor.md risk table: "WindowRegistry 提供
    /// `assert_consistency()` 在 debug build 每帧检查". Verifies the
    /// forward and reverse maps agree. Panics on mismatch — a mismatch
    /// means a bug in detach/return bookkeeping.
    pub fn assert_consistency(&self) {
        let detached = self.detached_tabs.read();
        let owner = self.tab_owner.read();

        // Every tab in detached_tabs must have a matching tab_owner entry.
        for (window_id, tabs) in detached.iter() {
            for tab_id in tabs {
                match owner.get(tab_id) {
                    Some(owning_window) => {
                        assert_eq!(
                            owning_window, window_id,
                            "tab {} is in detached_tabs[{}] but tab_owner says {}",
                            tab_id, window_id, owning_window
                        );
                    }
                    None => panic!(
                        "tab {} is in detached_tabs[{}] but missing from tab_owner",
                        tab_id, window_id
                    ),
                }
            }
        }

        // Every tab_owner entry must appear in some detached_tabs list.
        for (tab_id, window_id) in owner.iter() {
            let tabs = detached.get(window_id).unwrap_or_else(|| {
                panic!(
                    "tab {} tab_owner says {} but window has no detached_tabs entry",
                    tab_id, window_id
                )
            });
            assert!(
                tabs.contains(tab_id),
                "tab {} tab_owner says {} but not in that window's detached_tabs list",
                tab_id,
                window_id
            );
        }
    }
}

/// Convenience alias — the registry is always shared (`Arc<...>`).
pub type SharedWindowRegistry = Arc<WindowRegistry>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detach_then_return_roundtrip() {
        let reg = WindowRegistry::new();
        // Detach tab-1 into window "detached-session-1".
        reg.detach_tab("tab-1", "detached-session-1");
        assert_eq!(
            reg.window_for_tab("tab-1"),
            Some("detached-session-1".into())
        );
        assert_eq!(
            reg.list_placements(),
            vec![WorkspaceTabPlacement {
                tab_id: "tab-1".into(),
                window_id: "detached-session-1".into(),
            }]
        );
        // Return it.
        let returned = reg.return_tabs_to_main("detached-session-1");
        assert_eq!(returned, vec!["tab-1".to_string()]);
        assert_eq!(reg.window_for_tab("tab-1"), None);
        assert!(reg.list_placements().is_empty());
    }

    #[test]
    fn detach_moves_between_windows() {
        let reg = WindowRegistry::new();
        reg.detach_tab("tab-1", "detached-session-1");
        reg.detach_tab("tab-1", "detached-session-2");
        // Should be in window 2, not window 1.
        assert_eq!(
            reg.window_for_tab("tab-1"),
            Some("detached-session-2".into())
        );
        let placements = reg.list_placements();
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].window_id, "detached-session-2");
    }

    #[test]
    fn return_empty_window() {
        let reg = WindowRegistry::new();
        let returned = reg.return_tabs_to_main("never-existed");
        assert!(returned.is_empty());
    }

    #[test]
    fn multiple_tabs_in_one_window() {
        let reg = WindowRegistry::new();
        reg.detach_tab("tab-1", "detached-session-1");
        reg.detach_tab("tab-2", "detached-session-1");
        reg.detach_tab("tab-3", "detached-session-1");
        let placements = reg.list_placements();
        assert_eq!(placements.len(), 3);
        for p in &placements {
            assert_eq!(p.window_id, "detached-session-1");
        }
        let returned = reg.return_tabs_to_main("detached-session-1");
        assert_eq!(returned.len(), 3);
    }

    #[test]
    fn consistency_check_passes_after_operations() {
        let reg = WindowRegistry::new();
        reg.detach_tab("tab-1", "detached-session-1");
        reg.detach_tab("tab-2", "detached-session-1");
        reg.detach_tab("tab-1", "detached-session-2");
        reg.return_tabs_to_main("detached-session-1");
        // Should not panic.
        reg.assert_consistency();
    }

    #[test]
    fn detached_window_ids_lists_all() {
        let reg = WindowRegistry::new();
        reg.detach_tab("tab-1", "detached-session-1");
        reg.detach_tab("tab-2", "detached-session-2");
        let mut ids = reg.detached_window_ids();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "detached-session-1".to_string(),
                "detached-session-2".to_string()
            ]
        );
    }

    #[test]
    fn return_single_tab_leaves_siblings_in_window() {
        // G5: "Move to main window" on one tab must not evacuate the
        // whole detached window. Sibling tabs stay put.
        let reg = WindowRegistry::new();
        reg.detach_tab("tab-1", "detached-session-1");
        reg.detach_tab("tab-2", "detached-session-1");
        reg.detach_tab("tab-3", "detached-session-1");

        let moved = reg.return_tab_to_main("tab-2");
        assert!(
            moved,
            "return_tab_to_main should report success for tracked tab"
        );

        // tab-2 is gone from the registry; tab-1 and tab-3 remain.
        assert!(reg.window_for_tab("tab-2").is_none());
        assert_eq!(
            reg.window_for_tab("tab-1").as_deref(),
            Some("detached-session-1")
        );
        assert_eq!(
            reg.window_for_tab("tab-3").as_deref(),
            Some("detached-session-1")
        );
        // Placement list reflects the change.
        let placements = reg.list_placements();
        assert_eq!(placements.len(), 2);
        assert!(
            !placements.iter().any(|p| p.tab_id == "tab-2"),
            "tab-2 should not appear in placements after return"
        );
    }

    #[test]
    fn return_single_tab_for_untracked_returns_false() {
        // Calling return_tab_to_main on a tab that's already in main
        // (not tracked) is a no-op and reports false.
        let reg = WindowRegistry::new();
        let moved = reg.return_tab_to_main("never-detached");
        assert!(!moved);
    }
}
