//! Error types for the GPUI runtime.
//!
//! G0 phase of `docs/plans/active/gpui-refactor.md` section 6.1.
//!
//! Unlike Tauri's `AppError` (which needs `serde::Serialize` because it
//! crosses the IPC boundary to the WebView), the GPUI runtime is a single
//! process: bridge fns return `Result<T, AppError>` directly to the view
//! layer, no serialization. So this enum is a plain `thiserror::Error`
//! without the `serde::Serialize` impl.

use thiserror::Error;

/// Top-level error type returned by every `FileTermDesktopApi` method.
///
/// Variants are coarse-grained on purpose: G0's `GpuiDesktopApi` stub
/// returns `AppError::Unsupported` for *everything*; as real backends
/// land in G1–G5, methods will start returning `Storage` / `Window` /
/// `Command` / etc. variants with concrete string context. We keep the
/// Tauri-side variant names so the migration is line-for-line where
/// possible, and add `Unsupported` for the stub-only "not wired yet"
/// case.
#[derive(Debug, Error)]
pub enum AppError {
    /// Returned by the G0 `GpuiDesktopApi` stub for every method, and by
    /// later phases for any method that hasn't been wired up yet. Once a
    /// method gets a real backend, it must stop returning this variant —
    /// `Unsupported` is a migration placeholder, not a runtime fallback.
    #[error("unsupported: {0} (not yet wired up in GPUI runtime)")]
    Unsupported(&'static str),

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
