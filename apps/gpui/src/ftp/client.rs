use std::{collections::HashMap, path::Path, sync::Arc, time::UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use suppaftp::{
    list::{File as ListedFile, ListParser},
    tokio::{
        AsyncFtpStream, AsyncNativeTlsConnector, AsyncNativeTlsFtpStream, ImplAsyncFtpStream,
        TokioTlsStream,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
};

use crate::sftp::{
    client::RemoteFileEntry,
    transfer::{TransferControl, TransferIoOutcome, TransferProgress},
};

#[derive(Clone, Debug)]
pub struct FtpProfile {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub security_mode: FtpSecurityMode,
    pub remote_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FtpSecurityMode {
    None,
    Explicit,
    Implicit,
}

impl FtpProfile {
    pub fn from_value(value: &Value) -> Result<Self> {
        let host = required_string(value, "host", "FTP host is required")?;
        let port = value.get("port").and_then(Value::as_u64).unwrap_or(21);
        if !(1..=65535).contains(&port) {
            bail!("FTP port is invalid");
        }
        let security_mode = match value
            .get("securityMode")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if value.get("secure").and_then(Value::as_bool) == Some(true) {
                    "explicit"
                } else {
                    "none"
                }
            }) {
            "none" => FtpSecurityMode::None,
            "explicit" => FtpSecurityMode::Explicit,
            "implicit" => FtpSecurityMode::Implicit,
            mode => bail!("unsupported FTP security mode: {mode}"),
        };
        Ok(Self {
            host,
            port: port as u16,
            username: value
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("anonymous")
                .to_string(),
            password: value
                .get("password")
                .and_then(Value::as_str)
                .unwrap_or("anonymous@")
                .to_string(),
            security_mode,
            remote_path: value
                .get("remotePath")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .unwrap_or("/")
                .to_string(),
        })
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn required_string(value: &Value, key: &str, message: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(message.to_string()))
}

enum FtpClient {
    Plain(AsyncFtpStream),
    Secure(AsyncNativeTlsFtpStream),
}

#[derive(Default)]
struct ListingState {
    mlsd_disabled: bool,
    mlst_disabled: bool,
    size_disabled: bool,
    resolved_types: HashMap<String, bool>,
    resolved_sizes: HashMap<String, usize>,
}

struct ParsedListing {
    entry: ListedFile,
    type_is_trusted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FtpTextFile {
    pub content: String,
    pub size: u64,
    pub sha256: String,
}

pub struct FtpSession {
    profile: FtpProfile,
    client: Mutex<Option<FtpClient>>,
    listing_state: Mutex<ListingState>,
    cwd: parking_lot::RwLock<String>,
}

impl FtpSession {
    pub async fn connect(profile: FtpProfile) -> Result<Arc<Self>> {
        let client = connect_client(&profile).await?;
        Ok(Arc::new(Self {
            cwd: parking_lot::RwLock::new(profile.remote_path.clone()),
            profile,
            client: Mutex::new(Some(client)),
            listing_state: Mutex::new(ListingState::default()),
        }))
    }

    pub fn endpoint(&self) -> String {
        self.profile.endpoint()
    }

    pub fn cwd(&self) -> String {
        self.cwd.read().clone()
    }

    pub fn parent_path(&self) -> String {
        parent_remote_path(&self.cwd())
    }

    pub async fn list_dir(&self, path: Option<&str>) -> Result<Vec<RemoteFileEntry>> {
        let path = path.map(ToOwned::to_owned).unwrap_or_else(|| self.cwd());
        let mut client = self.client.lock().await;
        let client = client
            .as_mut()
            .ok_or_else(|| anyhow!("FTP session is closed"))?;
        let mut state = self.listing_state.lock().await;
        let entries = client_list(client, &path, &mut state).await?;
        *self.cwd.write() = path;
        Ok(entries)
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let mut client = self.client.lock().await;
        client_mkdir(live_client(&mut client)?, path).await
    }

    pub async fn create_file(&self, path: &str) -> Result<()> {
        let mut client = self.client.lock().await;
        client_write(live_client(&mut client)?, path, &[]).await
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut client = self.client.lock().await;
        client_rename(live_client(&mut client)?, from, to).await
    }

    pub async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        let mut client = self.client.lock().await;
        client_chmod(live_client(&mut client)?, path, mode).await
    }

    pub async fn delete(&self, entry: &RemoteFileEntry) -> Result<()> {
        let mut client = self.client.lock().await;
        client_delete(live_client(&mut client)?, &entry.path, entry.is_dir).await
    }

    pub async fn remote_file_identity(&self, path: &str) -> Result<(u64, Option<u64>)> {
        let mut client = self.client.lock().await;
        Ok((client_size(live_client(&mut client)?, path).await?, None))
    }

    pub async fn read_text_file(&self, path: &str, max_bytes: u64) -> Result<FtpTextFile> {
        let mut client = self.client.lock().await;
        let client = live_client(&mut client)?;
        let size = client_size(client, path).await?;
        if size > max_bytes {
            bail!("file exceeds editor limit of {max_bytes} bytes");
        }
        let bytes = client_read(client, path, max_bytes).await?;
        let sha256 = sha256_hex(&bytes);
        let content =
            String::from_utf8(bytes).map_err(|_| anyhow!("file is not valid UTF-8 text"))?;
        Ok(FtpTextFile {
            content,
            size,
            sha256,
        })
    }

    pub async fn write_text_file_if_unchanged(
        &self,
        path: &str,
        expected_size: u64,
        expected_sha256: &str,
        content: &str,
        max_bytes: u64,
    ) -> Result<FtpTextFile> {
        let mut client = self.client.lock().await;
        let client = live_client(&mut client)?;
        let current_size = client_size(client, path).await?;
        if current_size != expected_size {
            bail!("remote file changed from {expected_size} to {current_size} bytes");
        }
        let current = client_read(client, path, max_bytes).await?;
        if sha256_hex(&current) != expected_sha256 {
            bail!("remote file content changed since it was opened");
        }
        let partial = format!("{path}.fileterm-edit-{}", uuid::Uuid::new_v4());
        if let Err(error) = client_write(client, &partial, content.as_bytes()).await {
            let _ = client_remove_if_exists(client, &partial).await;
            return Err(error);
        }
        if let Err(error) = client_replace(client, &partial, path, true).await {
            let _ = client_remove_if_exists(client, &partial).await;
            return Err(error);
        }
        let bytes = content.as_bytes();
        Ok(FtpTextFile {
            content: content.to_string(),
            size: client_size(client, path).await?,
            sha256: sha256_hex(bytes),
        })
    }

    pub async fn remote_file_size_if_exists(&self, path: &str) -> Result<u64> {
        let mut client = self.client.lock().await;
        match client_size(live_client(&mut client)?, path).await {
            Ok(size) => Ok(size),
            Err(error) if is_missing_message(&error.to_string()) => Ok(0),
            Err(error) => Err(error),
        }
    }

    pub async fn upload_file(
        &self,
        local_path: &Path,
        remote_partial_path: &str,
        control: TransferControl,
        progress: tokio::sync::watch::Sender<TransferProgress>,
    ) -> Result<TransferIoOutcome> {
        let mut client = connect_client(&self.profile).await?;
        let result = client_upload(
            &mut client,
            local_path,
            remote_partial_path,
            control,
            progress,
        )
        .await;
        let _ = client_quit(&mut client).await;
        result
    }

    pub async fn download_file(
        &self,
        remote_path: &str,
        local_partial_path: &Path,
        control: TransferControl,
        progress: tokio::sync::watch::Sender<TransferProgress>,
    ) -> Result<TransferIoOutcome> {
        let mut client = connect_client(&self.profile).await?;
        let result = client_download(
            &mut client,
            remote_path,
            local_partial_path,
            control,
            progress,
        )
        .await;
        let _ = client_quit(&mut client).await;
        result
    }

    pub async fn finalize_upload(
        &self,
        partial: &str,
        destination: &str,
        overwrite: bool,
    ) -> Result<()> {
        let mut client = self.client.lock().await;
        client_replace(live_client(&mut client)?, partial, destination, overwrite).await
    }

    pub async fn remove_remote_file_if_exists(&self, path: &str) -> Result<()> {
        let mut client = self.client.lock().await;
        client_remove_if_exists(live_client(&mut client)?, path).await
    }

    pub async fn close(&self) {
        let client = self.client.lock().await.take();
        if let Some(mut client) = client {
            let _ = client_quit(&mut client).await;
        }
    }
}

fn live_client(client: &mut Option<FtpClient>) -> Result<&mut FtpClient> {
    client
        .as_mut()
        .ok_or_else(|| anyhow!("FTP session is closed"))
}

async fn connect_client(profile: &FtpProfile) -> Result<FtpClient> {
    let address = (profile.host.as_str(), profile.port);
    match profile.security_mode {
        FtpSecurityMode::None => {
            let mut client = AsyncFtpStream::connect(address).await?;
            client.login(&profile.username, &profile.password).await?;
            Ok(FtpClient::Plain(client))
        }
        FtpSecurityMode::Explicit => {
            let connector =
                AsyncNativeTlsConnector::from(suppaftp::async_native_tls::TlsConnector::new());
            let client = AsyncNativeTlsFtpStream::connect(address).await?;
            let mut client = client.into_secure(connector, &profile.host).await?;
            client.login(&profile.username, &profile.password).await?;
            Ok(FtpClient::Secure(client))
        }
        FtpSecurityMode::Implicit => {
            let connector =
                AsyncNativeTlsConnector::from(suppaftp::async_native_tls::TlsConnector::new());
            let mut client =
                AsyncNativeTlsFtpStream::connect_secure_implicit(address, connector, &profile.host)
                    .await?;
            client.login(&profile.username, &profile.password).await?;
            Ok(FtpClient::Secure(client))
        }
    }
}

macro_rules! ftp_match {
    ($client:expr, $ftp:ident => $operation:expr) => {
        match $client {
            FtpClient::Plain($ftp) => $operation.await,
            FtpClient::Secure($ftp) => $operation.await,
        }
    };
}

async fn client_list(
    client: &mut FtpClient,
    path: &str,
    state: &mut ListingState,
) -> Result<Vec<RemoteFileEntry>> {
    ftp_match!(client, ftp => list_files(ftp, path, state))
}

async fn client_mkdir(client: &mut FtpClient, path: &str) -> Result<()> {
    ftp_match!(client, ftp => mkdir(ftp, path))
}

async fn client_write(client: &mut FtpClient, path: &str, bytes: &[u8]) -> Result<()> {
    ftp_match!(client, ftp => write_file(ftp, path, bytes))
}

async fn client_rename(client: &mut FtpClient, from: &str, to: &str) -> Result<()> {
    ftp_match!(client, ftp => rename(ftp, from, to))
}

async fn client_chmod(client: &mut FtpClient, path: &str, mode: u32) -> Result<()> {
    ftp_match!(client, ftp => chmod(ftp, path, mode))
}

async fn client_delete(client: &mut FtpClient, path: &str, is_dir: bool) -> Result<()> {
    ftp_match!(client, ftp => delete_path(ftp, path, is_dir))
}

async fn client_size(client: &mut FtpClient, path: &str) -> Result<u64> {
    ftp_match!(client, ftp => size(ftp, path))
}

async fn client_read(client: &mut FtpClient, path: &str, max_bytes: u64) -> Result<Vec<u8>> {
    ftp_match!(client, ftp => read_file(ftp, path, max_bytes))
}

async fn client_upload(
    client: &mut FtpClient,
    local: &Path,
    remote: &str,
    control: TransferControl,
    progress: tokio::sync::watch::Sender<TransferProgress>,
) -> Result<TransferIoOutcome> {
    ftp_match!(client, ftp => upload(ftp, local, remote, control, progress))
}

async fn client_download(
    client: &mut FtpClient,
    remote: &str,
    local: &Path,
    control: TransferControl,
    progress: tokio::sync::watch::Sender<TransferProgress>,
) -> Result<TransferIoOutcome> {
    ftp_match!(client, ftp => download(ftp, remote, local, control, progress))
}

async fn client_replace(
    client: &mut FtpClient,
    partial: &str,
    destination: &str,
    overwrite: bool,
) -> Result<()> {
    ftp_match!(client, ftp => replace(ftp, partial, destination, overwrite))
}

async fn client_remove_if_exists(client: &mut FtpClient, path: &str) -> Result<()> {
    ftp_match!(client, ftp => remove_if_exists(ftp, path))
}

async fn client_quit(client: &mut FtpClient) -> Result<()> {
    ftp_match!(client, ftp => quit(ftp))
}

async fn list_files<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    state: &mut ListingState,
) -> Result<Vec<RemoteFileEntry>> {
    let lines = if state.mlsd_disabled {
        ftp.list(Some(path)).await?
    } else {
        match ftp.mlsd(Some(path)).await {
            Ok(lines) if lines.iter().all(|line| looks_like_mlsd(line)) => lines,
            Ok(lines) => {
                state.mlsd_disabled = true;
                lines
            }
            Err(_) => {
                state.mlsd_disabled = true;
                ftp.list(Some(path)).await?
            }
        }
    };
    let mut entries = Vec::new();
    for line in lines {
        let Some(parsed) = parse_listing(&line) else {
            continue;
        };
        let entry = parsed.entry;
        let name = entry.name();
        if matches!(name, "." | "..") {
            continue;
        }
        let full_path = join_remote_path(path, name);
        let (is_dir, size) = if parsed.type_is_trusted {
            state
                .resolved_types
                .insert(full_path.clone(), entry.is_directory());
            (entry.is_directory(), entry.size())
        } else {
            let (is_dir, size) = resolve_type(ftp, &full_path, state).await;
            (is_dir, size.unwrap_or(entry.size()))
        };
        entries.push(RemoteFileEntry {
            name: name.to_string(),
            path: full_path,
            is_dir,
            is_symlink: entry.is_symlink(),
            size: size as u64,
            modified: entry
                .modified()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|value| value.as_secs()),
            permissions: listing_permission(&line),
            owner: entry.uid().map(|value| value.to_string()),
            group: entry.gid().map(|value| value.to_string()),
        });
    }
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

async fn resolve_type<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    state: &mut ListingState,
) -> (bool, Option<usize>) {
    if let Some(is_dir) = state.resolved_types.get(path).copied() {
        return (is_dir, state.resolved_sizes.get(path).copied());
    }
    if !state.mlst_disabled {
        match ftp.mlst(Some(path)).await {
            Ok(line) if looks_like_mlsd(&line) => {
                if let Ok(entry) = ListParser::parse_mlst(&line) {
                    state
                        .resolved_types
                        .insert(path.to_string(), entry.is_directory());
                    state.resolved_sizes.insert(path.to_string(), entry.size());
                    return (entry.is_directory(), Some(entry.size()));
                }
                state.mlst_disabled = true;
            }
            _ => state.mlst_disabled = true,
        }
    }
    if !state.size_disabled {
        match ftp.size(path).await {
            Ok(size) => {
                state.resolved_types.insert(path.to_string(), false);
                state.resolved_sizes.insert(path.to_string(), size);
                return (false, Some(size));
            }
            Err(error) if is_unsupported_command(&error.to_string()) => state.size_disabled = true,
            Err(_) => {}
        }
    }
    let previous = ftp.pwd().await.ok();
    let is_dir = ftp.cwd(path).await.is_ok();
    if is_dir {
        if let Some(previous) = previous {
            let _ = ftp.cwd(previous).await;
        }
    }
    state.resolved_types.insert(path.to_string(), is_dir);
    (is_dir, None)
}

