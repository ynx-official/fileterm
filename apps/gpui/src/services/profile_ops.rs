use std::{collections::HashMap, fs, path::Path};

use serde_json::{Map, Value};

use crate::{
    backend::{storage, AppHandle},
    error::AppError,
};

const DEFAULT_GROUP: &str = "默认";

pub fn read_public_connection_library(
    app: &AppHandle,
) -> Result<(Vec<Value>, Vec<Value>), AppError> {
    storage::migrate_legacy_data_once(app)?;

    let mut folders = storage::read_json_array(app, "folders.json")?;
    if heal_folders(&mut folders) {
        storage::write_json_array(app, "folders.json", &folders)?;
    }

    let raw_profiles = read_raw_array(app, "profiles.json")?;
    let mut profiles = merge_profile_secrets(app, raw_profiles.clone())?;
    let healed = heal_profiles(&mut profiles, &folders);
    let stripped_profiles = profiles.iter().map(strip_secret_fields).collect::<Vec<_>>();

    if healed || raw_profiles != stripped_profiles {
        reconcile_profile_secrets(app, &profiles)?;
        storage::write_json_array(app, "profiles.json", &stripped_profiles)?;
    }

    let public_profiles = profiles
        .iter()
        .map(strip_secret_fields_public)
        .collect::<Vec<_>>();
    Ok((public_profiles, folders))
}

pub fn read_connection_profile(app: &AppHandle, profile_id: &str) -> Result<Value, AppError> {
    storage::migrate_legacy_data_once(app)?;
    let profiles = merge_profile_secrets(app, read_raw_array(app, "profiles.json")?)?;
    profiles
        .into_iter()
        .find(|profile| profile.get("id").and_then(Value::as_str) == Some(profile_id))
        .ok_or_else(|| AppError::Storage(format!("connection profile not found: {profile_id}")))
}

pub fn update_trusted_host_fingerprint(
    app: &AppHandle,
    profile_id: &str,
    fingerprint: &str,
) -> Result<(), AppError> {
    let mut profiles = read_raw_array(app, "profiles.json")?;
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.get("id").and_then(Value::as_str) == Some(profile_id))
        .ok_or_else(|| AppError::Storage(format!("connection profile not found: {profile_id}")))?;
    let object = profile
        .as_object_mut()
        .ok_or_else(|| AppError::Storage(format!("invalid connection profile: {profile_id}")))?;
    object.insert(
        "trustedHostFingerprint".to_string(),
        Value::String(fingerprint.to_string()),
    );
    storage::write_json_array(app, "profiles.json", &profiles)
}

fn read_raw_array(app: &AppHandle, name: &str) -> Result<Vec<Value>, AppError> {
    let path = storage::workspace_file(app, name)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
    serde_json::from_str(&content).map_err(|error| AppError::Serialization(error.to_string()))
}

fn merge_profile_secrets(
    app: &AppHandle,
    mut profiles: Vec<Value>,
) -> Result<Vec<Value>, AppError> {
    let secrets = storage::read_json_object(app, "profile-secrets.json")?;
    let Some(secret_profiles) = secrets.get("profiles").and_then(Value::as_object) else {
        return Ok(profiles);
    };

    for profile in &mut profiles {
        let Some(profile_object) = profile.as_object_mut() else {
            continue;
        };
        let Some(profile_id) = profile_object.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(secret_record) = secret_profiles.get(profile_id).and_then(Value::as_object) else {
            continue;
        };

        copy_secret(secret_record, "password", profile_object, "password");
        copy_secret(secret_record, "passphrase", profile_object, "passphrase");
        copy_secret(
            secret_record,
            "privateKeyPath",
            profile_object,
            "privateKeyPath",
        );
        if let Some(password) = secret_value(secret_record, "proxyPassword") {
            if let Some(proxy) = profile_object
                .get_mut("proxy")
                .and_then(Value::as_object_mut)
            {
                proxy.insert("password".to_string(), Value::String(password.to_string()));
            }
        }
    }

    Ok(profiles)
}

fn copy_secret(
    source: &Map<String, Value>,
    source_key: &str,
    target: &mut Map<String, Value>,
    target_key: &str,
) {
    if let Some(value) = secret_value(source, source_key) {
        target.insert(target_key.to_string(), Value::String(value.to_string()));
    }
}

fn secret_value<'a>(record: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    record
        .get(key)
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("value"))
        .and_then(Value::as_str)
}

fn reconcile_profile_secrets(app: &AppHandle, profiles: &[Value]) -> Result<(), AppError> {
    let current = storage::read_json_object(app, "profile-secrets.json")?;
    let current_profiles = current
        .get("profiles")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut next_profiles = Map::new();

    for profile in profiles {
        let Some(profile_object) = profile.as_object() else {
            continue;
        };
        let Some(profile_id) = profile_object.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut secret_record = current_profiles
            .get(profile_id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        set_secret(
            &mut secret_record,
            "password",
            profile_object.get("password"),
        );
        set_secret(
            &mut secret_record,
            "passphrase",
            profile_object.get("passphrase"),
        );
        set_secret(
            &mut secret_record,
            "privateKeyPath",
            profile_object.get("privateKeyPath"),
        );
        let proxy_password = profile_object
            .get("proxy")
            .and_then(Value::as_object)
            .and_then(|proxy| proxy.get("password"));
        set_secret(&mut secret_record, "proxyPassword", proxy_password);

        if !secret_record.is_empty() {
            next_profiles.insert(profile_id.to_string(), Value::Object(secret_record));
        }
    }

    write_confidential_object(
        &storage::workspace_file(app, "profile-secrets.json")?,
        &serde_json::json!({ "profiles": next_profiles }),
    )
}

fn set_secret(record: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    match value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            record.insert(key.to_string(), serde_json::json!({ "value": value }));
        }
        None => {
            record.remove(key);
        }
    }
}

