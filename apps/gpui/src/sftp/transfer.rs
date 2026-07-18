//! Transfer service — upload/download queue with journal + resume + pause.
//!
//! G4 phase of `docs/plans/active/gpui-refactor.md` section 6.5.
//!
//! Mirrors `apps/tauri/src-tauri/src/services/transfers.rs` line-for-line
//! on the type surface (`TransferTask`, `TransferManifest`, journal file
//! format) so on-disk state is interchangeable between the Tauri and
//! GPUI runtimes. All operations are G4 stubs — the real russh-sftp
//! I/O loop + journal write loop lands in G4.3.
//!
//! ## Journal format
//!
//! `transfer-journal.json` next to the profiles directory:
//!
//! ```json
//! { "version": 1, "transfers": [ { "id": "...", "direction": "upload", ... } ] }
//! ```
//!
//! On startup, `TransferService::load` reads the journal and marks every
//! task with `status == "running"` as `paused` so the user can explicitly
//! resume or cancel. On every state transition the journal is rewritten
//! atomically (write to `.tmp` + rename).
//!
//! ## Resume + pause
//!
//! Each task carries a `partial_path` (e.g. `/tmp/foo.fileterm-part`).
//! On resume, the I/O loop stats the partial file and seeks both the
//! source and destination to that offset before continuing. `pause`
//! cancels the in-flight I/O `CancellationToken` but keeps the partial
//! file + journal entry so resume can pick up.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// Transfer direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Upload,
    Download,
}

/// Stable identifier for a transfer task. Same shape as Tauri (UUID v4
/// string) so journal entries interoperate.
pub type TransferTaskId = String;

/// Lifecycle state of a transfer task. Mirrors Tauri's status strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferTaskStatus {
    Queued,
    Running,
    Paused,
    Verifying,
    Finalizing,
    Done,
    Failed,
    Canceled,
}

impl TransferTaskStatus {
    /// Whether the task is still active (will make progress).
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Verifying | Self::Finalizing
        )
    }

    /// Whether the task has reached a terminal state.
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Canceled)
    }
}

/// Identity of a transfer file (size + mtime), used for resume verification.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferFileIdentity {
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u64>,
}

/// One entry in a transfer manifest (one file in a directory-tree transfer).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferManifestEntry {
    pub relative_path: String,
    pub source_path: String,
    pub destination_path: String,
    pub partial_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staging_path: Option<String>,
    pub source_identity: TransferFileIdentity,
    pub status: String,
    pub transferred_bytes: u64,
}

/// Manifest for a directory-tree transfer (multiple files + dirs).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferManifest {
    pub version: u8,
    pub directories: Vec<String>,
    pub files: Vec<TransferManifestEntry>,
}

/// A single transfer task. Mirrors `apps/tauri/src-tauri/src/services/transfers.rs::TransferTask`
/// field-for-field so the JSON journal is interchangeable.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: TransferTaskId,
    pub direction: TransferDirection,
    pub name: String,
    pub progress: f64,
    pub status: TransferTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transferred_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_access_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staging_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<TransferFileIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<TransferManifest>,
    #[serde(default)]
    pub resumable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_attempt: Option<u32>,
    #[serde(default)]
    pub cleanup_pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

/// On-disk journal shape.
#[allow(dead_code)]
#[derive(Deserialize, Serialize)]
struct TransferJournal {
    version: u8,
    transfers: Vec<TransferTask>,
}

const JOURNAL_VERSION: u8 = 1;
const PARTIAL_SUFFIX: &str = ".fileterm-part";

/// Transfer service — owns the journal + the in-memory task list.
///
/// G4 stub — the real implementation (tokio task per active transfer,
/// `CancellationToken` for pause/cancel, atomic journal rewrite, partial
/// file stat-on-resume) lands in G4.3. The struct + types are here so
/// `TransferCenter` (the GPUI view) and `commands::workspace_*` can
/// hold a handle today.
#[allow(dead_code)]
pub struct TransferService {
    /// Path to `transfer-journal.json` (under `app_data_dir()`).
    journal_path: PathBuf,
    /// In-memory copy of the journal. The source of truth is on-disk;
    /// this is updated + flushed atomically on every state transition.
    tasks: Vec<TransferTask>,
}

impl TransferService {
    /// Create a new service backed by `journal_path`. Does not load —
    /// call `load` to read the existing journal.
    pub fn new(journal_path: PathBuf) -> Self {
        Self {
            journal_path,
            tasks: Vec::new(),
        }
    }

    /// Load the journal from disk. Tasks marked `Running` are downgraded
    /// to `Paused` so the user explicitly resumes or cancels.
    ///
    /// G4 stub — returns an empty task list.
    pub fn load(&mut self) -> Result<()> {
        // G4.3 TODO: read journal_path, deserialize, downgrade Running → Paused.
        self.tasks.clear();
        Ok(())
    }