async fn mkdir<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
) -> Result<()> {
    ftp.mkdir(path).await?;
    Ok(())
}

async fn ensure_dir<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
) -> Result<()> {
    let mut current = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        let _ = ftp.mkdir(&current).await;
    }
    Ok(())
}

async fn write_file<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    bytes: &[u8],
) -> Result<()> {
    ensure_dir(ftp, &parent_remote_path(path)).await?;
    let mut stream = ftp.put_with_stream(path).await?;
    stream.write_all(bytes).await?;
    ftp.finalize_put_stream(stream).await?;
    Ok(())
}

async fn rename<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    from: &str,
    to: &str,
) -> Result<()> {
    ftp.rename(from, to).await?;
    Ok(())
}

async fn chmod<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    mode: u32,
) -> Result<()> {
    ftp.site(format!("CHMOD {:o} {path}", mode & 0o7777))
        .await?;
    Ok(())
}

async fn delete_path<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    is_dir: bool,
) -> Result<()> {
    if !is_dir {
        ftp.rm(path).await?;
        return Ok(());
    }
    let mut state = ListingState::default();
    for child in list_files(ftp, path, &mut state).await? {
        Box::pin(delete_path(ftp, &child.path, child.is_dir)).await?;
    }
    ftp.rmdir(path).await?;
    Ok(())
}

