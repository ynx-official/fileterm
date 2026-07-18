//! Tab drag-and-drop state — tracks the in-flight drag between windows.
//!
//! G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
//!
//! GPUI doesn't expose cross-window drag-and-drop as a built-in (the
//! `Drag` element is single-window). The tab strip tracks drags manually:
//! on `MouseDown` outside any tab's hit box we call [`TabDragState::start`],
//! on `MouseMove` we update the visual ghost (renderer-side), on `MouseUp`
//! we call [`TabDragState::finish`] which classifies the drop target:
//!
//! * drop inside another window's bounds → "move to that window"
//! * drop on empty screen → "detach to new window"
//! * drop inside the source window → "no-op (in-window reorder is
//!   renderer-side)"
//!
//! Only one drag can be in-flight at a time (`start` errors if a drag
//! is already active). This matches Tauri's `WorkspaceState::tab_drag`
//! constraint — see `apps/tauri/src-tauri/src/services/workspace_window_registry.rs`.

use std::time::Instant;

/// Identifier for the source window of a drag. `"main"` for the main
/// window, `detached-session-{uuid}` for a detached window. Mirrors
/// `WorkspaceTabPlacement::window_id`.
pub type WindowId = String;

/// A tab drag operation in progress (or `None` if no drag is active).
///
/// Held as `Entity<TabDragState>` inside the workspace view so all tab
/// strips see the same drag state (a drag started in window A might
/// finish in window B; both need to read the same `tab_id`).
#[derive(Debug, Default)]
pub struct TabDragState {
    /// `Some(...)` while a drag is in flight; `None` otherwise.
    /// Encapsulated so callers can't mutate fields individually and
    /// leave the state inconsistent (e.g. set `tab_id` without resetting
    /// `started_at`).
    active: Option<ActiveDrag>,
}

#[derive(Debug, Clone)]
pub struct ActiveDrag {
    /// Which tab is being dragged. Same id space as `WorkspaceTabPlacement::tab_id`.
    pub tab_id: String,
    /// Which window the drag started from. Used by the finish classifier
    /// to detect "dropped in source window" (no-op) vs "dropped in another
    /// window" (move) vs "dropped on empty screen" (detach).
    pub source_window_id: WindowId,
    /// When the drag started, for analytics / "drag took too long, treat
    /// as click" heuristics. Renderer-side decides the threshold.
    pub started_at: Instant,
}

/// Outcome of [`TabDragState::finish`]. The caller (renderer) is
/// responsible for invoking the matching bridge command
/// (`workspace_move_tab` / `workspace_detach_tab` / no-op).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragDropTarget {
    /// Dropped inside the source window — in-window reorder, handled by
    /// the renderer's tab strip; no bridge call needed.
    SameWindow,
    /// Dropped inside another window — caller should call
    /// `workspace_move_tab` with the target window id.
    OtherWindow(WindowId),
    /// Dropped on empty screen — caller should call
    /// `workspace_detach_tab` with the screen coordinates.
    NewWindow,
}

impl TabDragState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a drag is currently in flight.
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The tab id being dragged, if any. Convenience accessor.
    pub fn active_tab_id(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.tab_id.as_str())
    }

    /// Start a new drag. Returns `true` if the drag was started, `false`
    /// if one is already in flight (the renderer should call `cancel`
    /// first if it lost a MouseUp, e.g. focus stolen by another app).
    ///
    /// The bool return shape avoids `Result<(), ()>` (clippy::result_unit_err)
    /// — there's no information to convey beyond "started or not", so a
    /// bool is the natural type.
    pub fn start(&mut self, tab_id: &str, source_window_id: &str) -> bool {
        if self.active.is_some() {
            return false;
        }
        self.active = Some(ActiveDrag {
            tab_id: tab_id.to_string(),
            source_window_id: source_window_id.to_string(),
            started_at: Instant::now(),
        });
        true
    }

    /// Cancel the in-flight drag (e.g. ESC pressed, focus lost). No-op
    /// if no drag is active.
    pub fn cancel(&mut self) {
        self.active = None;
    }

    /// Finish the drag at the given screen coordinates. The classifier
    /// decides what kind of drop this is based on which window's bounds
    /// contain `(screen_x, screen_y)`.
    ///
    /// `windows_in_z_order` is the list of `(window_id, bounds)` tuples
    /// for all open windows, topmost first. The first window whose
    /// bounds contain the drop point is the target. If none match, it's
    /// a detach-to-new-window drop.
    ///
    /// Returns `None` if no drag was active (the renderer called
    /// `finish` without `start` — a bug, but recoverable).
    pub fn finish(
        &mut self,
        screen_x: i32,
        screen_y: i32,
        windows_in_z_order: &[(WindowId, ScreenBounds)],
    ) -> Option<DragDropTarget> {
        let active = self.active.take()?;

        // Find the topmost window whose bounds contain the drop point.
        for (window_id, bounds) in windows_in_z_order {
            if bounds.contains(screen_x, screen_y) {
                if *window_id == active.source_window_id {
                    return Some(DragDropTarget::SameWindow);
                } else {
                    return Some(DragDropTarget::OtherWindow(window_id.clone()));
                }
            }
        }

        // No window contains the point — drop on empty screen → detach.
        Some(DragDropTarget::NewWindow)
    }
}

