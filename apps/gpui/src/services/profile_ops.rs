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

pub fn read_command_library(app: &AppHandle) -> Result<(Vec<Value>, Vec<Value>), AppError> {
    storage::migrate_legacy_data_once(app)?;
    let mut folders = storage::read_json_array(app, "command-folders.json")?;
    let mut commands = storage::read_json_array(app, "commands.json")?;
    if heal_command_folders(&mut folders) {
        storage::write_json_array(app, "command-folders.json", &folders)?;
    }
    if heal_command_templates(&mut commands) {
        storage::write_json_array(app, "commands.json", &commands)?;
    }
    Ok((commands, folders))
}

pub fn create_profile(app: &AppHandle, input: Value) -> Result<Value, AppError> {
    storage::migrate_legacy_data_once(app)?;
    let mut profile = input
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Command("连接配置必须是对象".to_string()))?;
    validate_profile_input(&profile)?;
    let folders = storage::read_json_array(app, "folders.json")?;
    let requested_group = profile
        .get("group")
        .and_then(Value::as_str)
        .filter(|group| !group.trim().is_empty())
        .unwrap_or(DEFAULT_GROUP);
    let (group, parent_id) = resolve_group(&folders, requested_group);
    profile.insert(
        "id".to_string(),
        Value::String(format!("profile-{}", uuid::Uuid::new_v4())),
    );
    profile.insert("group".to_string(), Value::String(group));
    profile.insert("parentId".to_string(), parent_id);
    profile
        .entry("order".to_string())
        .or_insert_with(|| Value::Number(now_millis().into()));

    let mut profiles = merge_profile_secrets(app, read_raw_array(app, "profiles.json")?)?;
    let profile = Value::Object(profile);
    profiles.insert(0, profile.clone());
    persist_profiles(app, &profiles)?;
    Ok(strip_secret_fields_public(&profile))
}

pub fn update_profile(app: &AppHandle, profile_id: &str, input: Value) -> Result<Value, AppError> {
    storage::migrate_legacy_data_once(app)?;
    let input = input
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Command("连接配置必须是对象".to_string()))?;
    validate_profile_input(&input)?;
    let folders = storage::read_json_array(app, "folders.json")?;
    let mut profiles = merge_profile_secrets(app, read_raw_array(app, "profiles.json")?)?;
    let index = profiles
        .iter()
        .position(|profile| profile.get("id").and_then(Value::as_str) == Some(profile_id))
        .ok_or_else(|| AppError::Storage(format!("connection profile not found: {profile_id}")))?;
    let previous = profiles[index]
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Serialization("连接配置格式无效".to_string()))?;
    let mut next = previous.clone();
    for (key, value) in input {
        let preserve_secret = matches!(key.as_str(), "password" | "passphrase" | "privateKeyPath")
            && value.as_str().is_some_and(str::is_empty);
        if key == "proxy" {
            let mut proxy = previous
                .get("proxy")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(input_proxy) = value.as_object() {
                for (proxy_key, proxy_value) in input_proxy {
                    let preserve_password =
                        proxy_key == "password" && proxy_value.as_str().is_some_and(str::is_empty);
                    if !preserve_password {
                        proxy.insert(proxy_key.clone(), proxy_value.clone());
                    }
                }
            } else {
                next.insert(key, value);
                continue;
            }
            next.insert("proxy".to_string(), Value::Object(proxy));
        } else if !preserve_secret {
            next.insert(key, value);
        }
    }
    let requested_group = next
        .get("group")
        .and_then(Value::as_str)
        .filter(|group| !group.trim().is_empty())
        .unwrap_or(DEFAULT_GROUP);
    let (group, parent_id) = resolve_group(&folders, requested_group);
    next.insert("id".to_string(), Value::String(profile_id.to_string()));
    next.insert("group".to_string(), Value::String(group));
    next.insert("parentId".to_string(), parent_id);
    for key in ["order", "lastUsedAt"] {
        if let Some(value) = previous.get(key) {
            next.insert(key.to_string(), value.clone());
        }
    }

    let profile = Value::Object(next);
    profiles[index] = profile.clone();
    persist_profiles(app, &profiles)?;
    Ok(strip_secret_fields_public(&profile))
}

