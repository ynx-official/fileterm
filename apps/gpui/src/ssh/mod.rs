//! SSH session controller and terminal integration.
//!
//! [`controller::SshController`] owns the authenticated SSH transport, shell
//! channel, exec/SFTP channels, and session-scoped port forwarding. Terminal
//! output is published through `broadcast::Sender<TermChunk>` so GPUI views do
//! not depend on a framework event bus. [`system_sidebar`] collects remote
//! metrics and [`terminal_dock`] provides command composition/history.

pub mod controller;
pub mod system_sidebar;
pub mod terminal_dock;
pub mod tunnel;

pub use controller::SshController;
pub use tunnel::{SshTunnelRule, SshTunnelSnapshot};
