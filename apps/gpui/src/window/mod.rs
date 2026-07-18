//! Window management for the GPUI runtime.
//!
//! G2 + G5 phases of `docs/plans/active/gpui-refactor.md` sections 6.3 + 6.6.
//!
//! ## What G2 delivers
//!
//! * [`WindowRegistry`] — tracks which tab is in which window, supports
//!   detaching tabs to new windows and returning them on close. Mirrors
//!   the Tauri `services/workspace_window_registry.rs` contract but uses
//!   GPUI's `WindowHandle` instead of `WebviewWindow`.
//! * [`kind`] — the 7 window kinds from refactor.md 4.2.1 (main,
//!   connection-manager, command-manager, connection-form, command-form,
//!   file-editor, detached-session) as a typed enum so the registry
//!   can't accidentally mix labels.
//! * [`tray`] — `TrayHandler` with platform branches (macOS / Windows /
//!   Linux). G2 ships the structure; real platform glue (native menu
//!   items, click → window show/hide) lands incrementally.
//! * [`menu`] — `build_application_menu` returns a GPUI menu matching
//!   Tauri's `init_menu` (File / Edit / View / Window / Help).
//!
//! ## What G5 adds
//!
//! * [`detach`] — pure-logic orchestration for "detach tab to new
//!   window" + "return window to main". Generates `detached-session-{uuid}`
//!   ids, updates the [`WindowRegistry`] bookkeeping, returns the
//!   placement list for broadcast. The actual GPUI window open is the
//!   caller's job (it needs `&mut App` + a root view).
//! * [`tab_drag`] — `TabDragState` tracks the in-flight cross-window
//!   tab drag and classifies the drop target (same window / other
//!   window / new window). Pure logic — GPUI's built-in `Drag` element
//!   is single-window, so cross-window drags are tracked manually.
//!
//! ## What G2/G5 do NOT deliver
//!
//! * Actual window opening — that's G3+ (each window kind needs its
//!   own root view). G2 only provides the registry + types.
//! * Tray icon rendering — needs platform-specific image assets; G2
//!   ships the handler structure with a default icon placeholder.

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
