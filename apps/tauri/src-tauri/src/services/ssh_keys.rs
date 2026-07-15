use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::AppError;

const MAX_PRIVATE_KEY_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug)]
pub struct ManagedSshKey {
    pub key: StoredSshKey,
    pub private_key: String,
    pub saved_passphrase: Option<String>,
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
    base_dir: PathBuf,
    keys_dir: PathBuf,
    index_path: PathBuf,
    secrets_path: PathBuf,
}

impl KeyPaths {
    fn for_app(app: &AppHandle) -> Result<Self, AppError> {
        let index_path = crate::storage::workspace_file(app, "ssh-keys.json")?;
        let base_dir = index_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::Storage("无法解析 SSH 密钥存储目录".to_string()))?;
        Ok(Self {
            keys_dir: base_dir.join("ssh-keys"),
            secrets_path: base_dir.join("ssh-key-secrets.json"),
            base_dir,
            index_path,
        })
    }

    fn key_path(&self, id: &str) -> Result<PathBuf, AppError> {
        uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::Command("无效的 SSH 密钥 ID".to_string()))?;
        Ok(self.keys_dir.join(format!("{id}.key")))
    }

    fn legacy_dir(&self) -> Option<PathBuf> {
        Some(self.base_dir.parent()?.join("FileTerm"))
    }
}

pub fn list(app: &AppHandle) -> Result<Vec<serde_json::Value>, AppError> {
    let paths = ensure_store(app)?;
    let index = read_index(&paths)?;
    let usage = key_usage_counts(app)?;
    Ok(index
        .keys
        .iter()
        .map(|key| metadata_value(key, *usage.get(&key.id).unwrap_or(&0)))
        .collect())
}

pub async fn select_file(app: &AppHandle) -> Result<Option<serde_json::Value>, AppError> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("导入 SSH 私钥")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let source_path = file.path().to_path_buf();
    let inspected = inspect_private_key(&source_path)?;
    let paths = ensure_store(app)?;
    let index = read_index(&paths)?;
    let existing = index
        .keys
        .iter()
        .find(|key| key.fingerprint == inspected.fingerprint);
    let usage = key_usage_counts(app)?;
    Ok(Some(serde_json::json!({
        "sourcePath": source_path.to_string_lossy(),
        "fileName": inspected.name,
        "existingKey": existing.map(|key| metadata_value(key, *usage.get(&key.id).unwrap_or(&0))),
    })))
}

pub fn import(
    app: &AppHandle,
    source_path: Option<String>,
    note: Option<String>,
) -> Result<Option<serde_json::Value>, AppError> {
    let note = note.unwrap_or_default().trim().to_string();
    if note.is_empty() {
        return Err(AppError::Command("请输入密钥备注。".to_string()));
    }
    let Some(source_path) = source_path else {
        return Ok(None);
    };
    let source_path = PathBuf::from(source_path);
    let inspected = inspect_private_key(&source_path)?;
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let usage = key_usage_counts(app)?;
    if let Some(existing) = index
        .keys
        .iter()
        .find(|key| key.fingerprint == inspected.fingerprint)
    {
        return Ok(Some(serde_json::json!({
            "key": metadata_value(existing, *usage.get(&existing.id).unwrap_or(&0)),
            "duplicate": true,
        })));
    }

    let key = StoredSshKey {
        id: uuid::Uuid::new_v4().to_string(),
        name: inspected.name,
        note: Some(note),
        algorithm: inspected.algorithm,
        fingerprint: inspected.fingerprint,
        encrypted: inspected.encrypted,
        imported_at: now_millis(),
    };
    let key_path = paths.key_path(&key.id)?;
    write_bytes_atomic(&key_path, &inspected.bytes)?;
    lock_down_file(&key_path);
    index.keys.insert(0, key.clone());
    if let Err(error) = write_index(&paths, &index) {
        let _ = fs::remove_file(&key_path);
        return Err(error);
    }
    Ok(Some(
        serde_json::json!({ "key": metadata_value(&key, 0), "duplicate": false }),
    ))
}

pub fn update_note(
    app: &AppHandle,
    key_id: &str,
    note: String,
) -> Result<serde_json::Value, AppError> {
    let note = note.trim().to_string();
    if note.is_empty() {
        return Err(AppError::Command("密钥备注不能为空。".to_string()));
    }
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let key = index
        .keys
        .iter_mut()
        .find(|key| key.id == key_id)
        .ok_or_else(|| AppError::Command("SSH 密钥不存在。".to_string()))?;
    key.note = Some(note);
    let updated = key.clone();
    write_index(&paths, &index)?;
    let usage = key_usage_counts(app)?;
    Ok(metadata_value(&updated, *usage.get(key_id).unwrap_or(&0)))
}