async fn size<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
) -> Result<u64> {
    Ok(ftp.size(path).await? as u64)
}

async fn read_file<T: TokioTlsStream + Send + 'static>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let mut stream = ftp.retr_as_stream(path).await?;
    let mut bytes = Vec::new();
    let read_result = (&mut stream)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .await;
    if let Err(error) = read_result {
        let _ = ftp.abort(stream).await;
        return Err(error.into());
    }
    if bytes.len() as u64 > max_bytes {
        let _ = ftp.abort(stream).await;
        bail!("file exceeds editor limit of {max_bytes} bytes");
    }
    ftp.finalize_retr_stream(stream).await?;
    Ok(bytes)
}

async fn upload<T: TokioTlsStream + Send + 'static>(
    ftp: &mut ImplAsyncFtpStream<T>,
    local_path: &Path,
    remote_path: &str,
    control: TransferControl,
    progress: tokio::sync::watch::Sender<TransferProgress>,
) -> Result<TransferIoOutcome> {
    let mut local = tokio::fs::File::open(local_path).await?;
    let total = local.metadata().await?.len();
    let offset = match ftp.size(remote_path).await {
        Ok(size) => size as u64,
        Err(_) => 0,
    };
    if offset > total {
        bail!("remote partial file is larger than local source");
    }
    ensure_dir(ftp, &parent_remote_path(remote_path)).await?;
    local.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut stream = if offset > 0 {
        match ftp.append_with_stream(remote_path).await {
            Ok(stream) => stream,
            Err(append_error) => {
                ftp.resume_transfer(offset as usize)
                    .await
                    .with_context(|| format!("APPE failed ({append_error}); REST failed"))?;
                ftp.put_with_stream(remote_path).await?
            }
        }
    } else {
        ftp.put_with_stream(remote_path).await?
    };
    let outcome = crate::sftp::transfer::copy_with_progress(
        &mut local,
        &mut stream,
        offset,
        total,
        &control,
        &progress,
    )
    .await?;
    if outcome == TransferIoOutcome::Completed {
        ftp.finalize_put_stream(stream).await?;
        let remote_size = ftp.size(remote_path).await? as u64;
        if remote_size != total {
            bail!("FTP upload verification failed: remote {remote_size} bytes, expected {total}");
        }
    } else {
        let _ = ftp.abort(stream).await;
    }
    Ok(outcome)
}

