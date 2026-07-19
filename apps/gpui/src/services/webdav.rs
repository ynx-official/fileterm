use std::{fs, path::PathBuf, time::Duration};

use reqwest::{
    header::{HeaderMap, ETAG, IF_MATCH, IF_NONE_MATCH},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    backend::{storage, AppHandle},
    error::{AppError, Result},
    services::profile_ops,
};

const MAX_BUNDLE_BYTES: usize = 5 * 1024 * 1024;
const DEFAULT_REMOTE_PATH: &str = "fileterm-connections.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    enabled: bool,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    remote_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_insecure_tls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_synced_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
}

impl Default for StoredConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            username: None,
            remote_path: DEFAULT_REMOTE_PATH.to_string(),
            allow_insecure_tls: None,
            password: None,
            last_synced_at: None,
            last_etag: None,
            content_hash: None,
        }
    }
}

fn command_error(message: impl Into<String>) -> AppError {
    AppError::Command(message.into())
}

fn config_path(app: &AppHandle) -> Result<PathBuf> {
    storage::workspace_file(app, "webdav-sync.json")
}

fn normalize_base_url(value: &str, allow_insecure_tls: bool) -> Result<String> {
    let mut url = Url::parse(value.trim()).map_err(|_| command_error("WebDAV 地址无效"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(command_error("WebDAV 地址不得内嵌用户名或密码"));
    }
    if url.fragment().is_some() {
        return Err(command_error("WebDAV 地址不得包含片段"));
    }
    if url.scheme() != "https" && !(allow_insecure_tls && url.scheme() == "http") {
        return Err(command_error(
            "WebDAV 地址必须使用 HTTPS；HTTP 需要明确启用高风险选项。",
        ));
    }
    url.set_query(None);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn normalize_remote_path(value: &str) -> Result<String> {
    let path = value.trim().trim_start_matches('/');
    if path.is_empty()
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(command_error("WebDAV 远端路径无效"));
    }
    Ok(path.to_string())
}

fn remote_url(config: &StoredConfig) -> Result<Url> {
    let base = normalize_base_url(&config.url, config.allow_insecure_tls == Some(true))?;
    Url::parse(&(base + "/"))
        .map_err(|error| command_error(error.to_string()))?
        .join(&normalize_remote_path(&config.remote_path)?)
        .map_err(|error| command_error(format!("WebDAV 远端路径无效: {error}")))
}

fn read_config(app: &AppHandle) -> Result<StoredConfig> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(StoredConfig::default());
    }
    lock_down_file(&path)?;
    let content = fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
    let mut config: StoredConfig = serde_json::from_str(&content)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    if config.remote_path.trim().is_empty() {
        config.remote_path = DEFAULT_REMOTE_PATH.to_string();
    }
    Ok(config)
}

fn write_config(app: &AppHandle, config: &StoredConfig) -> Result<()> {
    let path = config_path(app)?;
    let temporary = path.with_file_name(format!(".webdav-sync.{}.tmp", uuid::Uuid::new_v4()));
    let content = serde_json::to_vec_pretty(config)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    storage::write_restricted_file(&temporary, &content)?;
    storage::replace_file_atomically(&temporary, &path)?;
    lock_down_file(&path)
}

#[cfg(unix)]
fn lock_down_file(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::Storage(error.to_string()))
}

