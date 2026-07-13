use std::collections::HashMap;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use tokio::sync::{oneshot, RwLock};
use crate::sessions::WorkerCmd;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTab {
    pub id: String,
    pub profile_id: String,
    pub session_type: String,
    pub title: String,
    pub layout: String, // "terminal-file" | "file-only" | "terminal-only"
    pub status: String, // "connecting" | "connected" | "disconnected"
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub profile_id: String,
    pub access_host: String,
    pub summary: String,
    pub terminal_transcript: String,
    pub remote_path: String,
    pub shell_cwd: Option<String>,
    pub follow_shell_cwd: bool,
    pub remote_files: Vec<serde_json::Value>,
    pub file_access_mode: String, // "user" | "root"
    pub sudo_user: Option<String>,
    pub has_reusable_sudo_auth: bool,
    pub connected: bool,
    pub system_metrics: Option<serde_json::Value>,
}

pub struct WorkspaceState {
    pub tabs: Arc<RwLock<Vec<WorkspaceTab>>>,
    pub active_tab_id: Arc<RwLock<Option<String>>>,
    pub sessions: Arc<RwLock<HashMap<String, SessionSnapshot>>>,
    pub workers: Arc<RwLock<HashMap<String, tokio::sync::mpsc::Sender<WorkerCmd>>>>,
    /// Pending SSH interaction requests (host-key verification, MFA prompts).
    /// The renderer resolves each one via `app_resolve_ssh_interaction`.
    pub pending_interactions:
        Arc<RwLock<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tabs: Arc::new(RwLock::new(Vec::new())),
            active_tab_id: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            workers: Arc::new(RwLock::new(HashMap::new())),
            pending_interactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}