async fn download<T: TokioTlsStream + Send + 'static>(
    ftp: &mut ImplAsyncFtpStream<T>,
    remote_path: &str,
    local_path: &Path,
    control: TransferControl,
    progress: tokio::sync::watch::Sender<TransferProgress>,
) -> Result<TransferIoOutcome> {
    let total = ftp.size(remote_path).await? as u64;
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let offset = match tokio::fs::metadata(local_path).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    if offset > total {
        bail!("local partial file is larger than remote source");
    }
    if offset > 0 {
        ftp.resume_transfer(offset as usize).await?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true);
    if offset == 0 {
        options.truncate(true);
    }
    let mut local = options.open(local_path).await?;
    local.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut stream = ftp.retr_as_stream(remote_path).await?;
    let outcome = crate::sftp::transfer::copy_with_progress(
        &mut stream,
        &mut local,
        offset,
        total,
        &control,
        &progress,
    )
    .await?;
    if outcome == TransferIoOutcome::Completed {
        ftp.finalize_retr_stream(stream).await?;
    } else {
        let _ = ftp.abort(stream).await;
    }
    Ok(outcome)
}

async fn replace<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    partial: &str,
    destination: &str,
    overwrite: bool,
) -> Result<()> {
    let exists = ftp.size(destination).await.is_ok();
    if exists && !overwrite {
        bail!("remote destination already exists: {destination}");
    }
    let backup = format!("{destination}.fileterm-backup-{}", uuid::Uuid::new_v4());
    if exists {
        ftp.rename(destination, &backup).await?;
    }
    if let Err(error) = ftp.rename(partial, destination).await {
        if exists {
            let _ = ftp.rename(backup.as_str(), destination).await;
        }
        return Err(error.into());
    }
    if exists {
        let _ = ftp.rm(&backup).await;
    }
    Ok(())
}