#[cfg(not(unix))]
fn lock_down_file(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

fn public_config(config: &StoredConfig) -> Value {
    serde_json::json!({
        "enabled": config.enabled,
        "url": config.url,
        "username": config.username,
        "remotePath": config.remote_path,
        "allowInsecureTls": config.allow_insecure_tls == Some(true),
        "hasSavedPassword": config.password.as_deref().is_some_and(|password| !password.is_empty()),
        "lastSyncedAt": config.last_synced_at,
        "lastEtag": config.last_etag,
    })
}

pub fn get_config(app: &AppHandle) -> Result<Value> {
    Ok(public_config(&read_config(app)?))
}

pub fn save_config(app: &AppHandle, input: Value) -> Result<Value> {
    let previous = read_config(app)?;
    let enabled = input
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(previous.enabled);
    let allow_insecure_tls = input
        .get("allowInsecureTls")
        .and_then(Value::as_bool)
        .unwrap_or(previous.allow_insecure_tls == Some(true));
    let url = input
        .get("url")
        .and_then(Value::as_str)
        .map(|value| normalize_base_url(value, allow_insecure_tls))
        .transpose()?
        .unwrap_or(previous.url);
    let remote_path = input
        .get("remotePath")
        .and_then(Value::as_str)
        .map(normalize_remote_path)
        .transpose()?
        .unwrap_or(previous.remote_path);
    let username = string_update(input.get("username"), previous.username, true);
    let password = string_update(input.get("password"), previous.password, false);
    let config = StoredConfig {
        enabled,
        url,
        username,
        remote_path,
        allow_insecure_tls: allow_insecure_tls.then_some(true),
        password,
        last_synced_at: previous.last_synced_at,
        last_etag: previous.last_etag,
        content_hash: previous.content_hash,
    };
    if config.enabled && config.url.is_empty() {
        return Err(command_error("启用 WebDAV 同步时必须填写地址"));
    }
    write_config(app, &config)?;
    Ok(public_config(&config))
}

fn string_update(input: Option<&Value>, previous: Option<String>, trim: bool) -> Option<String> {
    match input {
        Some(Value::String(value)) => {
            let value = if trim { value.trim() } else { value.as_str() };
            (!value.is_empty()).then(|| value.to_string())
        }
        Some(Value::Null) => None,
        _ => previous,
    }
}

fn configured(app: &AppHandle) -> Result<StoredConfig> {
    let config = read_config(app)?;
    if !config.enabled {
        return Err(command_error("请先启用 WebDAV 配置同步"));
    }
    normalize_base_url(&config.url, config.allow_insecure_tls == Some(true))?;
    normalize_remote_path(&config.remote_path)?;
    Ok(config)
}

fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| command_error(format!("无法初始化 WebDAV 客户端: {error}")))
}

fn authenticated(
    request: reqwest::RequestBuilder,
    config: &StoredConfig,
) -> reqwest::RequestBuilder {
    match config.username.as_deref() {
        Some(username) => request.basic_auth(username, config.password.as_deref()),
        None => request,
    }
}

fn etag(headers: &HeaderMap) -> Option<String> {
    headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn response_error(action: &str, status: StatusCode) -> AppError {
    command_error(format!("WebDAV {action}失败 ({status})"))
}

pub async fn upload(app: &AppHandle) -> Result<Value> {
    let mut config = configured(app)?;
    let profiles = profile_ops::read_profiles_for_sync(app)?;
    let canonical = serde_json::to_vec(&profiles)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    let content_hash = sha256_hex(&canonical);
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": 2,
        "containsSecrets": true,
        "generatedAt": timestamp(),
        "contentHash": content_hash,
        "profiles": profiles,
    }))
    .map_err(|error| AppError::Serialization(error.to_string()))?;
    let client = client()?;
    config.last_etag = upload_payload(&client, &config, payload).await?;
    config.last_synced_at = Some(timestamp());
    config.content_hash = Some(content_hash);
    write_config(app, &config)?;
    Ok(serde_json::json!({ "action": "upload", "message": "连接配置已上传到 WebDAV。" }))
}

async fn upload_payload(
    client: &Client,
    config: &StoredConfig,
    payload: Vec<u8>,
) -> Result<Option<String>> {
    let remote = remote_url(config)?;
    let head = authenticated(client.head(remote.clone()), config)
        .send()
        .await
        .map_err(|error| command_error(format!("WebDAV 预检失败: {error}")))?;
    let remote_exists = head.status().is_success();
    if !remote_exists && head.status() != StatusCode::NOT_FOUND {
        return Err(response_error("预检", head.status()));
    }
    let remote_etag = etag(head.headers());
    if remote_exists && config.last_etag.is_none() {
        return Err(command_error(
            "远端已存在配置包。请先下载并确认内容，再上传以避免首次同步覆盖。",
        ));
    }
    if config
        .last_etag
        .as_deref()
        .is_some_and(|last| remote_etag.as_deref() != Some(last))
    {
        return Err(command_error(
            "远端配置自上次同步后已变更。请先下载并确认冲突，再上传。",
        ));
    }
    let request = authenticated(
        client
            .put(remote)
            .header("content-type", "application/json; charset=utf-8")
            .body(payload),
        config,
    );
    let request = match remote_etag.as_deref() {
        Some(value) => request.header(IF_MATCH, value),
        None => request.header(IF_NONE_MATCH, "*"),
    };
    let response = request
        .send()
        .await
        .map_err(|error| command_error(format!("WebDAV 上传失败: {error}")))?;
    if response.status() == StatusCode::PRECONDITION_FAILED {
        return Err(command_error("WebDAV ETag 冲突：远端文件已被其他设备修改"));
    }
    if !response.status().is_success() {
        return Err(response_error("上传", response.status()));
    }
    Ok(etag(response.headers()).or(remote_etag))
}