/// Screen-space axis-aligned bounds of a window. Used by
/// [`TabDragState::finish`] for hit-testing the drop point.
///
/// Coordinates are physical pixels in the same space as the
/// `screen_x` / `screen_y` passed to `finish`. On macOS this is the
/// global display coordinate space (origin top-left of primary display
/// since GPUI 1.8); on Linux X11 it's the root window coordinate space;
/// on Windows it's the virtual screen coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ScreenBounds {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether `(px, py)` falls inside this rect (inclusive on the
    /// top-left, exclusive on the bottom-right — standard GUI convention).
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: i32, y: i32, w: i32, h: i32) -> ScreenBounds {
        ScreenBounds::new(x, y, w, h)
    }

    #[test]
    fn start_then_finish_returns_active_state_correctly() {
        let mut drag = TabDragState::new();
        assert!(!drag.is_active());
        assert!(drag.active_tab_id().is_none());

        assert!(drag.start("tab-1", "main"), "start should succeed");
        assert!(drag.is_active());
        assert_eq!(drag.active_tab_id(), Some("tab-1"));

        // Drop inside the main window's bounds → SameWindow.
        let windows = vec![("main".to_string(), bounds(0, 0, 1200, 800))];
        let target = drag.finish(100, 100, &windows);
        assert_eq!(target, Some(DragDropTarget::SameWindow));
        assert!(!drag.is_active(), "finish must clear active state");
    }

    #[test]
    fn start_twice_fails() {
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "first start should succeed");
        let second = drag.start("tab-2", "main");
        assert!(!second, "second start should fail");
        // The first drag is still active (the failed start didn't clobber it).
        assert_eq!(drag.active_tab_id(), Some("tab-1"));
    }

    #[test]
    fn cancel_clears_active_state() {
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");
        assert!(drag.is_active());
        drag.cancel();
        assert!(!drag.is_active());
    }

    #[test]
    fn drop_in_other_window_returns_other_window_target() {
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");
        // Two windows: main at (0,0,1200,800), detached at (1300,0,800,600).
        // Drop point (1400, 100) is inside the detached window.
        let windows = vec![
            ("detached-session-1".to_string(), bounds(1300, 0, 800, 600)),
            ("main".to_string(), bounds(0, 0, 1200, 800)),
        ];
        let target = drag.finish(1400, 100, &windows);
        assert_eq!(
            target,
            Some(DragDropTarget::OtherWindow("detached-session-1".into()))
        );
    }

    #[test]
    fn drop_on_empty_screen_returns_new_window_target() {
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");
        // Main window only; drop point far away.
        let windows = vec![("main".to_string(), bounds(0, 0, 1200, 800))];
        let target = drag.finish(5000, 5000, &windows);
        assert_eq!(target, Some(DragDropTarget::NewWindow));
    }

    #[test]
    fn finish_without_start_returns_none() {
        let mut drag = TabDragState::new();
        let windows: Vec<(WindowId, ScreenBounds)> = vec![];
        assert!(drag.finish(0, 0, &windows).is_none());
    }

    #[test]
    fn z_order_topmost_window_wins() {
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");
        // Two overlapping windows; the topmost (first in the list) is
        // the detached one. Drop point is in the overlap region.
        let windows = vec![
            ("detached-session-1".to_string(), bounds(1000, 0, 800, 600)),
            ("main".to_string(), bounds(0, 0, 1200, 800)),
        ];
        let target = drag.finish(1100, 100, &windows);
        assert_eq!(
            target,
            Some(DragDropTarget::OtherWindow("detached-session-1".into()))
        );
    }

    #[test]
    fn bounds_contains_inclusive_top_left_exclusive_bottom_right() {
        let b = bounds(10, 20, 100, 200);
        // Top-left corner is inside.
        assert!(b.contains(10, 20));
        // One pixel in from each edge.
        assert!(b.contains(11, 21));
        // Just before the bottom-right edge.
        assert!(b.contains(109, 219));
        // On the bottom-right edge — exclusive, so outside.
        assert!(!b.contains(110, 220));
        // Way outside.
        assert!(!b.contains(0, 0));
        assert!(!b.contains(1000, 1000));
    }
}
