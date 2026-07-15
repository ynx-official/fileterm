use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use crate::AppError;

pub fn state_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::Storage(error.to_string()))?;
    fs::create_dir_all(&dir).map_err(|error| AppError::Storage(error.to_string()))?;
    Ok(dir.join("ui-preferences.json"))
}

pub fn workspace_file(app: &AppHandle, name: &str) -> Result<PathBuf, AppError> {
    Ok(state_path(app)?.with_file_name(name))
}

pub fn read_json_object(app: &AppHandle, name: &str) -> Result<serde_json::Value, AppError> {
    let path = workspace_file(app, name)?;
    let mut value: serde_json::Value = if path.exists() {
        let content =
            fs::read_to_string(&path).map_err(|error| AppError::Storage(error.to_string()))?;
        serde_json::from_str(&content)
            .map_err(|error| AppError::Serialization(error.to_string()))?
    } else {
        serde_json::json!({})
    };

    // Electron stored the same JSON collections under the product-name directory.
    // Read it as a compatibility source while the Rust store is being migrated.
    if let Some(app_data_dir) = path.parent() {
        if let Some(parent_dir) = app_data_dir.parent() {
            let legacy_path = parent_dir.join("FileTerm").join(name);
            if legacy_path.exists() {
                if let Ok(content) = fs::read_to_string(legacy_path) {
                    if let Ok(legacy_value) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let (Some(obj), Some(legacy_obj)) = (value.as_object_mut(), legacy_value.as_object()) {
                            for (k, v) in legacy_obj {
                                if !obj.contains_key(k) {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(value)
}

pub fn read_json_array(app: &AppHandle, name: &str) -> Result<Vec<serde_json::Value>, AppError> {
    let path = workspace_file(app, name)?;
    let mut values: Vec<serde_json::Value> = if path.exists() {
        let content =
            fs::read_to_string(&path).map_err(|error| AppError::Storage(error.to_string()))?;
        serde_json::from_str(&content)
            .map_err(|error| AppError::Serialization(error.to_string()))?
    } else {
        Vec::new()
    };

    // Electron stored the same JSON collections under the product-name directory.
    // Read it as a compatibility source while the Rust store is being migrated.
    if let Some(app_data_dir) = path.parent() {
        if let Some(parent_dir) = app_data_dir.parent() {
            let legacy_path = parent_dir.join("FileTerm").join(name);
            if legacy_path.exists() {
                let content = fs::read_to_string(legacy_path)
                    .map_err(|error| AppError::Storage(error.to_string()))?;
                let legacy_values: Vec<serde_json::Value> = serde_json::from_str(&content)
                    .map_err(|error| AppError::Serialization(error.to_string()))?;
                let known_ids: std::collections::HashSet<String> = values
                    .iter()
                    .filter_map(|value| value.get("id").and_then(|id| id.as_str()))
                    .map(ToOwned::to_owned)
                    .collect();
                values.extend(legacy_values.into_iter().filter(|value| {
                    value
                        .get("id")
                        .and_then(|id| id.as_str())
                        .map(|id| !known_ids.contains(id))
                        .unwrap_or(true)
                }));
            }
        }
    }

    if name == "profiles.json" {
        if let Ok(secrets) = read_json_object(app, "profile-secrets.json") {
            if let Some(secrets_profiles) = secrets.get("profiles").and_then(|p| p.as_object()) {
                for profile in values.iter_mut() {
                    if let Some(profile_obj) = profile.as_object_mut() {
                        if let Some(profile_id) = profile_obj.get("id").and_then(|id| id.as_str()) {
                            if let Some(profile_secrets) = secrets_profiles.get(profile_id).and_then(|ps| ps.as_object()) {
                                if let Some(password) = profile_secrets.get("password").and_then(|p| p.get("value")).and_then(|v| v.as_str()) {
                                    profile_obj.insert("password".to_string(), serde_json::json!(password));
                                }
                                if let Some(passphrase) = profile_secrets.get("passphrase").and_then(|p| p.get("value")).and_then(|v| v.as_str()) {
                                    profile_obj.insert("passphrase".to_string(), serde_json::json!(passphrase));
                                }
                                if let Some(pkey_path) = profile_secrets.get("privateKeyPath").and_then(|p| p.get("value")).and_then(|v| v.as_str()) {
                                    profile_obj.insert("privateKeyPath".to_string(), serde_json::json!(pkey_path));
                                }
                                if let Some(proxy_pass) = profile_secrets.get("proxyPassword").and_then(|p| p.get("value")).and_then(|v| v.as_str()) {
                                    if let Some(proxy_obj) = profile_obj.get_mut("proxy").and_then(|p| p.as_object_mut()) {
                                        proxy_obj.insert("password".to_string(), serde_json::json!(proxy_pass));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(values)
}

pub fn write_json_array(
    app: &AppHandle,
    name: &str,
    values: &[serde_json::Value],
) -> Result<(), AppError> {
    let path = workspace_file(app, name)?;
    let temp_path = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(values)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    fs::write(&temp_path, content).map_err(|error| AppError::Storage(error.to_string()))?;
    fs::rename(temp_path, path).map_err(|error| AppError::Storage(error.to_string()))
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}
