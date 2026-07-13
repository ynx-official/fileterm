pub mod ssh;
pub mod system_metrics;
pub mod local_files;

pub enum WorkerCmd {
    WriteTerminal(String),
    ResizeTerminal {
        cols: u32,
        rows: u32,
        width: u32,
        height: u32,
    },
    ListRemoteFiles {
        path: String,
        respond_to: tokio::sync::oneshot::Sender<Result<Vec<serde_json::Value>, String>>,
    },
    ReadRemoteFile {
        path: String,
        encoding: String,
        respond_to: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
    WriteRemoteFile {
        path: String,
        content: String,
        encoding: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    CreateRemoteDirectory {
        parent_path: String,
        name: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    CreateRemoteFile {
        parent_path: String,
        name: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    CopyRemotePath {
        target_path: String,
        destination_path: String,
        target_type: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    MoveRemotePath {
        target_path: String,
        destination_path: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    RenameRemotePath {
        target_path: String,
        new_name: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    DeleteRemotePath {
        target_path: String,
        target_type: String,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    ChangeRemotePermissions {
        target_path: String,
        permissions: u32,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    SetRemoteFileAccessMode {
        mode: String,
        sudo_user: Option<String>,
        sudo_password: Option<String>,
        respond_to: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Disconnect,
}
