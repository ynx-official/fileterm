use std::{collections::HashMap, fs, path::{Path, PathBuf}};

use anyhow::{bail, Context, Result as AnyResult};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    backend::{storage, AppHandle},
    error::{AppError, Result},
};

const MAX_PRIVATE_KEY_BYTES: u64 = 1024 * 1024;
const STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredSshKey {
    pub id: String,
    pub name: String,
    pub note: Option<String>,
    pub algorithm: String,
    pub fingerprint: String,
    pub encrypted: bool,
    pub imported_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyMetadata {
    pub id: String,
    pub name: String,
    pub note: Option<String>,
    pub algorithm: String,
    pub fingerprint: String,
    pub encrypted: bool,
    pub imported_at: u64,
    pub usage_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyFileSelection {
    pub source_path: String,
    pub file_name: String,
    pub existing_key: Option<SshKeyMetadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyImportResult {
    pub key: SshKeyMetadata,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SshKeyFolder {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyLayout {
    pub folders: Vec<SshKeyFolder>,
    pub assignments: HashMap<String, String>,
    pub item_order: HashMap<String, u64>,
}

#[derive(Default, Deserialize, Serialize)]
struct StoredSshKeyIndex {
    version: u32,
    keys: Vec<StoredSshKey>,
}

#[derive(Default, Deserialize, Serialize)]
struct StoredSshKeySecrets {
    version: u32,
    passphrases: HashMap<String, String>,
}

struct KeyPaths {
    keys_dir: PathBuf,
    index_path: PathBuf,
    secrets_path: PathBuf,
    layout_path: PathBuf,
}

impl KeyPaths {
    fn for_app(app: &AppHandle) -> Result<Self> {
        let index_path = storage::workspace_file(app, "ssh-keys.json")?;
        let base_dir = index_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::Storage("无法解析 SSH 密钥存储目录".to_string()))?;
        Ok(Self {
            keys_dir: base_dir.join("ssh-keys"),
            secrets_path: base_dir.join("ssh-key-secrets.json"),
            layout_path: base_dir.join("ssh-key-layout.json"),
            index_path,
        })
    }

    fn key_path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::Command("无效的 SSH 密钥 ID".to_string()))?;
        Ok(self.keys_dir.join(format!("{id}.key")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrivateKey {
    pub private_key: String,
    pub passphrase: Option<String>,
}

pub fn list(app: &AppHandle) -> Result<Vec<SshKeyMetadata>> {
    let paths = ensure_store(app)?;
    let usage = key_usage(app)?;
    Ok(read_index(&paths)?
        .keys
        .into_iter()
        .map(|key| metadata(key, &usage))
        .collect())
}

pub fn select_file(app: &AppHandle, source_path: &Path) -> Result<SshKeyFileSelection> {
    let inspected = inspect_private_key(source_path)?;
    let usage = key_usage(app)?;
    let existing_key = read_index(&ensure_store(app)?)?
        .keys
        .into_iter()
        .find(|key| key.fingerprint == inspected.fingerprint)
        .map(|key| metadata(key, &usage));
    Ok(SshKeyFileSelection {
        source_path: source_path.to_string_lossy().to_string(),
        file_name: inspected.name,
        existing_key,
    })
}

pub fn import(app: &AppHandle, source_path: &str, note: &str) -> Result<SshKeyImportResult> {
    let note = note.trim();
    if note.is_empty() {
        return Err(AppError::Command("请输入密钥备注。".to_string()));
    }
    if note.chars().count() > 120 {
        return Err(AppError::Command("密钥备注不能超过 120 个字符。".to_string()));
    }

    let inspected = inspect_private_key(Path::new(source_path))?;
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    if let Some(existing) = index
        .keys
        .iter()
        .find(|key| key.fingerprint == inspected.fingerprint)
        .cloned()
    {
        return Ok(SshKeyImportResult {
            key: metadata(existing, &key_usage(app)?),
            duplicate: true,
        });
    }

    let key = StoredSshKey {
        id: uuid::Uuid::new_v4().to_string(),
        name: inspected.name,
        note: Some(note.to_string()),
        algorithm: inspected.algorithm,
        fingerprint: inspected.fingerprint,
        encrypted: inspected.encrypted,
        imported_at: now_millis(),
    };
    let key_path = paths.key_path(&key.id)?;
    write_bytes_atomic(&key_path, &inspected.bytes)?;
    index.keys.insert(0, key.clone());
    if let Err(error) = write_index(&paths, &index) {
        let _ = fs::remove_file(key_path);
        return Err(error);
    }
    Ok(SshKeyImportResult {
        key: metadata(key, &HashMap::new()),
        duplicate: false,
    })
}

pub fn update_note(app: &AppHandle, key_id: &str, note: &str) -> Result<SshKeyMetadata> {
    validate_key_id(key_id)?;
    let note = note.trim();
    if note.is_empty() {
        return Err(AppError::Command("密钥备注不能为空。".to_string()));
    }
    if note.chars().count() > 120 {
        return Err(AppError::Command("密钥备注不能超过 120 个字符。".to_string()));
    }
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let key = index
        .keys
        .iter_mut()
        .find(|key| key.id == key_id)
        .ok_or_else(|| AppError::Command("SSH 密钥不存在。".to_string()))?;
    key.note = Some(note.to_string());
    let updated = key.clone();
    write_index(&paths, &index)?;
    Ok(metadata(updated, &key_usage(app)?))
}

pub fn rename(app: &AppHandle, key_id: &str, name: &str) -> Result<SshKeyMetadata> {
    validate_key_id(key_id)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Command("密钥名称不能为空。".to_string()));
    }
    if name.chars().count() > 120 {
        return Err(AppError::Command("密钥名称不能超过 120 个字符。".to_string()));
    }
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let key = index
        .keys
        .iter_mut()
        .find(|key| key.id == key_id)
        .ok_or_else(|| AppError::Command("SSH 密钥不存在。".to_string()))?;
    key.name = name.to_string();
    let updated = key.clone();
    write_index(&paths, &index)?;
    Ok(metadata(updated, &key_usage(app)?))
}

pub fn delete(app: &AppHandle, key_id: &str) -> Result<()> {
    validate_key_id(key_id)?;
    let users = key_usage(app)?.remove(key_id).unwrap_or_default();
    if !users.is_empty() {
        return Err(AppError::Command(format!(
            "该密钥正在被以下连接使用：{}",
            users.join("、")
        )));
    }

    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let Some(position) = index.keys.iter().position(|key| key.id == key_id) else {
        return Ok(());
    };
    let key = index.keys.remove(position);
    write_index(&paths, &index)?;
    let key_path = paths.key_path(&key.id)?;
    if let Err(error) = fs::remove_file(&key_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            index.keys.insert(position, key);
            let _ = write_index(&paths, &index);
            return Err(AppError::Storage(error.to_string()));
        }
    }
    let mut secrets = read_secrets(&paths)?;
    secrets.passphrases.remove(key_id);
    write_secrets(&paths, &secrets)?;
    let mut layout = read_layout(&paths)?;
    layout.assignments.remove(key_id);
    layout.item_order.remove(key_id);
    write_layout(&paths, &layout)
}

pub fn get_layout(app: &AppHandle) -> Result<SshKeyLayout> {
    let paths = ensure_store(app)?;
    let mut layout = read_layout(&paths)?;
    heal_layout(&mut layout, &read_index(&paths)?.keys);
    write_layout(&paths, &layout)?;
    Ok(layout)
}

pub fn save_layout(app: &AppHandle, mut layout: SshKeyLayout) -> Result<SshKeyLayout> {
    let paths = ensure_store(app)?;
    validate_layout(&layout)?;
    heal_layout(&mut layout, &read_index(&paths)?.keys);
    write_layout(&paths, &layout)?;
    Ok(layout)
}

pub fn resolve_managed_key(app: &AppHandle, key_id: &str) -> AnyResult<ResolvedPrivateKey> {
    validate_key_id(key_id).map_err(anyhow::Error::from)?;
    let paths = ensure_store(app).map_err(anyhow::Error::from)?;
    let key = read_index(&paths)
        .map_err(anyhow::Error::from)?
        .keys
        .into_iter()
        .find(|key| key.id == key_id)
        .context("selected managed SSH key does not exist")?;
    let key_path = paths.key_path(&key.id).map_err(anyhow::Error::from)?;
    lock_down_file(&key_path).map_err(anyhow::Error::from)?;
    let private_key = fs::read_to_string(&key_path)
        .with_context(|| format!("read managed SSH private key {}", key_path.display()))?;
    let passphrase = read_secrets(&paths)
        .map_err(anyhow::Error::from)?
        .passphrases
        .remove(key_id)
        .filter(|value| !value.is_empty());
    Ok(ResolvedPrivateKey {
        private_key,
        passphrase,
    })
}

pub fn discover_default_keys() -> Vec<ResolvedPrivateKey> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa", "identity"]
        .into_iter()
        .filter_map(|name| {
            fs::read_to_string(home.join(".ssh").join(name))
                .ok()
                .map(|private_key| ResolvedPrivateKey {
                    private_key,
                    passphrase: None,
                })
        })
        .collect()
}

fn ensure_store(app: &AppHandle) -> Result<KeyPaths> {
    let paths = KeyPaths::for_app(app)?;
    fs::create_dir_all(&paths.keys_dir).map_err(|error| AppError::Storage(error.to_string()))?;
    lock_down_dir(&paths.keys_dir)?;
    if !paths.index_path.exists() {
        write_index(
            &paths,
            &StoredSshKeyIndex {
                version: STORE_VERSION,
                keys: Vec::new(),
            },
        )?;
    }
    if !paths.secrets_path.exists() {
        write_secrets(
            &paths,
            &StoredSshKeySecrets {
                version: STORE_VERSION,
                passphrases: HashMap::new(),
            },
        )?;
    } else {
        lock_down_file(&paths.secrets_path)?;
    }
    if !paths.layout_path.exists() {
        write_layout(&paths, &SshKeyLayout::default())?;
    }
    Ok(paths)
}

fn read_index(paths: &KeyPaths) -> Result<StoredSshKeyIndex> {
    read_json(&paths.index_path)
}

fn write_index(paths: &KeyPaths, index: &StoredSshKeyIndex) -> Result<()> {
    write_json_atomic(&paths.index_path, index, false)
}

fn read_secrets(paths: &KeyPaths) -> Result<StoredSshKeySecrets> {
    lock_down_file(&paths.secrets_path)?;
    read_json(&paths.secrets_path)
}

fn write_secrets(paths: &KeyPaths, secrets: &StoredSshKeySecrets) -> Result<()> {
    write_json_atomic(&paths.secrets_path, secrets, true)
}

fn read_layout(paths: &KeyPaths) -> Result<SshKeyLayout> {
    read_json(&paths.layout_path)
}

fn write_layout(paths: &KeyPaths, layout: &SshKeyLayout) -> Result<()> {
    write_json_atomic(&paths.layout_path, layout, false)
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let content = fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
    serde_json::from_str(&content).map_err(|error| AppError::Serialization(error.to_string()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T, confidential: bool) -> Result<()> {
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    write_bytes_atomic(path, &content)?;
    if confidential {
        lock_down_file(path)?;
    }
    Ok(())
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    storage::write_restricted_file(&temporary, bytes)?;
    if let Err(error) = lock_down_file(&temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = storage::replace_file_atomically(temporary.clone(), path) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    lock_down_file(path)
}

fn key_usage(app: &AppHandle) -> Result<HashMap<String, Vec<String>>> {
    let mut usage: HashMap<String, Vec<String>> = HashMap::new();
    for profile in storage::read_json_array(app, "profiles.json")? {
        if profile.get("type").and_then(serde_json::Value::as_str) == Some("ssh") {
            if let Some(key_id) = profile
                .get("privateKeyId")
                .and_then(serde_json::Value::as_str)
            {
                usage.entry(key_id.to_string()).or_default().push(
                    profile
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("未命名连接")
                        .to_string(),
                );
            }
        }
    }
    Ok(usage)
}

fn metadata(key: StoredSshKey, usage: &HashMap<String, Vec<String>>) -> SshKeyMetadata {
    let usage_count = usage.get(&key.id).map_or(0, Vec::len);
    SshKeyMetadata {
        id: key.id,
        name: key.name,
        note: key.note,
        algorithm: key.algorithm,
        fingerprint: key.fingerprint,
        encrypted: key.encrypted,
        imported_at: key.imported_at,
        usage_count,
    }
}

struct InspectedPrivateKey {
    bytes: Vec<u8>,
    name: String,
    algorithm: String,
    fingerprint: String,
    encrypted: bool,
}

fn inspect_private_key(path: &Path) -> Result<InspectedPrivateKey> {
    let file_metadata = fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?;
    if !file_metadata.is_file() || file_metadata.len() == 0 || file_metadata.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(AppError::Command(
            "私钥文件为空、无效或超过 1 MB 限制。".to_string(),
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?;
    let content = String::from_utf8(bytes.clone()).map_err(|_| {
        AppError::Command("无法识别该文件，请选择文本格式的 SSH 私钥。".to_string())
    })?;
    if !content.contains("PRIVATE KEY") {
        return Err(AppError::Command("选择的文件不是 SSH 私钥。".to_string()));
    }
    let decoded = russh::keys::decode_secret_key(&content, None);
    let (algorithm, fingerprint, encrypted) = match decoded {
        Ok(key) => (
            key.public_key().algorithm().as_str().to_string(),
            key.public_key()
                .fingerprint(russh::keys::HashAlg::Sha256)
                .to_string(),
            false,
        ),
        Err(_) if probably_encrypted(&content) => (
            "encrypted".to_string(),
            format!(
                "FILE-SHA256:{}",
                STANDARD_NO_PAD.encode(Sha256::digest(&bytes))
            ),
            true,
        ),
        Err(_) => {
            return Err(AppError::Command(
                "无法识别该文件，请选择 OpenSSH、PEM 或兼容格式的私钥。".to_string(),
            ));
        }
    };
    Ok(InspectedPrivateKey {
        bytes,
        name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("ssh-key")
            .to_string(),
        algorithm,
        fingerprint,
        encrypted,
    })
}

fn validate_layout(layout: &SshKeyLayout) -> Result<()> {
    let mut folder_ids = std::collections::HashSet::new();
    let mut folder_names = std::collections::HashSet::new();
    for folder in &layout.folders {
        uuid::Uuid::parse_str(&folder.id)
            .map_err(|_| AppError::Command("无效的密钥文件夹 ID".to_string()))?;
        let name = folder.name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(AppError::Command("密钥文件夹名称无效。".to_string()));
        }
        if !folder_ids.insert(folder.id.clone()) || !folder_names.insert(name.to_string()) {
            return Err(AppError::Command("密钥文件夹名称或 ID 重复。".to_string()));
        }
    }
    for (key_id, folder_id) in &layout.assignments {
        validate_key_id(key_id)?;
        if !folder_ids.contains(folder_id) {
            return Err(AppError::Command("密钥引用了不存在的文件夹。".to_string()));
        }
    }
    Ok(())
}

fn heal_layout(layout: &mut SshKeyLayout, keys: &[StoredSshKey]) {
    let key_ids = keys.iter().map(|key| key.id.as_str()).collect::<std::collections::HashSet<_>>();
    let folder_ids = layout
        .folders
        .iter()
        .map(|folder| folder.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    layout
        .assignments
        .retain(|key_id, folder_id| key_ids.contains(key_id.as_str()) && folder_ids.contains(folder_id.as_str()));
    layout
        .item_order
        .retain(|id, _| key_ids.contains(id.as_str()) || folder_ids.contains(id.as_str()));
}

fn validate_key_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| AppError::Command("无效的 SSH 密钥 ID".to_string()))
}

fn probably_encrypted(content: &str) -> bool {
    content.contains("ENCRYPTED") || content.contains("BEGIN OPENSSH PRIVATE KEY")
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(unix)]
fn lock_down_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::Storage(error.to_string()))
}

#[cfg(not(unix))]
fn lock_down_file(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(AppError::Storage(format!("文件不存在: {}", path.display())))
    }
}

#[cfg(unix)]
fn lock_down_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| AppError::Storage(error.to_string()))
}

