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

    pub fn load(&mut self) -> Result<()> {
        if !self.journal_path.exists() {
            self.tasks.clear();
            return Ok(());
        }
        let bytes = std::fs::read(&self.journal_path)
            .map_err(|error| AppError::Storage(format!("read transfer journal: {error}")))?;
        let mut journal: TransferJournal = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::Serialization(format!("parse transfer journal: {error}")))?;
        if journal.version != JOURNAL_VERSION {
            return Err(AppError::Storage(format!(
                "unsupported transfer journal version: {}",
                journal.version
            )));
        }
        for task in &mut journal.transfers {
            if task.status.active() {
                task.status = TransferTaskStatus::Paused;
                task.message = Some("应用重启后等待恢复".to_string());
                task.updated_at = Some(now_secs());
            }
        }
        self.tasks = journal.transfers;
        self.flush()
    }

    pub fn flush(&self) -> Result<()> {
        if let Some(parent) = self.journal_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::Storage(format!("create transfer journal directory: {error}"))
            })?;
        }
        let journal = TransferJournal {
            version: JOURNAL_VERSION,
            transfers: self.tasks.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&journal).map_err(|error| {
            AppError::Serialization(format!("serialize transfer journal: {error}"))
        })?;
        let temporary = self.journal_path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .map_err(|error| AppError::Storage(format!("write transfer journal: {error}")))?;
        std::fs::rename(&temporary, &self.journal_path)
            .map_err(|error| AppError::Storage(format!("replace transfer journal: {error}")))
    }

    pub fn enqueue(
        &mut self,
        direction: TransferDirection,
        source_path: &str,
        destination_path: &str,
        tab_id: Option<&str>,
    ) -> Result<TransferTaskId> {
        if source_path.trim().is_empty() || destination_path.trim().is_empty() {
            return Err(AppError::Command(
                "transfer source and destination are required".to_string(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let name = source_path
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or(source_path)
            .to_string();
        let now = now_secs();
        let partial_path = format!("{destination_path}{PARTIAL_SUFFIX}");
        self.tasks.push(TransferTask {
            id: id.clone(),
            direction,
            name,
            progress: 0.0,
            status: TransferTaskStatus::Queued,
            message: None,
            speed: None,
            transferred_bytes: Some(0),
            total_bytes: None,
            tab_id: tab_id.map(ToOwned::to_owned),
            profile_id: None,
            session_type: Some("ssh".to_string()),
            file_access_mode: Some("sftp".to_string()),
            target_type: Some("file".to_string()),
            source_path: Some(source_path.to_string()),
            destination_path: Some(destination_path.to_string()),
            partial_path: Some(partial_path),
            staging_path: None,
            source_identity: None,
            manifest: None,
            resumable: true,
            retry_attempt: Some(0),
            cleanup_pending: false,
            created_at: Some(now),
            updated_at: Some(now),
        });
        self.flush()?;
        Ok(id)
    }

    pub fn pause(&mut self, id: &str) -> Result<()> {
        let task = self.task_mut(id)?;
        if !matches!(
            task.status,
            TransferTaskStatus::Queued
                | TransferTaskStatus::Running
                | TransferTaskStatus::Verifying
                | TransferTaskStatus::Finalizing
        ) {
            return Err(AppError::Command(format!(
                "transfer cannot be paused from {:?}",
                task.status
            )));
        }
        task.status = TransferTaskStatus::Paused;
        task.message = Some("已暂停".to_string());
        task.updated_at = Some(now_secs());
        self.flush()
    }

    pub fn resume(&mut self, id: &str) -> Result<()> {
        let task = self.task_mut(id)?;
        if !matches!(
            task.status,
            TransferTaskStatus::Paused | TransferTaskStatus::Failed
        ) {
            return Err(AppError::Command(format!(
                "transfer cannot be resumed from {:?}",
                task.status
            )));
        }
        task.status = TransferTaskStatus::Queued;
        task.message = None;
        task.retry_attempt = Some(task.retry_attempt.unwrap_or(0).saturating_add(1));
        task.updated_at = Some(now_secs());
        self.flush()
    }

    pub fn cancel(&mut self, id: &str) -> Result<()> {
        let task = self.task_mut(id)?;
        if task.status.terminal() {
            return Err(AppError::Command(format!(
                "transfer is already terminal: {:?}",
                task.status
            )));
        }
        task.status = TransferTaskStatus::Canceled;
        task.message = Some("已取消".to_string());
        task.updated_at = Some(now_secs());
        self.flush()
    }

    pub fn discard(&mut self, id: &str) -> Result<()> {
        let Some(index) = self.tasks.iter().position(|task| task.id == id) else {
            return Err(AppError::Command(format!("transfer not found: {id}")));
        };
        if !self.tasks[index].status.terminal() {
            return Err(AppError::Command(
                "only terminal transfers can be discarded".to_string(),
            ));
        }
        self.tasks.remove(index);
        self.flush()
    }

    fn task_mut(&mut self, id: &str) -> Result<&mut TransferTask> {
        self.tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| AppError::Command(format!("transfer not found: {id}")))
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    fn temporary_journal() -> PathBuf {
        std::env::temp_dir()
            .join(format!("fileterm-transfer-{}", uuid::Uuid::new_v4()))
            .join("transfer-journal.json")
    }

    #[test]
    fn missing_journal_loads_empty() {
        let mut service = TransferService::new(temporary_journal());
        service.load().unwrap();
        assert!(service.list().is_empty());
    }

    #[test]
    fn enqueue_pause_resume_cancel_and_discard_are_persisted() {
        let journal = temporary_journal();
        let mut service = TransferService::new(journal.clone());
        let id = service
            .enqueue(
                TransferDirection::Upload,
                "/local/path.txt",
                "/remote/path.txt",
                Some("tab-1"),
            )
            .unwrap();
        assert_eq!(service.get(&id).unwrap().status, TransferTaskStatus::Queued);
        service.pause(&id).unwrap();
        assert_eq!(service.get(&id).unwrap().status, TransferTaskStatus::Paused);
        service.resume(&id).unwrap();
        assert_eq!(service.get(&id).unwrap().status, TransferTaskStatus::Queued);
        service.cancel(&id).unwrap();
        assert_eq!(
            service.get(&id).unwrap().status,
            TransferTaskStatus::Canceled
        );

        let mut restored = TransferService::new(journal);
        restored.load().unwrap();
        assert_eq!(
            restored.get(&id).unwrap().status,
            TransferTaskStatus::Canceled
        );
        restored.discard(&id).unwrap();
        assert!(restored.list().is_empty());
    }

    #[test]
    fn active_tasks_are_paused_after_restart() {
        let journal = temporary_journal();
        let mut service = TransferService::new(journal.clone());
        let id = service
            .enqueue(
                TransferDirection::Download,
                "/remote/path.txt",
                "/local/path.txt",
                Some("tab-1"),
            )
            .unwrap();
        service.tasks[0].status = TransferTaskStatus::Running;
        service.flush().unwrap();

        let mut restored = TransferService::new(journal);
        restored.load().unwrap();
        assert_eq!(
            restored.get(&id).unwrap().status,
            TransferTaskStatus::Paused
        );
        assert_eq!(
            restored.get(&id).unwrap().message.as_deref(),
            Some("应用重启后等待恢复")
        );
    }

    #[test]
    fn partial_suffix_and_journal_version_match_tauri() {
        // These constants are part of the on-disk contract with the Tauri
        // runtime — changing them breaks journal interop. Lock them down.
        assert_eq!(TransferService::partial_suffix(), ".fileterm-part");
        assert_eq!(TransferService::journal_version(), 1);
    }
}
