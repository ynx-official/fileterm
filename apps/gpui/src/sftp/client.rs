//! SFTP client — wraps `russh-sftp` over an existing SSH session.
//!
//! G4 phase of `docs/plans/active/gpui-refactor.md` section 6.5.
//!
//! Owns the SFTP channel and exposes typed file operations. The channel
//! is opened over a `russh::client::Handle` (from `SshController`); this
//! client takes a weak ref to the handle so dropping the SSH session
//! invalidates the SFTP client cleanly.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// One remote directory entry.
///
/// Mirrors Tauri's `RemoteFileEntry` shape line-for-line so the
/// renderer-side table renders identically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>, // Unix timestamp seconds.
    pub permissions: Option<String>, // e.g. "rwxr-xr-x".
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// SFTP client handle.
///
/// G4 stub — the real russh-sftp integration (channel open over
/// `SshController`'s handle, `SftpSession::list_dir` etc.) lands in G4.1.
/// The struct is here so `FileManager` and `TransferService` can hold
/// a handle to it today.
#[allow(dead_code)]
#[derive(Debug)]
pub struct SftpClient {
    /// Remote path the client is currently "cd"'d into. Tracked here so
    /// `list_dir()` without an arg lists the current dir.
    cwd: PathBuf,
}

impl SftpClient {
    /// Open an SFTP channel over an existing SSH session.
    ///
    /// G4 stub — real impl:
    /// ```ignore
    /// let channel = ssh_handle.channel_open_session().await?;
    /// channel.request_subsystem(true, "sftp").await?;
    /// let sftp = russh_sftp::client::SftpSession::new(channel.into_stream()).await?;
    /// ```
    pub async fn connect() -> Result<Self> {
        Err(anyhow::anyhow!("G4 stub: SFTP connect not yet implemented"))
    }

    /// List directory entries at `path`. If `path` is `None`, lists `cwd`.
    ///
    /// G4 stub — returns empty.
    pub async fn list_dir(&self, _path: Option<&str>) -> Result<Vec<RemoteFileEntry>> {
        Err(anyhow::anyhow!("G4 stub: list_dir not yet implemented"))
    }

    /// Stat a remote path (size, mtime, permissions).
    ///
    /// G4 stub.
    pub async fn stat(&self, _path: &str) -> Result<RemoteFileEntry> {
        Err(anyhow::anyhow!("G4 stub: stat not yet implemented"))
    }

    /// Open a remote file for reading. Returns a stream-like handle.
    ///
    /// G4 stub.
    pub async fn open_read(&self, _path: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: open_read not yet implemented"))
    }

    /// Open a remote file for writing (creates / truncates).
    ///
    /// G4 stub.
    pub async fn open_write(&self, _path: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: open_write not yet implemented"))
    }

    /// Rename a remote file or directory.
    ///
    /// G4 stub.
    pub async fn rename(&self, _from: &str, _to: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: rename not yet implemented"))
    }

    /// Delete a remote file.
    ///
    /// G4 stub.
    pub async fn delete(&self, _path: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: delete not yet implemented"))
    }

    /// Create a remote directory.
    ///
    /// G4 stub.
    pub async fn mkdir(&self, _path: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: mkdir not yet implemented"))
    }

    /// Change permissions on a remote path.
    ///
    /// G4 stub.
    pub async fn chmod(&self, _path: &str, _mode: u32) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: chmod not yet implemented"))
    }

    /// Get the current working directory.
    pub fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    /// Change the current working directory. G4 stub — real impl uses
    /// `realpath` to verify the path exists before updating `cwd`.
    pub async fn cd(&mut self, _path: &str) -> Result<()> {
        Err(anyhow::anyhow!("G4 stub: cd not yet implemented"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_returns_stub_error() {
        let result = SftpClient::connect().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("G4 stub"));
    }
}
