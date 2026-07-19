//! SFTP client, file manager, and durable transfer service.
//!
//! The authenticated SSH session owns the SFTP subsystem. File operations,
//! conflict-aware text editing, native local-file selection, resumable transfer
//! journals, pause/cancel, and overwrite confirmation remain session-scoped.

pub mod client;
pub mod file_manager;
pub mod transfer;

pub use client::SftpClient;
pub use transfer::{TransferTask, TransferTaskId, TransferTaskStatus};