pub fn delete_profile(app: &AppHandle, profile_id: &str) -> Result<(), AppError> {
    storage::migrate_legacy_data_once(app)?;
    let mut profiles = merge_profile_secrets(app, read_raw_array(app, "profiles.json")?)?;
    let previous_len = profiles.len();
    profiles.retain(|profile| profile.get("id").and_then(Value::as_str) != Some(profile_id));
    if profiles.len() == previous_len {
        return Err(AppError::Storage(format!(
            "connection profile not found: {profile_id}"
        )));
    }
    persist_profiles(app, &profiles)
}

fn heal_typed_entities(entities: &mut [Value], expected_type: &str) -> bool {
    let mut dirty = false;
    let mut order = now_millis();
    for entity in entities {
        let Some(object) = entity.as_object_mut() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some(expected_type) {
            object.insert("type".to_string(), Value::String(expected_type.to_string()));
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

fn heal_command_folders(folders: &mut [Value]) -> bool {
    heal_typed_entities(folders, "command-folder")
}

fn heal_command_templates(commands: &mut [Value]) -> bool {
    let mut dirty = heal_typed_entities(commands, "command-template");
    for command in commands {
        let Some(object) = command.as_object_mut() else {
            continue;
        };
        if object.get("command").and_then(Value::as_str).is_none() {
            object.insert("command".to_string(), Value::String(String::new()));
            dirty = true;
        }
        if object
            .get("appendCarriageReturn")
            .and_then(Value::as_bool)
            .is_none()
        {
            object.insert("appendCarriageReturn".to_string(), Value::Bool(true));
            dirty = true;
        }
    }
    dirty
}

pub fn save_command_template(
    app: &AppHandle,
    command_id: Option<&str>,
    command_text: &str,
) -> Result<Value, AppError> {
    let command_text = command_text.trim();
    if command_text.is_empty() {
        return Err(AppError::Command("命令内容不能为空".to_string()));
    }
    let (mut commands, _) = read_command_library(app)?;
    let name = command_text
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(command_id) = command_id {
        let command = commands
            .iter_mut()
            .find(|command| command.get("id").and_then(Value::as_str) == Some(command_id))
            .ok_or_else(|| {
                AppError::Storage(format!("command template not found: {command_id}"))
            })?;
        let object = command
            .as_object_mut()
            .ok_or_else(|| AppError::Serialization("命令模板格式无效".to_string()))?;
        object.insert("name".to_string(), Value::String(name));
        object.insert(
            "command".to_string(),
            Value::String(command_text.to_string()),
        );
        let saved = command.clone();
        storage::write_json_array(app, "commands.json", &commands)?;
        return Ok(saved);
    }

    let command = serde_json::json!({
        "id": format!("cmd-{}", uuid::Uuid::new_v4()),
        "type": "command-template",
        "name": name,
        "parentId": null,
        "order": now_millis(),
        "command": command_text,
        "appendCarriageReturn": true,
    });
    commands.insert(0, command.clone());
    storage::write_json_array(app, "commands.json", &commands)?;
    Ok(command)
}

pub fn delete_command_template(app: &AppHandle, command_id: &str) -> Result<(), AppError> {
    let (mut commands, _) = read_command_library(app)?;
    let before = commands.len();
    commands.retain(|command| command.get("id").and_then(Value::as_str) != Some(command_id));
    if commands.len() == before {
        return Err(AppError::Storage(format!(
            "command template not found: {command_id}"
        )));
    }
    storage::write_json_array(app, "commands.json", &commands)
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

fn validate_profile_input(profile: &Map<String, Value>) -> Result<(), AppError> {
    let required_text = |key: &str, label: &str| {
        profile
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::Command(format!("{label}不能为空")))
    };
    required_text("name", "连接名称")?;
    let protocol = required_text("type", "连接类型")?;
    if !matches!(protocol, "ssh" | "ftp" | "telnet" | "serial") {
        return Err(AppError::Command(format!("不支持的连接类型: {protocol}")));
    }
    if protocol == "serial" {
        required_text("devicePath", "串口设备路径")?;
        if profile
            .get("baudRate")
            .and_then(Value::as_u64)
            .is_some_and(|baud_rate| baud_rate == 0 || baud_rate > u32::MAX as u64)
        {
            return Err(AppError::Command("串口波特率无效".to_string()));
        }
    } else {
        required_text("host", "主机地址")?;
        if profile
            .get("port")
            .and_then(Value::as_u64)
            .is_none_or(|port| port == 0 || port > u16::MAX as u64)
        {
            return Err(AppError::Command("端口必须在 1 到 65535 之间".to_string()));
        }
    }
    Ok(())
}

fn resolve_group(folders: &[Value], requested_group: &str) -> (String, Value) {
    if requested_group == DEFAULT_GROUP {
        return (DEFAULT_GROUP.to_string(), Value::Null);
    }
    folders
        .iter()
        .find(|folder| folder.get("name").and_then(Value::as_str) == Some(requested_group))
        .and_then(|folder| folder.get("id").and_then(Value::as_str))
        .map(|id| (requested_group.to_string(), Value::String(id.to_string())))
        .unwrap_or_else(|| (DEFAULT_GROUP.to_string(), Value::Null))
}

fn persist_profiles(app: &AppHandle, profiles: &[Value]) -> Result<(), AppError> {
    let folders = storage::read_json_array(app, "folders.json")?;
    let mut profiles = profiles.to_vec();
    heal_profiles(&mut profiles, &folders);
    reconcile_profile_secrets(app, &profiles)?;
    let public_profiles = profiles.iter().map(strip_secret_fields).collect::<Vec<_>>();
    storage::write_json_array(app, "profiles.json", &public_profiles)
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

pub fn read_profiles_for_sync(app: &AppHandle) -> Result<Vec<Value>, AppError> {
    storage::migrate_legacy_data_once(app)?;
    let folders = storage::read_json_array(app, "folders.json")?;
    let mut profiles = merge_profile_secrets(app, read_raw_array(app, "profiles.json")?)?;
    if heal_profiles(&mut profiles, &folders) {
        reconcile_profile_secrets(app, &profiles)?;
        let stripped = profiles.iter().map(strip_secret_fields).collect::<Vec<_>>();
        storage::write_json_array(app, "profiles.json", &stripped)?;
    }
    Ok(profiles)
}

pub fn merge_synced_profiles(
    app: &AppHandle,
    incoming: Vec<Value>,
) -> Result<(u64, u64, u64), AppError> {
    let folders = storage::read_json_array(app, "folders.json")?;
    let mut profiles = read_profiles_for_sync(app)?;
    let mut imported = 0_u64;
    let mut updated = 0_u64;
    let mut skipped = 0_u64;

    for incoming in incoming {
        let Some(fingerprint) = profile_fingerprint(&incoming) else {
            skipped += 1;
            continue;
        };
        if let Some(index) = profiles
            .iter()
            .position(|profile| profile_fingerprint(profile).as_ref() == Some(&fingerprint))
        {
            let previous = profiles[index].as_object().cloned().unwrap_or_default();
            let Some(mut next) = incoming.as_object().cloned() else {
                skipped += 1;
                continue;
            };
            for key in ["id", "parentId", "order", "lastUsedAt"] {
                if let Some(value) = previous.get(key) {
                    next.insert(key.to_string(), value.clone());
                } else {
                    next.remove(key);
                }
            }
            profiles[index] = Value::Object(next);
            updated += 1;
        } else {
            let Some(mut next) = incoming.as_object().cloned() else {
                skipped += 1;
                continue;
            };
            next.insert(
                "id".to_string(),
                Value::String(format!("profile-{}", uuid::Uuid::new_v4())),
            );
            next.insert("order".to_string(), Value::Number(now_millis().into()));
            next.remove("parentId");
            next.remove("lastUsedAt");
            profiles.insert(0, Value::Object(next));
            imported += 1;
        }
    }

    heal_profiles(&mut profiles, &folders);
    reconcile_profile_secrets(app, &profiles)?;
    let stripped = profiles.iter().map(strip_secret_fields).collect::<Vec<_>>();
    storage::write_json_array(app, "profiles.json", &stripped)?;
    Ok((imported, updated, skipped))
}

fn profile_fingerprint(profile: &Value) -> Option<(String, String, String, u64, String)> {
    Some((
        profile.get("type")?.as_str()?.to_ascii_lowercase(),
        profile.get("name")?.as_str()?.trim().to_string(),
        profile
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        profile.get("port").and_then(Value::as_u64).unwrap_or(0),
        profile
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
    ))
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
    fn command_library_healing_matches_shared_core_shape() {
        let mut folders = vec![serde_json::json!({
            "id": "folder-1",
            "name": "运维"
        })];
        let mut commands = vec![serde_json::json!({
            "id": "command-1",
            "name": "查看日志"
        })];

        assert!(heal_command_folders(&mut folders));
        assert!(heal_command_templates(&mut commands));
        assert_eq!(folders[0]["type"], "command-folder");
        assert!(folders[0]["order"].is_number());
        assert_eq!(commands[0]["type"], "command-template");
        assert_eq!(commands[0]["command"], "");
        assert_eq!(commands[0]["appendCarriageReturn"], true);
        assert!(commands[0]["order"].is_number());

        assert!(!heal_command_folders(&mut folders));
        assert!(!heal_command_templates(&mut commands));
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

    #[test]
    fn profile_crud_persists_public_and_secret_data_separately() {
        let (app, directory) = test_app();
        storage::write_json_array(
            &app,
            "folders.json",
            &[serde_json::json!({ "id": "folder-prod", "name": "生产" })],
        )
        .unwrap();

        let created = create_profile(
            &app,
            serde_json::json!({
                "id": "caller-controlled-id",
                "type": "ssh",
                "name": "primary",
                "host": "example.test",
                "port": 22,
                "username": "root",
                "group": "生产",
                "password": "first-secret",
                "proxy": { "type": "http", "password": "proxy-secret" }
            }),
        )
        .unwrap();
        let profile_id = created["id"].as_str().unwrap().to_string();
        assert_ne!(profile_id, "caller-controlled-id");
        assert_eq!(created["parentId"], "folder-prod");
        assert_eq!(created["hasSavedPassword"], true);
        assert!(created.get("password").is_none());

        let public_profiles = read_raw_array(&app, "profiles.json").unwrap();
        assert!(public_profiles[0].get("password").is_none());
        assert!(public_profiles[0]["proxy"].get("password").is_none());
        let stored = read_connection_profile(&app, &profile_id).unwrap();
        assert_eq!(stored["password"], "first-secret");
        assert_eq!(stored["proxy"]["password"], "proxy-secret");

        let updated = update_profile(
            &app,
            &profile_id,
            serde_json::json!({
                "id": "replacement-id",
                "type": "ssh",
                "name": "renamed",
                "host": "example.test",
                "port": 2222,
                "username": "root",
                "group": "missing-folder",
                "password": ""
            }),
        )
        .unwrap();
        assert_eq!(updated["id"], profile_id);
        assert_eq!(updated["group"], DEFAULT_GROUP);
        assert!(updated["parentId"].is_null());
        assert_eq!(
            read_connection_profile(&app, &profile_id).unwrap()["password"],
            "first-secret"
        );

        delete_profile(&app, &profile_id).unwrap();
        assert!(read_raw_array(&app, "profiles.json").unwrap().is_empty());
        let secrets = storage::read_json_object(&app, "profile-secrets.json").unwrap();
        assert!(secrets["profiles"].as_object().unwrap().is_empty());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_profile_does_not_modify_existing_storage() {
        let (app, directory) = test_app();
        let existing = vec![serde_json::json!({
            "id": "existing",
            "type": "ssh",
            "name": "existing",
            "host": "example.test",
            "port": 22,
            "group": DEFAULT_GROUP,
            "parentId": null
        })];
        storage::write_json_array(&app, "profiles.json", &existing).unwrap();
        let before = fs::read(storage::workspace_file(&app, "profiles.json").unwrap()).unwrap();

        let result = create_profile(
            &app,
            serde_json::json!({
                "type": "ssh",
                "name": "invalid",
                "host": "",
                "port": 22
            }),
        );
        assert!(result.is_err());
        let after = fs::read(storage::workspace_file(&app, "profiles.json").unwrap()).unwrap();
        assert_eq!(after, before);
        let _ = fs::remove_dir_all(directory);
    }

    fn test_app() -> (AppHandle, std::path::PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-profile-ops-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        (AppHandle::new(directory.clone()), directory)
    }
}