async fn remove_if_exists<T: TokioTlsStream + Send>(
    ftp: &mut ImplAsyncFtpStream<T>,
    path: &str,
) -> Result<()> {
    match ftp.rm(path).await {
        Ok(()) => Ok(()),
        Err(error) if is_missing_message(&error.to_string()) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn quit<T: TokioTlsStream + Send>(ftp: &mut ImplAsyncFtpStream<T>) -> Result<()> {
    ftp.quit().await?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_listing(line: &str) -> Option<ParsedListing> {
    if let Ok(entry) = ListParser::parse_posix(line) {
        return Some(ParsedListing {
            entry,
            type_is_trusted: true,
        });
    }
    if let Ok(entry) = ListParser::parse_dos(line) {
        return Some(ParsedListing {
            entry,
            type_is_trusted: true,
        });
    }
    if looks_like_mlsd(line) {
        if let Ok(entry) = ListParser::parse_mlsd(line) {
            return Some(ParsedListing {
                entry,
                type_is_trusted: true,
            });
        }
    }
    line.parse::<ListedFile>().ok().map(|entry| ParsedListing {
        entry,
        type_is_trusted: false,
    })
}

fn looks_like_mlsd(line: &str) -> bool {
    line.trim_start()
        .split_once(' ')
        .map(|(facts, _)| facts)
        .is_some_and(|facts| {
            facts.contains(';') && facts.split(';').any(|fact| fact.split_once('=').is_some())
        })
}

fn is_unsupported_command(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "500",
        "501",
        "502",
        "504",
        "unknown command",
        "not implemented",
        "unsupported",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn is_missing_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "not found",
        "no such",
        "does not exist",
        "cannot find",
        "550",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn listing_permission(line: &str) -> Option<String> {
    let token = line.split_whitespace().next().unwrap_or_default();
    if token.len() == 10 && matches!(token.as_bytes().first(), Some(b'-' | b'd' | b'l')) {
        return Some(token.to_string());
    }
    None
}

pub fn parent_remote_path(path: &str) -> String {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

pub fn join_remote_path(directory: &str, name: &str) -> String {
    if directory == "/" || directory.is_empty() {
        format!("/{name}")
    } else {
        format!("{}/{name}", directory.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_rejects_unknown_security_mode() {
        let error = FtpProfile::from_value(&serde_json::json!({
            "host": "example.test",
            "securityMode": "broken"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unsupported FTP security mode"));
    }

    #[test]
    fn classic_and_mlsd_rows_keep_metadata() {
        let posix = parse_listing("drwxr-xr-x 5 0 0 4096 Jun 18 23:00 data").unwrap();
        assert!(posix.entry.is_directory());
        assert_eq!(posix.entry.name(), "data");
        let mlsd = parse_listing("type=file;size=42;modify=20260715163248; report.txt").unwrap();
        assert!(!mlsd.entry.is_directory());
        assert_eq!(mlsd.entry.size(), 42);
    }

    #[test]
    fn remote_paths_stay_rooted() {
        assert_eq!(join_remote_path("/", "a"), "/a");
        assert_eq!(parent_remote_path("/a/b"), "/a");
        assert_eq!(parent_remote_path("/a"), "/");
    }

    #[test]
    fn text_revision_hash_detects_same_size_changes() {
        assert_ne!(sha256_hex(b"left"), sha256_hex(b"rift"));
        assert_eq!(sha256_hex(b"left").len(), 64);
    }
}
