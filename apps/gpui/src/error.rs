//! Error types shared by the GPUI bridge and runtime services.
//!
//! GPUI runs the bridge and backend in one process, so errors are returned
//! directly without the serialization layer required by a WebView IPC boundary.

use thiserror::Error;

/// Top-level error type returned by GPUI bridge methods and runtime services.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("clipboard error: {0}")]
    Clipboard(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("window error: {0}")]
    Window(String),

    #[error("SSH host verification required for {host}:{port}: {fingerprint}")]
    SshHostVerification {
        host: String,
        port: u16,
        fingerprint: String,
        changed: bool,
    },

    #[error("SSH authentication input required")]
    SshAuthenticationRequired {
        prompts: Vec<SshAuthenticationPrompt>,
    },

    #[error("command error: {0}")]
    Command(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshAuthenticationPromptKind {
    Password,
    PrivateKeyPassphrase,
    KeyboardInteractive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshAuthenticationPrompt {
    pub kind: SshAuthenticationPromptKind,
    pub label: String,
    pub echo: bool,
}

/// Convenience alias so bridge signatures read as `Result<T>`.
pub type Result<T> = std::result::Result<T, AppError>;