    /// Flush the in-memory task list to disk atomically.
    ///
    /// G4 stub.
    pub fn flush(&self) -> Result<()> {
        // G4.3 TODO: serialize to JSON, write to {journal_path}.tmp, rename.
        Ok(())
    }

    /// Enqueue a new transfer task. Returns the new task id.
    ///
    /// G4 stub — returns `AppError::Unsupported`.
    pub fn enqueue(
        &mut self,
        _direction: TransferDirection,
        _source_path: &str,
        _destination_path: &str,
        _tab_id: Option<&str>,
    ) -> Result<TransferTaskId> {
        Err(AppError::Unsupported(
            "G4: TransferService::enqueue not yet wired up",
        ))
    }

    /// Pause a running task. Keeps the partial file + journal entry.
    ///
    /// G4 stub.
    pub fn pause(&mut self, _id: &str) -> Result<()> {
        Err(AppError::Unsupported(
            "G4: TransferService::pause not yet wired up",
        ))
    }

    /// Resume a paused task. Stats the partial file and seeks both ends
    /// to that offset before continuing the I/O loop.
    ///
    /// G4 stub.
    pub fn resume(&mut self, _id: &str) -> Result<()> {
        Err(AppError::Unsupported(
            "G4: TransferService::resume not yet wired up",
        ))
    }

    /// Cancel a task. Deletes the partial file + removes the journal entry.
    ///
    /// G4 stub.
    pub fn cancel(&mut self, _id: &str) -> Result<()> {
        Err(AppError::Unsupported(
            "G4: TransferService::cancel not yet wired up",
        ))
    }

    /// Snapshot all tasks (for `TransferCenter` view rendering).
    pub fn list(&self) -> &[TransferTask] {
        &self.tasks
    }

    /// Look up a single task by id.
    pub fn get(&self, id: &str) -> Option<&TransferTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Path to the journal file (exposed for tests + diagnostics).
    pub fn journal_path(&self) -> &std::path::Path {
        &self.journal_path
    }

    /// Partial-file suffix (exposed for tests; same as Tauri side).
    pub fn partial_suffix() -> &'static str {
        PARTIAL_SUFFIX
    }

    /// Journal format version (exposed for tests; same as Tauri side).
    pub fn journal_version() -> u8 {
        JOURNAL_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_active_and_terminal_partitions() {
        let active = [
            TransferTaskStatus::Queued,
            TransferTaskStatus::Running,
            TransferTaskStatus::Verifying,
            TransferTaskStatus::Finalizing,
        ];
        let terminal = [
            TransferTaskStatus::Done,
            TransferTaskStatus::Failed,
            TransferTaskStatus::Canceled,
        ];
        for s in active {
            assert!(s.active(), "{:?} should be active", s);
            assert!(!s.terminal(), "{:?} should not be terminal", s);
        }
        for s in terminal {
            assert!(!s.active(), "{:?} should not be active", s);
            assert!(s.terminal(), "{:?} should be terminal", s);
        }
        // Paused is the odd one out: not active (won't make progress on
        // its own) but not terminal either (can be resumed).
        assert!(!TransferTaskStatus::Paused.active());
        assert!(!TransferTaskStatus::Paused.terminal());
    }

    #[test]
    fn service_load_returns_empty_stub() {
        let mut svc = TransferService::new(PathBuf::from("/tmp/nonexistent-journal.json"));
        // G4 stub: load always returns Ok with an empty task list.
        svc.load().expect("stub load should not error");
        assert!(svc.list().is_empty());
    }

    #[test]
    fn service_enqueue_returns_unsupported() {
        let mut svc = TransferService::new(PathBuf::from("/tmp/nonexistent-journal.json"));
        let result = svc.enqueue(
            TransferDirection::Upload,
            "/local/path",
            "/remote/path",
            Some("tab-1"),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, AppError::Unsupported(msg) if msg.contains("enqueue")),
            "expected Unsupported(enqueue), got {:?}",
            err
        );
    }

    #[test]
    fn service_pause_resume_cancel_return_unsupported() {
        let mut svc = TransferService::new(PathBuf::from("/tmp/nonexistent-journal.json"));
        for op in ["pause", "resume", "cancel"] {
            let result = match op {
                "pause" => svc.pause("any"),
                "resume" => svc.resume("any"),
                "cancel" => svc.cancel("any"),
                _ => unreachable!(),
            };
            assert!(result.is_err(), "{} should error", op);
            let err = result.unwrap_err();
            assert!(
                matches!(err, AppError::Unsupported(msg) if msg.contains(op)),
                "expected Unsupported({}), got {:?}",
                op,
                err
            );
        }
    }

    #[test]
    fn partial_suffix_and_journal_version_match_tauri() {
        // These constants are part of the on-disk contract with the Tauri
        // runtime — changing them breaks journal interop. Lock them down.
        assert_eq!(TransferService::partial_suffix(), ".fileterm-part");
        assert_eq!(TransferService::journal_version(), 1);
    }
}
