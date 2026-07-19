use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result as AnyResult};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

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
        return Err(AppError::Command(
            "密钥备注不能超过 120 个字符。".to_string(),
        ));
    }

    let inspected = inspect_private_key(Path::new(source_path))?;
    import_inspected(app, inspected, note)
}

fn import_inspected(
    app: &AppHandle,
    inspected: InspectedPrivateKey,
    note: &str,
) -> Result<SshKeyImportResult> {
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
        return Err(AppError::Command(
            "密钥备注不能超过 120 个字符。".to_string(),
        ));
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
        return Err(AppError::Command(
            "密钥名称不能超过 120 个字符。".to_string(),
        ));
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
    let result = write_secrets(&paths, &secrets);
    secrets.passphrases.values_mut().for_each(Zeroize::zeroize);
    result?;
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
    let mut secrets = read_secrets(&paths).map_err(anyhow::Error::from)?;
    let passphrase = secrets
        .passphrases
        .remove(key_id)
        .filter(|value| !value.is_empty());
    secrets.passphrases.values_mut().for_each(Zeroize::zeroize);
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
        let layout = read_legacy_layout(app).unwrap_or_default();
        write_layout(&paths, &layout)?;
    }
    Ok(paths)
}

fn read_legacy_layout(app: &AppHandle) -> Option<SshKeyLayout> {
    let path = storage::workspace_file(app, "ui-state.json").ok()?;
    let root: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let raw = root
        .get("values")
        .and_then(|values| values.get("ssh-key-manager-ui"))
        .or_else(|| root.get("ssh-key-manager-ui"))?
        .as_str()?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some(SshKeyLayout {
        folders: serde_json::from_value(value.get("folders")?.clone()).ok()?,
        assignments: serde_json::from_value(
            value
                .get("assignments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .ok()?,
        item_order: serde_json::from_value(
            value
                .get("itemOrder")
                .or_else(|| value.get("keyOrder"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .ok()?,
    })
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
    if let Err(error) = storage::replace_file_atomically(&temporary, path) {
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
    bytes: Zeroizing<Vec<u8>>,
    name: String,
    algorithm: String,
    fingerprint: String,
    encrypted: bool,
}

fn inspect_private_key(path: &Path) -> Result<InspectedPrivateKey> {
    let file_metadata = fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?;
    if !file_metadata.is_file()
        || file_metadata.len() == 0
        || file_metadata.len() > MAX_PRIVATE_KEY_BYTES
    {
        return Err(AppError::Command(
            "私钥文件为空、无效或超过 1 MB 限制。".to_string(),
        ));
    }
    let bytes = Zeroizing::new(
        fs::read(path).map_err(|error| AppError::Storage(format!("无法读取私钥文件: {error}")))?,
    );
    let content = Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| {
        AppError::Command("无法识别该文件，请选择文本格式的 SSH 私钥。".to_string())
    })?);
    if !content.contains("PRIVATE KEY") {
        return Err(AppError::Command("选择的文件不是 SSH 私钥。".to_string()));
    }
    let decoded = russh_keys::decode_secret_key(&content, None);
    let (algorithm, fingerprint, encrypted) = match decoded {
        Ok(key) => {
            let public = key
                .clone_public_key()
                .map_err(|_| AppError::Command("无法从 SSH 私钥提取公钥。".to_string()))?;
            (
                key.name().to_string(),
                format!("SHA256:{}", public.fingerprint()),
                false,
            )
        }
        Err(russh_keys::Error::KeyIsEncrypted) => (
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
    let key_ids = keys
        .iter()
        .map(|key| key.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let folder_ids = layout
        .folders
        .iter()
        .map(|folder| folder.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    layout.assignments.retain(|key_id, folder_id| {
        key_ids.contains(key_id.as_str()) && folder_ids.contains(folder_id.as_str())
    });
    layout
        .item_order
        .retain(|id, _| key_ids.contains(id.as_str()) || folder_ids.contains(id.as_str()));
}

fn validate_key_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| AppError::Command("无效的 SSH 密钥 ID".to_string()))
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
        fs::write(
            &malformed,
            "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----",
        )
        .unwrap();
        assert!(select_file(&app, &malformed).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepts_real_ed25519_pkcs8_and_rsa_pem_keys() {
        let (_, directory) = test_app();

        let ed25519 = russh_keys::key::KeyPair::generate_ed25519().unwrap();
        let mut ed25519_pem = Vec::new();
        russh_keys::encode_pkcs8_pem(&ed25519, &mut ed25519_pem).unwrap();
        let ed25519_path = directory.join("id_ed25519");
        fs::write(&ed25519_path, ed25519_pem).unwrap();
        let inspected_ed25519 = inspect_private_key(&ed25519_path).unwrap();
        assert_eq!(inspected_ed25519.algorithm, "ssh-ed25519");
        assert!(!inspected_ed25519.encrypted);
        assert!(inspected_ed25519.fingerprint.starts_with("SHA256:"));

        let rsa =
            russh_keys::key::KeyPair::generate_rsa(2048, russh_keys::key::SignatureHash::SHA2_256)
                .unwrap();
        let rsa_pem = match &rsa {
            russh_keys::key::KeyPair::RSA { key, .. } => key.private_key_to_pem().unwrap(),
            _ => unreachable!(),
        };
        let rsa_path = directory.join("id_rsa");
        fs::write(&rsa_path, rsa_pem).unwrap();
        let inspected_rsa = inspect_private_key(&rsa_path).unwrap();
        assert_eq!(inspected_rsa.algorithm, "rsa-sha2-256");
        assert!(!inspected_rsa.encrypted);
        assert!(inspected_rsa.fingerprint.starts_with("SHA256:"));

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn accepts_ssh_keygen_openssh_rsa_when_available() {
        use std::process::Command;

        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            return;
        }
        let (_, directory) = test_app();
        let key_path = directory.join("id_rsa_openssh");
        let output = Command::new("ssh-keygen")
            .args(["-q", "-t", "rsa", "-b", "2048", "-N", "", "-f"])
            .arg(&key_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let inspected = inspect_private_key(&key_path).unwrap();
        assert!(inspected.algorithm.starts_with("rsa-sha2-"));
        assert!(!inspected.encrypted);
        assert!(inspected.fingerprint.starts_with("SHA256:"));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_malformed_openssh_key_instead_of_treating_it_as_encrypted() {
        let (_, directory) = test_app();
        let malformed = directory.join("malformed-openssh");
        fs::write(
            &malformed,
            "-----BEGIN OPENSSH PRIVATE KEY-----\naW52YWxpZA==\n-----END OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();

        let error = match inspect_private_key(&malformed) {
            Ok(_) => panic!("malformed OpenSSH key must be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("无法识别"));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn layout_rejects_path_like_ids_and_unknown_folders() {
        let (app, directory) = test_app();
        let mut layout = SshKeyLayout::default();
        layout
            .assignments
            .insert("../secret".to_string(), "missing".to_string());
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
        assert!(!fs::read_to_string(&paths.index_path)
            .unwrap()
            .contains("private material"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(paths.keys_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    fn inspected(fingerprint: &str, bytes: &[u8]) -> InspectedPrivateKey {
        InspectedPrivateKey {
            bytes: Zeroizing::new(bytes.to_vec()),
            name: "id_ed25519".to_string(),
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: fingerprint.to_string(),
            encrypted: false,
        }
    }

    #[test]
    fn import_duplicate_rename_and_delete_preserve_library_invariants() {
        let (app, directory) = test_app();
        let first =
            import_inspected(&app, inspected("SHA256:first", b"PRIVATE ONE"), "生产").unwrap();
        assert!(!first.duplicate);
        let duplicate = import_inspected(
            &app,
            inspected("SHA256:first", b"DIFFERENT CONTENT"),
            "重复",
        )
        .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.key.id, first.key.id);
        assert_eq!(list(&app).unwrap().len(), 1);

        let renamed = rename(&app, &first.key.id, "deploy-key").unwrap();
        assert_eq!(renamed.name, "deploy-key");
        let updated = update_note(&app, &first.key.id, "新的备注").unwrap();
        assert_eq!(updated.note.as_deref(), Some("新的备注"));
        let paths = KeyPaths::for_app(&app).unwrap();
        let mut passphrases = HashMap::new();
        passphrases.insert(first.key.id.clone(), "secret".to_string());
        write_secrets(
            &paths,
            &StoredSshKeySecrets {
                version: STORE_VERSION,
                passphrases,
            },
        )
        .unwrap();
        let resolved = resolve_managed_key(&app, &first.key.id).unwrap();
        assert_eq!(resolved.private_key, "PRIVATE ONE");
        assert_eq!(resolved.passphrase.as_deref(), Some("secret"));

        delete(&app, &first.key.id).unwrap();
        assert!(list(&app).unwrap().is_empty());
        assert!(!directory
            .join("ssh-keys")
            .join(format!("{}.key", first.key.id))
            .exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn referenced_key_cannot_be_deleted_and_secret_never_enters_public_json() {
        let (app, directory) = test_app();
        let imported = import_inspected(
            &app,
            inspected("SHA256:referenced", b"TOP SECRET PRIVATE KEY"),
            "引用测试",
        )
        .unwrap();
        storage::write_json_array(
            &app,
            "profiles.json",
            &[serde_json::json!({
                "id": "profile-1",
                "type": "ssh",
                "name": "production",
                "privateKeyId": imported.key.id,
            })],
        )
        .unwrap();

        let error = delete(&app, &imported.key.id).unwrap_err().to_string();
        assert!(error.contains("production"));
        let public_json = fs::read_to_string(directory.join("ssh-keys.json")).unwrap();
        assert!(!public_json.contains("TOP SECRET PRIVATE KEY"));
        assert!(!error.contains("TOP SECRET PRIVATE KEY"));
        assert_eq!(list(&app).unwrap()[0].usage_count, 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_electron_folder_layout_is_migrated_once() {
        let (app, directory) = test_app();
        let folder_id = uuid::Uuid::new_v4().to_string();
        fs::write(
            directory.join("ui-state.json"),
            serde_json::to_vec(&serde_json::json!({
                "values": {
                    "ssh-key-manager-ui": serde_json::json!({
                        "folders": [{ "id": folder_id, "name": "历史分组" }],
                        "assignments": {},
                        "itemOrder": {}
                    }).to_string()
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let layout = get_layout(&app).unwrap();
        assert_eq!(layout.folders[0].name, "历史分组");
        assert!(directory.join("ssh-key-layout.json").is_file());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn layout_persists_folders_and_heals_deleted_key_assignments() {
        let (app, directory) = test_app();
        let imported = import_inspected(
            &app,
            inspected("SHA256:layout", b"PRIVATE LAYOUT"),
            "布局测试",
        )
        .unwrap();
        let folder_id = uuid::Uuid::new_v4().to_string();
        let mut layout = SshKeyLayout {
            folders: vec![SshKeyFolder {
                id: folder_id.clone(),
                name: "生产".to_string(),
            }],
            ..SshKeyLayout::default()
        };
        layout
            .assignments
            .insert(imported.key.id.clone(), folder_id.clone());
        save_layout(&app, layout).unwrap();
        assert_eq!(
            get_layout(&app).unwrap().assignments.get(&imported.key.id),
            Some(&folder_id)
        );
        delete(&app, &imported.key.id).unwrap();
        assert!(get_layout(&app).unwrap().assignments.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}