pub async fn download(app: &AppHandle) -> Result<Value> {
    let mut config = configured(app)?;
    let response = authenticated(client()?.get(remote_url(&config)?), &config)
        .send()
        .await
        .map_err(|error| command_error(format!("WebDAV 下载失败: {error}")))?;
    if !response.status().is_success() {
        return Err(response_error("下载", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size as usize > MAX_BUNDLE_BYTES)
    {
        return Err(command_error("WebDAV 配置包超过 5 MB 限制"));
    }
    let remote_etag = etag(response.headers());
    let bytes = response
        .bytes()
        .await
        .map_err(|error| command_error(format!("WebDAV 下载内容失败: {error}")))?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(command_error("WebDAV 配置包超过 5 MB 限制"));
    }
    let profiles = parse_bundle(&bytes)?;
    let (imported, updated, skipped) = profile_ops::merge_synced_profiles(app, profiles)?;
    config.last_etag = remote_etag;
    config.last_synced_at = Some(timestamp());
    config.content_hash = Some(sha256_hex(&bytes));
    write_config(app, &config)?;
    Ok(serde_json::json!({
        "action": "download",
        "message": format!("已从 WebDAV 导入 {imported} 个连接，更新 {updated} 个现有连接；跳过 {skipped} 个无效项。"),
        "imported": imported,
        "updated": updated,
        "skipped": skipped,
    }))
}

fn parse_bundle(bytes: &[u8]) -> Result<Vec<Value>> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| command_error(format!("WebDAV 配置包无效: {error}")))?;
    let profiles = match &value {
        Value::Array(items) => items.clone(),
        Value::Object(object) => object
            .get("profiles")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| command_error("WebDAV 配置包缺少 profiles"))?,
        _ => return Err(command_error("WebDAV 配置包格式无效")),
    };
    if let Some(expected) = value.get("contentHash").and_then(Value::as_str) {
        let canonical = serde_json::to_vec(&profiles)
            .map_err(|error| AppError::Serialization(error.to_string()))?;
        if sha256_hex(&canonical) != expected {
            return Err(command_error(
                "WebDAV 配置包 hash 校验失败，文件可能已损坏或被篡改",
            ));
        }
    }
    Ok(profiles.into_iter().filter_map(sanitize_profile).collect())
}

fn sanitize_profile(value: Value) -> Option<Value> {
    let mut object = value.as_object()?.clone();
    let kind = object.get("type")?.as_str()?.to_ascii_lowercase();
    if !matches!(kind.as_str(), "ssh" | "ftp" | "telnet" | "serial") {
        return None;
    }
    if object
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(|name| name.trim().is_empty())
    {
        return None;
    }
    if kind == "serial" {
        if object
            .get("devicePath")
            .and_then(Value::as_str)
            .is_none_or(|path| path.trim().is_empty())
        {
            return None;
        }
    } else {
        let host_valid = object
            .get("host")
            .and_then(Value::as_str)
            .is_some_and(|host| !host.trim().is_empty());
        let port_valid = object
            .get("port")
            .and_then(Value::as_u64)
            .is_some_and(|port| (1..=65535).contains(&port));
        if !host_valid || !port_valid {
            return None;
        }
    }
    object.insert("type".to_string(), Value::String(kind));
    object
        .entry("group".to_string())
        .or_insert_with(|| Value::String("默认".to_string()));
    object
        .entry("remotePath".to_string())
        .or_insert_with(|| Value::String("/".to_string()));
    object
        .entry("username".to_string())
        .or_insert_with(|| Value::String(String::new()));
    for key in ["id", "parentId", "order", "lastUsedAt"] {
        object.remove(key);
    }
    Some(Value::Object(object))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy_is_fail_closed() {
        assert!(normalize_base_url("http://example.test/dav", false).is_err());
        assert!(normalize_base_url("https://user:pass@example.test/dav", false).is_err());
        assert!(normalize_base_url("https://example.test/dav", false).is_ok());
    }

    #[test]
    fn bundle_hash_detects_tampering() {
        let profiles = vec![serde_json::json!({
            "type": "ssh", "name": "server", "host": "example.test", "port": 22
        })];
        let canonical = serde_json::to_vec(&profiles).unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "contentHash": sha256_hex(&canonical), "profiles": profiles
        }))
        .unwrap();
        assert_eq!(parse_bundle(&bytes).unwrap().len(), 1);
        let tampered = serde_json::to_vec(&serde_json::json!({
            "contentHash": "deadbeef", "profiles": []
        }))
        .unwrap();
        assert!(parse_bundle(&tampered).is_err());
    }
}