#[cfg(not(unix))]
fn lock_down_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> (AppHandle, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-key-library-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        (AppHandle::new(directory.clone()), directory)
    }

    #[test]
    fn rejects_public_and_malformed_key_files() {
        let (app, directory) = test_app();
        let public = directory.join("id.pub");
        fs::write(&public, "ssh-ed25519 AAAA test").unwrap();
        assert!(select_file(&app, &public).is_err());
        let malformed = directory.join("bad.key");
        fs::write(&malformed, "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----").unwrap();
        assert!(select_file(&app, &malformed).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn layout_rejects_path_like_ids_and_unknown_folders() {
        let (app, directory) = test_app();
        let mut layout = SshKeyLayout::default();
        layout.assignments.insert("../secret".to_string(), "missing".to_string());
        assert!(save_layout(&app, layout).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn metadata_and_private_key_files_are_separate_and_restricted() {
        let (app, directory) = test_app();
        ensure_store(&app).unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();
        let paths = KeyPaths::for_app(&app).unwrap();
        let key_path = paths.key_path(&key_id).unwrap();
        write_bytes_atomic(&key_path, b"private material").unwrap();
        assert!(!fs::read_to_string(&paths.index_path).unwrap().contains("private material"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(key_path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(paths.keys_dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        fs::remove_dir_all(directory).unwrap();
    }
}