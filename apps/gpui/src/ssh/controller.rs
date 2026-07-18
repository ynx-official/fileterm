//! SSH session controller.
//!
//! G3 phase of `docs/plans/active/gpui-refactor.md` section 6.4.
//!
//! Wraps a `russh` client session and bridges SSH shell output to a
//! `broadcast::Sender<TermChunk>` (consumed by `TermView`'s feed pump,
//! same as the local PTY path). Input from `TermView` is written to the
//! SSH channel's stdin.
//!
//! ## Why a controller (not direct russh calls in the view)
//!
//! Same reason as `PtyHandle` — isolates the protocol library from the
//! view layer so:
//! 1. `TermView` doesn't know whether it's talking to a local PTY or a
//!    remote SSH shell (both produce `TermChunk` streams).
//! 2. The controller owns the `russh::client::Handle` lifetime and can
//!    cleanly disconnect + reap on drop.
//! 3. Reconnect logic (auth retry, channel re-open after network blip)
//!    lives here, not in the view.

use anyhow::{Context, Result};
use tokio::sync::{broadcast, mpsc};

use crate::term::TermChunk;

/// Configuration for opening an SSH session.
///
/// Mirrors the connection profile fields the user fills in the
/// connection form. `password` and `private_key` are mutually exclusive
/// in v1 (form enforces one or the other); both `Some` is a validation
/// error.
#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    /// PEM-encoded private key. The controller passes this to
    /// `russh::keys::PrivateKey::from_openssh` for parsing.
    pub private_key: Option<String>,
    /// Initial terminal grid size. The controller sends a `stty rows N
    /// cols M` after the shell opens so the remote pty matches the
    /// local view.
    pub cols: u16,
    pub rows: u16,
}

/// Handle to a running SSH shell session.
///
/// Drop to disconnect. The controller spawns a background task that
/// holds the `russh::client::Handle` and the shell channel; when this
/// handle drops, the task is cancelled and the session closes.
///
/// The output stream is a `broadcast::Sender<TermChunk>` — same shape
/// as `PtyHandle`'s, so `TermView` can subscribe identically.
#[derive(Debug)]
pub struct SshController {
    /// Sender for the output broadcast. Subscribed to by `TermView`.
    /// Kept here so the controller can be cloned to give out new
    /// receivers (e.g. for a second view on the same session).
    tx: broadcast::Sender<TermChunk>,
    /// Input channel. `write_input` sends bytes here; the background
    /// task drains them into the SSH channel's stdin.
    input_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Join handle for the background session task. Aborted on drop.
    _task: tokio::task::JoinHandle<()>,
}

impl SshController {
    /// Open a new SSH session.
    ///
    /// Returns the controller plus a receiver for the first subscriber
    /// (same pattern as `PtyHandle::spawn`).
    ///
    /// G3 stub: this method signature is final, but the body is a
    /// placeholder that returns `Err` immediately. The real russh
    /// integration (client connect → auth → channel open → shell req →
    /// pump loop) lands in G3.1.
    pub async fn connect(_config: SshConfig) -> Result<(Self, broadcast::Receiver<TermChunk>)> {
        // G3.1 TODO:
        // 1. let config = Arc::new(russh::client::Config::default());
        // 2. let handler = ClientHandler { ... };
        // 3. let mut handle = russh::client::connect(config, (host, port), handler).await?;
        // 4. authenticate (password or private_key)
        // 5. let mut channel = handle.channel_open_session().await?;
        // 6. channel.request_pty(false, "xterm-256color", cols, rows, 0, 0, []).await?;
        // 7. channel.request_shell(true).await?;
        // 8. spawn pump task: loop { select! { channel data → broadcast, input_rx → channel } }
        Err(anyhow::anyhow!("G3 stub: SSH connect not yet implemented"))
    }

    /// Subscribe to the output stream. Each subscriber independently
    /// tracks its position; a slow subscriber going `Lagged` doesn't
    /// affect others (same semantics as `PtyHandle::subscribe`).
    pub fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.tx.subscribe()
    }

    /// Write user input (keystrokes, paste) to the SSH channel's stdin.
    ///
    /// Non-blocking: enqueues onto `input_tx` and returns immediately.
    /// The background task drains the queue and writes to the channel.
    /// Returns `Err` only if the session has closed (input_tx dropped).
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.input_tx
            .send(bytes.to_vec())
            .context("SSH session closed (input channel dropped)")
    }

    /// Resize the remote pty. Sends `window-change` channel request.
    ///
    /// G3 stub — real impl calls `channel.window_change(cols, rows, 0, 0)`.
    pub fn resize(&self, _cols: u16, _rows: u16) -> Result<()> {
        // G3.1 TODO: send window-change request via the channel handle.
        Ok(())
    }
}

impl Drop for SshController {
    fn drop(&mut self) {
        // Aborting the task closes the SSH channel + disconnects the
        // client. The broadcast sender drops here too, so subscribers
        // see `RecvError::Closed`.
        self._task.abort();
    }
}

/// Internal russh client handler (auth callbacks).
///
/// G3 stub — real impl implements `russh::client::Handler` with
/// `check_server_key` (return Ok to accept any key in spike; real
/// known-hosts check lands in G3.2).
#[allow(dead_code)]
struct ClientHandler {
    username: String,
    password: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_returns_unimplemented_stub() {
        let config = SshConfig {
            host: "localhost".into(),
            port: 22,
            username: "user".into(),
            password: Some("pass".into()),
            private_key: None,
            cols: 80,
            rows: 24,
        };
        let result = SshController::connect(config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("G3 stub"));
    }
}
