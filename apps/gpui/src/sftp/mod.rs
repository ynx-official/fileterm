//! SFTP client + file manager + transfer service.
//!
//! G4 phase of `docs/plans/active/gpui-refactor.md` section 6.5.
//!
//! ## What G4 delivers
//!
//! * [`client::SftpClient`] — wraps `russh-sftp` client, owns the SFTP
//!   channel over an existing SSH session. Provides list/read/write/
//!   rename/delete/chmod/stat operations.
//! * [`file_manager`] — `FileManager` + `FileTable` (virtual scrolling)
//!   + `FileContextMenu`. Renders the remote directory listing.
//! * [`transfer`] — `TransferService` (journal + resume + pause) +
//!   `TransferCenter` + `TransferPopover`. Manages upload/download
//!   queues with progress + cancellation.
//!
//! ## What G4 does NOT deliver
//!
//! * Local file picker UI — uses `rfd` (already a dep from G1
//!   `local_files`) for the native file dialog.
//! * Conflict resolution policy — v1 always overwrites; v2 adds
//!   skip/rename/compare-size prompts (tracked as G4.5).

pub mod client;
pub mod file_manager;
pub mod transfer;

pub use client::SftpClient;
pub use transfer::{TransferTask, TransferTaskId, TransferTaskStatus};