fn write_confidential_object(path: &Path, value: &Value) -> Result<(), AppError> {
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    fs::write(&temporary, content).map_err(|error| AppError::Storage(error.to_string()))?;
    lock_down_file(&temporary)?;
    storage::replace_file_atomically(&temporary, path)?;
    lock_down_file(path)
}

#[cfg(unix)]
fn lock_down_file(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::Storage(error.to_string()))
}

#[cfg(not(unix))]
fn lock_down_file(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

fn heal_folders(folders: &mut [Value]) -> bool {
    let mut dirty = false;
    let mut order = now_millis();
    for folder in folders {
        let Some(object) = folder.as_object_mut() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("folder") {
            object.insert("type".to_string(), Value::String("folder".to_string()));
            dirty = true;
        }
        if object.get("order").and_then(Value::as_f64).is_none() {
            object.insert("order".to_string(), Value::Number(order.into()));
            order = order.saturating_add(1);
            dirty = true;
        }
    }
    dirty
}

fn heal_profiles(profiles: &mut [Value], folders: &[Value]) -> bool {
    let folder_by_name = folders
        .iter()
        .filter_map(|folder| {
            Some((
                folder.get("name")?.as_str()?.to_string(),
                folder.get("id")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let folder_by_id = folder_by_name
        .iter()
        .map(|(name, id)| (id.clone(), name.clone()))
        .collect::<HashMap<_, _>>();
    let mut dirty = false;

    for profile in profiles {
        let Some(object) = profile.as_object_mut() else {
            continue;
        };
        let group = object
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_GROUP)
            .to_string();
        let parent_id = object
            .get("parentId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        if group.is_empty() || group == DEFAULT_GROUP {
            match parent_id.as_deref().and_then(|id| folder_by_id.get(id)) {
                Some(folder_name) if folder_name != &group => {
                    object.insert("group".to_string(), Value::String(folder_name.clone()));
                    dirty = true;
                }
                None if parent_id.is_some() => {
                    object.insert("parentId".to_string(), Value::Null);
                    object.insert(
                        "group".to_string(),
                        Value::String(DEFAULT_GROUP.to_string()),
                    );
                    dirty = true;
                }
                _ => {}
            }
            continue;
        }

        match folder_by_name.get(&group) {
            Some(folder_id) if parent_id.as_deref() != Some(folder_id) => {
                object.insert("parentId".to_string(), Value::String(folder_id.clone()));
                dirty = true;
            }
            None => {
                object.insert("parentId".to_string(), Value::Null);
                object.insert(
                    "group".to_string(),
                    Value::String(DEFAULT_GROUP.to_string()),
                );
                dirty = true;
            }
            _ => {}
        }
    }

    dirty
}

fn strip_secret_fields(profile: &Value) -> Value {
    let mut stripped = profile.clone();
    if let Some(object) = stripped.as_object_mut() {
        for key in ["password", "passphrase", "privateKeyPath", "proxyPassword"] {
            object.remove(key);
        }
        if let Some(proxy) = object.get_mut("proxy").and_then(Value::as_object_mut) {
            proxy.remove("password");
        }
    }
    stripped
}

fn strip_secret_fields_public(profile: &Value) -> Value {
    let has_saved_password = profile
        .get("password")
        .and_then(Value::as_str)
        .is_some_and(|password| !password.is_empty());
    let mut public = strip_secret_fields(profile);
    if let Some(object) = public.as_object_mut() {
        object.insert(
            "hasSavedPassword".to_string(),
            Value::Bool(has_saved_password),
        );
    }
    public
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healing_keeps_group_and_parent_id_consistent() {
        let folders = vec![serde_json::json!({ "id": "f1", "name": "生产" })];
        let mut profiles = vec![serde_json::json!({
            "id": "p1",
            "name": "server",
            "group": "生产",
            "parentId": null
        })];

        assert!(heal_profiles(&mut profiles, &folders));
        assert_eq!(profiles[0]["parentId"], "f1");
    }

    #[test]
    fn public_projection_removes_nested_secrets() {
        let public = strip_secret_fields_public(&serde_json::json!({
            "id": "p1",
            "password": "secret",
            "privateKeyPath": "/tmp/key",
            "proxy": { "type": "http", "password": "proxy-secret" }
        }));

        assert_eq!(public["hasSavedPassword"], true);
        assert!(public.get("password").is_none());
        assert!(public.get("privateKeyPath").is_none());
        assert!(public["proxy"].get("password").is_none());
    }
}
