pub mod connections;
pub mod logging;
pub mod profile_ops;
pub mod ssh_keys;
pub mod transfers;
pub mod updates;
pub mod webdav;
pub mod workspace;
pub mod workspace_window_placement;
pub mod workspace_window_registry;

pub use workspace::{SessionSnapshot, WorkspaceState, WorkspaceTab, WorkspaceTabStatus};
pub use workspace_window_registry::{
    WorkspaceWindowContext, WorkspaceWindowKind, WorkspaceWindowRegistry, WorkspaceTabPlacement,
};
