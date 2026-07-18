use std::sync::Arc;

use anyhow::{Context, Result};
use russh_sftp::{client::SftpSession, protocol::FileAttributes};
use serde::{Deserialize, Serialize};

use crate::ssh::SshController;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub permissions: Option<String>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

pub struct SftpClient {
    session: Arc<SftpSession>,
    cwd: parking_lot::RwLock<String>,
}

impl SftpClient {
    pub async fn connect(ssh: Arc<SshController>) -> Result<Self> {
        let session = ssh.open_sftp().await?;
        let cwd = session
            .canonicalize(".")
            .await
            .context("resolve remote home directory")?;
        Ok(Self {
            session: Arc::new(session),
            cwd: parking_lot::RwLock::new(cwd),
        })
    }

    pub async fn list_dir(&self, path: Option<&str>) -> Result<Vec<RemoteFileEntry>> {
        let target = path.map(ToOwned::to_owned).unwrap_or_else(|| self.cwd());
        let canonical = self
            .session
            .canonicalize(target)
            .await
            .context("resolve remote directory")?;
        let mut entries = self
            .session
            .read_dir(canonical.clone())
            .await
            .context("list remote directory")?
            .map(remote_entry)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .is_dir
                .cmp(&left.is_dir)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub async fn stat(&self, path: &str) -> Result<RemoteFileEntry> {
        let metadata = self
            .session
            .symlink_metadata(path)
            .await
            .with_context(|| format!("stat remote path {path}"))?;
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string();
        Ok(entry_from_metadata(name, path.to_string(), metadata))
    }

    pub async fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.session
            .read(path)
            .await
            .with_context(|| format!("read remote file {path}"))
    }

    pub async fn write(&self, path: &str, data: &[u8]) -> Result<()> {
        self.session
            .create(path)
            .await
            .with_context(|| format!("create remote file {path}"))?
            .write_all(data)
            .await
            .with_context(|| format!("write remote file {path}"))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.session
            .rename(from, to)
            .await
            .with_context(|| format!("rename remote path {from} to {to}"))
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let metadata = self
            .session
            .symlink_metadata(path)
            .await
            .with_context(|| format!("stat remote path {path}"))?;
        if metadata.is_dir() {
            self.session
                .remove_dir(path)
                .await
                .with_context(|| format!("remove remote directory {path}"))
        } else {
            self.session
                .remove_file(path)
                .await
                .with_context(|| format!("remove remote file {path}"))
        }
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.session
            .create_dir(path)
            .await
            .with_context(|| format!("create remote directory {path}"))
    }

    pub async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        let metadata = FileAttributes {
            permissions: Some(mode),
            ..FileAttributes::default()
        };
        self.session
            .set_metadata(path, metadata)
            .await
            .with_context(|| format!("change remote permissions {path}"))
    }

    pub fn cwd(&self) -> String {
        self.cwd.read().clone()
    }

    pub async fn cd(&self, path: &str) -> Result<String> {
        let canonical = self
            .session
            .canonicalize(path)
            .await
            .with_context(|| format!("resolve remote directory {path}"))?;
        let metadata = self
            .session
            .metadata(canonical.clone())
            .await
            .with_context(|| format!("stat remote directory {canonical}"))?;
        anyhow::ensure!(
            metadata.is_dir(),
            "remote path is not a directory: {canonical}"
        );
        *self.cwd.write() = canonical.clone();
        Ok(canonical)
    }

    pub fn parent_path(&self) -> String {
        parent_remote_path(&self.cwd())
    }
}

fn remote_entry(entry: russh_sftp::client::fs::DirEntry) -> RemoteFileEntry {
    entry_from_metadata(entry.file_name(), entry.path(), entry.metadata())
}

fn entry_from_metadata(name: String, path: String, metadata: FileAttributes) -> RemoteFileEntry {
    RemoteFileEntry {
        name,
        path,
        is_dir: metadata.is_dir(),
        is_symlink: metadata.is_symlink(),
        size: metadata.size.unwrap_or(0),
        modified: metadata.mtime.map(u64::from),
        permissions: metadata
            .permissions
            .map(|mode| russh_sftp::protocol::FilePermissions::from(mode).to_string()),
        owner: metadata
            .user
            .or_else(|| metadata.uid.map(|uid| uid.to_string())),
        group: metadata
            .group
            .or_else(|| metadata.gid.map(|gid| gid.to_string())),
    }
}

fn parent_remote_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
    }
}

use tokio::io::AsyncWriteExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_path_stays_at_root() {
        assert_eq!(parent_remote_path("/"), "/");
        assert_eq!(parent_remote_path("/tmp"), "/");
        assert_eq!(parent_remote_path("/tmp/data"), "/tmp");
    }

    #[test]
    fn metadata_maps_to_public_entry() {
        let metadata = FileAttributes {
            size: Some(42),
            uid: Some(1000),
            gid: Some(100),
            permissions: Some(0o100644),
            mtime: Some(12),
            ..FileAttributes::default()
        };
        let entry = entry_from_metadata("a.txt".into(), "/a.txt".into(), metadata);
        assert_eq!(entry.size, 42);
        assert_eq!(entry.permissions.as_deref(), Some("rw-r--r--"));
        assert!(!entry.is_dir);
    }
}