pub fn delete(app: &AppHandle, key_id: &str) -> Result<(), AppError> {
    let paths = ensure_store(app)?;
    let mut index = read_index(&paths)?;
    let Some(position) = index.keys.iter().position(|key| key.id == key_id) else {
        return Ok(());
    };
    let profiles = crate::storage::read_json_array(app, "profiles.json")?;
    let users: Vec<String> = profiles
        .iter()
        .filter(|profile| profile.get("type").and_then(|value| value.as_str()) == Some("ssh"))
        .filter(|profile| {
            profile.get("privateKeyId").and_then(|value| value.as_str()) == Some(key_id)
        })
        .filter_map(|profile| {
            profile
                .get("name")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .collect();
    if !users.is_empty() {
        return Err(AppError::Command(format!(
            "该密钥正在被以下连接使用：{}",
            users.join("、")
        )));
    }
    let key = index.keys.remove(position);
    let key_path = paths.key_path(&key.id)?;
    if key_path.exists() {
        fs::remove_file(&key_path).map_err(|error| AppError::Storage(error.to_string()))?;
    }
    write_index(&paths, &index)?;
    let mut secrets = read_secrets(&paths)?;
    secrets.passphrases.remove(key_id);
    write_secrets(&paths, &secrets)
}

pub fn resolve(app: &AppHandle, key_id: &str) -> Result<ManagedSshKey, AppError> {
    let paths = ensure_store(app)?;
    let key = read_index(&paths)?
        .keys
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| AppError::Command("选择的 SSH 密钥不存在，请重新选择。".to_string()))?;
    let private_key = fs::read_to_string(paths.key_path(&key.id)?)
        .map_err(|error| AppError::Storage(format!("无法读取 SSH 私钥: {error}")))?;
    let saved_passphrase = read_secrets(&paths)?.passphrases.get(&key.id).cloned();
    Ok(ManagedSshKey {
        key,
        private_key,
        saved_passphrase,
    })
}

pub fn set_passphrase(
    app: &AppHandle,
    key_id: &str,
    passphrase: Option<String>,
) -> Result<(), AppError> {
    let paths = ensure_store(app)?;
    if !read_index(&paths)?.keys.iter().any(|key| key.id == key_id) {
        return Err(AppError::Command("SSH 密钥不存在。".to_string()));
    }
    let mut secrets = read_secrets(&paths)?;
    match passphrase.filter(|value| !value.is_empty()) {
        Some(value) => {
            secrets.passphrases.insert(key_id.to_string(), value);
        }
        None => {
            secrets.passphrases.remove(key_id);
        }
    }
    write_secrets(&paths, &secrets)
}

fn ensure_store(app: &AppHandle) -> Result<KeyPaths, AppError> {
    let paths = KeyPaths::for_app(app)?;
    fs::create_dir_all(&paths.keys_dir).map_err(|error| AppError::Storage(error.to_string()))?;
    lock_down_dir(&paths.keys_dir);
    migrate_legacy(&paths)?;
    if !paths.index_path.exists() {
        write_index(
            &paths,
            &StoredSshKeyIndex {
                version: 1,
                keys: Vec::new(),
            },
        )?;
    }
    if !paths.secrets_path.exists() {
        write_secrets(
            &paths,
            &StoredSshKeySecrets {
                version: 1,
                passphrases: HashMap::new(),
            },
        )?;
    }
    Ok(paths)
}

fn migrate_legacy(paths: &KeyPaths) -> Result<(), AppError> {
    let Some(legacy_dir) = paths.legacy_dir() else {
        return Ok(());
    };
    let legacy_index_path = legacy_dir.join("ssh-keys.json");
    if !legacy_index_path.exists() {
        return Ok(());
    }
    let mut current = read_json(
        &paths.index_path,
        StoredSshKeyIndex {
            version: 1,
            keys: Vec::new(),
        },
    )?;
    let legacy = read_json(
        &legacy_index_path,
        StoredSshKeyIndex {
            version: 1,
            keys: Vec::new(),
        },
    )?;
    let known: HashSet<String> = current.keys.iter().map(|key| key.id.clone()).collect();
    let mut changed = false;
    for key in legacy.keys {
        if uuid::Uuid::parse_str(&key.id).is_err() || known.contains(&key.id) {
            continue;
        }
        let source = legacy_dir.join("ssh-keys").join(format!("{}.key", key.id));
        let target = paths.key_path(&key.id)?;
        if source.is_file() && !target.exists() {
            fs::copy(&source, &target).map_err(|error| AppError::Storage(error.to_string()))?;
            lock_down_file(&target);
        }
        if target.is_file() {
            current.keys.push(key);
            changed = true;
        }
    }
    if changed {
        write_index(paths, &current)?;
    }

    let legacy_secrets_path = legacy_dir.join("ssh-key-secrets.json");
    if legacy_secrets_path.exists() {
        let mut current_secrets = read_secrets(paths)?;
        let legacy_secrets = read_json(
            &legacy_secrets_path,
            StoredSshKeySecrets {
                version: 1,
                passphrases: HashMap::new(),
            },
        )?;
        let before = current_secrets.passphrases.len();
        for (id, passphrase) in legacy_secrets.passphrases {
            current_secrets.passphrases.entry(id).or_insert(passphrase);
        }
        if current_secrets.passphrases.len() != before {
            write_secrets(paths, &current_secrets)?;
        }
    }
    Ok(())
}

fn read_index(paths: &KeyPaths) -> Result<StoredSshKeyIndex, AppError> {
    read_json(
        &paths.index_path,
        StoredSshKeyIndex {
            version: 1,
            keys: Vec::new(),
        },
    )
}

fn write_index(paths: &KeyPaths, index: &StoredSshKeyIndex) -> Result<(), AppError> {
    write_json_atomic(&paths.index_path, index)
}

fn read_secrets(paths: &KeyPaths) -> Result<StoredSshKeySecrets, AppError> {
    read_json(
        &paths.secrets_path,
        StoredSshKeySecrets {
            version: 1,
            passphrases: HashMap::new(),
        },
    )
}

fn write_secrets(paths: &KeyPaths, secrets: &StoredSshKeySecrets) -> Result<(), AppError> {
    write_json_atomic(&paths.secrets_path, secrets)?;
    lock_down_file(&paths.secrets_path);
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path, fallback: T) -> Result<T, AppError> {
    if !path.exists() {
        return Ok(fallback);
    }
    let content = fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
    serde_json::from_str(&content).map_err(|error| AppError::Serialization(error.to_string()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    write_bytes_atomic(path, &content)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let temporary = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    fs::write(&temporary, bytes).map_err(|error| AppError::Storage(error.to_string()))?;
    lock_down_file(&temporary);
    fs::rename(&temporary, path).map_err(|error| AppError::Storage(error.to_string()))
}

fn key_usage_counts(app: &AppHandle) -> Result<HashMap<String, usize>, AppError> {
    let mut usage = HashMap::new();
    for profile in crate::storage::read_json_array(app, "profiles.json")? {
        if profile.get("type").and_then(|value| value.as_str()) == Some("ssh") {
            if let Some(key_id) = profile.get("privateKeyId").and_then(|value| value.as_str()) {
                *usage.entry(key_id.to_string()).or_insert(0) += 1;
            }
        }
    }
    Ok(usage)
}

fn metadata_value(key: &StoredSshKey, usage_count: usize) -> serde_json::Value {
    serde_json::json!({
        "id": key.id,
        "name": key.name,
        "note": key.note,
        "algorithm": key.algorithm,
        "fingerprint": key.fingerprint,
        "encrypted": key.encrypted,
        "importedAt": key.imported_at,
        "usageCount": usage_count,
    })
}

struct InspectedPrivateKey {
    bytes: Vec<u8>,
    name: String,
    algorithm: String,
    fingerprint: String,
    encrypted: bool,
}

fn inspect_private_key(path: &Path) -> Result<InspectedPrivateKey, AppError> {
    let metadata = fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(AppError::Command(
            "私钥文件为空、无效或超过 1 MB 限制。".to_string(),
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?;
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
            ))
        }
    };
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ssh-key")
        .to_string();
    Ok(InspectedPrivateKey {
        bytes,
        name,
        algorithm,
        fingerprint,
        encrypted,
    })
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
fn lock_down_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn lock_down_file(_path: &Path) {}

#[cfg(unix)]
fn lock_down_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn lock_down_dir(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::probably_encrypted;

    #[test]
    fn recognizes_private_key_headers_without_treating_public_keys_as_private() {
        assert!(!probably_encrypted("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(probably_encrypted("-----BEGIN OPENSSH PRIVATE KEY-----"));
    }
}
