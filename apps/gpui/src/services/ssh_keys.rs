use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::backend::{storage, AppHandle};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSshKey {
    id: String,
}

#[derive(Default, Deserialize)]
struct StoredSshKeyIndex {
    keys: Vec<StoredSshKey>,
}

#[derive(Default, Deserialize)]
struct StoredSshKeySecrets {
    passphrases: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrivateKey {
    pub private_key: String,
    pub passphrase: Option<String>,
}

pub fn resolve_managed_key(app: &AppHandle, key_id: &str) -> Result<ResolvedPrivateKey> {
    uuid::Uuid::parse_str(key_id).context("invalid managed SSH key id")?;

    let index: StoredSshKeyIndex = read_json_or_default(
        storage::workspace_file(app, "ssh-keys.json")?,
        "read managed SSH key index",
    )?;
    if !index.keys.iter().any(|key| key.id == key_id) {
        bail!("selected managed SSH key does not exist");
    }

    let key_path = app
        .app_data_dir()
        .join("ssh-keys")
        .join(format!("{key_id}.key"));
    lock_down_file(&key_path)?;
    let private_key = fs::read_to_string(&key_path)
        .with_context(|| format!("read managed SSH private key {}", key_path.display()))?;
    let secrets: StoredSshKeySecrets = read_json_or_default(
        storage::workspace_file(app, "ssh-key-secrets.json")?,
        "read managed SSH key secrets",
    )?;

    Ok(ResolvedPrivateKey {
        private_key,
        passphrase: secrets
            .passphrases
            .get(key_id)
            .filter(|value| !value.is_empty())
            .cloned(),
    })
}

pub fn discover_default_keys() -> Vec<ResolvedPrivateKey> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa", "identity"]
        .into_iter()
        .filter_map(|name| {
            let path = home.join(".ssh").join(name);
            fs::read_to_string(path)
                .ok()
                .map(|private_key| ResolvedPrivateKey {
                    private_key,
                    passphrase: None,
                })
        })
        .collect()
}

fn read_json_or_default<T>(path: PathBuf, operation: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("{operation}: {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("{operation}: {}", path.display()))
}

#[cfg(unix)]
fn lock_down_file(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .with_context(|| format!("inspect managed SSH key {}", path.display()))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("protect managed SSH key {}", path.display()))
}

#[cfg(not(unix))]
fn lock_down_file(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("managed SSH key does not exist: {}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_managed_key_and_saved_passphrase() {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-managed-key-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(directory.join("ssh-keys")).unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();
        fs::write(
            directory.join("ssh-keys.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "keys": [{ "id": key_id }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("ssh-key-secrets.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "passphrases": { (key_id.clone()): "secret" }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("ssh-keys").join(format!("{key_id}.key")),
            "PRIVATE",
        )
        .unwrap();

        let resolved = resolve_managed_key(&AppHandle::new(directory.clone()), &key_id).unwrap();
        assert_eq!(resolved.private_key, "PRIVATE");
        assert_eq!(resolved.passphrase.as_deref(), Some("secret"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_unknown_managed_key() {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-missing-key-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let result = resolve_managed_key(
            &AppHandle::new(directory.clone()),
            &uuid::Uuid::new_v4().to_string(),
        );
        assert!(result.is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
