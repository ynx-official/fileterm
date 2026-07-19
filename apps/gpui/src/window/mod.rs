//! Native window coordination.
//!
//! The registry tracks unique tab ownership across the main and detached
//! windows. Detach and drag modules classify placement changes; root views own
//! the actual GPUI window creation because it requires foreground `App` access.
//! Menu and tray modules translate native actions back into that same view-level
//! coordinator.

pub mod detach;
pub mod kind;
pub mod menu;
pub mod registry;
pub mod tab_drag;
pub mod tray;

pub use detach::{detach_tab_to_new_window, return_window_to_main, DetachResult};
pub use kind::WindowKind;
pub use registry::{SharedWindowRegistry, WindowRegistry, WorkspaceTabPlacement};
pub use tab_drag::{DragDropTarget, ScreenBounds, TabDragState};
