//! Commands module forked from `apps/tauri/src-tauri/src/commands/mod.rs` for
//! the GPUI runtime's G1 phase (see `docs/plans/active/gpui-refactor.md`
//! section 6.2).
//!
//! G1 migration status:
//! - Type definitions (structs/enums) are kept line-for-line so the
//!   renderer-side IPC contract (camelCase payload shapes, deny_unknown_fields
//!   guards) is preserved.
//! - Storage-only commands (UI preferences, UI state, command history,
//!   command-send preferences, permission parsing, command-template
//!   rendering, platform/arch detection, atomic JSON writes) are migrated
//!   fully: they only need `&AppHandle` + `crate::backend::storage` + std +
//!   `regex`, all available in G1.
//! - Commands that reference `WorkspaceState` / `WorkerCmd` / `WebviewWindow`
//!   / `Emitter` / `ipc::Channel` / `services::*` / `crate::sessions::*` /
//!   `tauri::async_runtime` are stubbed with
//!   `Err(AppError::Unsupported("G2/G3/G4: ..."))` and a
//!   `// TODO(G?): migrate original body from Tauri` comment. Their
//!   signatures are preserved (with framework/unmigrated-type params removed)
//!   so G2–G5 can fill in the bodies without reshaping the API surface.
//! - `pub mod workspace_window;` is preserved (already migrated separately).
//!
//! Original: `apps/tauri/src-tauri/src/commands/mod.rs`.
//!
//! Most of this module is intentionally stub code waiting on G2–G5 backends,
//! so unused-variable / dead-code lints are silenced at the module level to
//! keep the G1 fork readable. Once a stub gains a real body (and the params it
//! consumes), the module-level allow can be tightened.

#![allow(unused_variables, dead_code)]

use crate::backend::app_handle::AppHandle;
use crate::error::AppError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub mod workspace_window;

/// 等待 worker 接收命令的最大时间。worker 主循环被 SFTP init / shell
/// channel 写阻塞 时，mpsc 一旦满，send 会永久 await，导致前端 invoke
/// 链路整体卡死（多窗口发送后续 tab 全部排队、Cmd+Q 退出无法完成）。
/// 超时后返回显式 busy 错误，绝不静默吞掉输入。SSH 终端输入已经走
/// 独立 channel；这里仍作为 Telnet / Serial 和通用 worker 命令的保护。
const WORKER_CMD_SEND_TIMEOUT: Duration = Duration::from_millis(500);

/// 文件/会话级操作（list/read/write/重连等）容忍更长延迟，但同样不能
/// 永久阻塞——一旦 worker 卡死，应当让前端拿到明确错误。
const WORKER_FILE_CMD_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Worker 已接收命令后也必须在有限时间内答复。之前仅限制了 mpsc send，
/// 但某个后台 SFTP/exec task 丢失 reply 时，oneshot 会一直 await，导致
/// 删除、打开目录和 Root 弹窗永久 loading。
const WORKER_FILE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

/// 退出时给 worker 的 Disconnect 命令留 1 秒，超时直接放弃发送：worker
/// 主循环卡死时 channel 满，send 不进去；强行 await 会让 Cmd+Q 整个
/// 退出链路 hang 住，用户只能强制杀进程。drop sender 后 worker 的
/// `cmd_rx.recv()` 会返回 None，自然走清理路径。
const WORKER_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Let a child-window close command resolve its IPC callback before destroying
/// the calling WebView. Destroying synchronously makes WebView2 report a
/// missing callback id and can leave renderer cleanup half-finished.
const CHILD_WINDOW_DESTROY_DELAY: Duration = Duration::from_millis(25);

// TODO(G3): migrate original body from Tauri
async fn send_terminal_input(tab_id: &str, data: String) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct UiPreferences {
    pub theme: String,
    pub locale: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct UiPreferencesInput {
    pub theme: Option<String>,
    pub locale: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCommandHistoryEntry {
    pub command: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSendPreferences {
    pub remember_selection: bool,
    pub send_scope: String,
    pub selected_tab_ids: Vec<String>,
}

impl Default for CommandSendPreferences {
    fn default() -> Self {
        Self {
            remember_selection: false,
            send_scope: "current".to_string(),
            selected_tab_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSshKeyInput {
    pub source_path: Option<String>,
    pub note: Option<String>,
}

fn write_json_object(app: &AppHandle, name: &str, value: &Value) -> Result<(), AppError> {
    let path = crate::backend::storage::workspace_file(app, name)?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    std::fs::write(&temporary, content).map_err(|error| AppError::Storage(error.to_string()))?;
    crate::backend::storage::replace_file_atomically(&temporary, &path)
}

pub fn app_get_platform() -> String {
    std::env::consts::OS.to_string()
}

fn canonical_arch(arch: &str) -> String {
    match arch {
        "aarch64" => "arm64".to_string(),
        "x86_64" => "x64".to_string(),
        other => other.to_string(),
    }
}

fn resolve_native_arch(platform: &str, process_arch: &str, macos_arm64_capable: bool) -> String {
    if platform == "macos" && macos_arm64_capable {
        return "arm64".to_string();
    }

    canonical_arch(process_arch)
}

#[cfg(target_os = "macos")]
fn macos_arm64_capable() -> bool {
    std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.optional.arm64"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|value| value.trim() == "1")
}

#[cfg(not(target_os = "macos"))]
fn macos_arm64_capable() -> bool {
    false
}

pub fn app_get_arch() -> String {
    resolve_native_arch(
        std::env::consts::OS,
        std::env::consts::ARCH,
        macos_arm64_capable(),
    )
}

// TODO(G2): migrate original body from Tauri (original returned
// `tauri::VERSION.to_string()`; GPUI has no framework version string yet).
pub fn app_get_runtime_version() -> String {
    "gpui".to_string()
}

pub fn app_read_clipboard_text() -> Result<String, AppError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .get_text()
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

pub fn app_write_clipboard_text(text: String) -> Result<(), AppError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

pub fn app_open_external_url(url: String) -> Result<(), AppError> {
    let parsed = validate_external_url(&url)?;
    open::that(parsed.as_str()).map_err(|error| AppError::Command(error.to_string()))
}

fn validate_external_url(url: &str) -> Result<url::Url, AppError> {
    let parsed = url::Url::parse(url)
        .map_err(|error| AppError::Command(format!("外部链接无效: {error}")))?;
    if matches!(parsed.scheme(), "http" | "https") {
        Ok(parsed)
    } else {
        Err(AppError::Command(
            "仅允许打开 http 或 https 外部链接".to_string(),
        ))
    }
}

// TODO(G2): migrate original body from Tauri (needs `services::updates`).
pub async fn app_get_update_status(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G2: needs update service"))
}

// TODO(G2): migrate original body from Tauri (needs `services::updates`).
pub async fn app_check_for_updates(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G2: needs update service"))
}

// TODO(G2): migrate original body from Tauri (needs `services::updates`).
pub async fn app_download_update(app: &AppHandle) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs update service"))
}

// TODO(G2): migrate original body from Tauri (needs `services::updates`).
pub async fn app_install_update(app: &AppHandle) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs update service"))
}

pub fn app_open_logs_directory(app: &AppHandle) -> Result<(), AppError> {
    let log_directory = crate::backend::storage::state_path(app)?.with_file_name("logs");
    std::fs::create_dir_all(&log_directory)
        .map_err(|error| AppError::Storage(error.to_string()))?;
    open::that(log_directory).map_err(|error| AppError::Command(error.to_string()))
}

pub fn app_get_ui_preferences(app: &AppHandle) -> Result<UiPreferences, AppError> {
    let path = crate::backend::storage::state_path(app)?;
    if path.exists() {
        let content =
            std::fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
        let preferences: UiPreferences = serde_json::from_str(&content)
            .map_err(|error| AppError::Serialization(error.to_string()))?;
        Ok(preferences)
    } else {
        Ok(UiPreferences {
            theme: "default-dark".to_string(),
            locale: "zhCN".to_string(),
        })
    }
}

pub fn app_set_ui_preferences(
    app: &AppHandle,
    input: UiPreferencesInput,
) -> Result<UiPreferences, AppError> {
    let current = app_get_ui_preferences(app)?;
    let theme = input.theme.unwrap_or(current.theme);
    let locale = input.locale.unwrap_or(current.locale);
    if !matches!(theme.as_str(), "default-dark" | "default-light") {
        return Err(AppError::Command("主题设置无效".to_string()));
    }
    if !matches!(locale.as_str(), "zhCN" | "enUS") {
        return Err(AppError::Command("语言设置无效".to_string()));
    }
    let preferences = UiPreferences { theme, locale };
    let value = serde_json::to_value(&preferences)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    write_json_object(app, "ui-preferences.json", &value)?;
    Ok(preferences)
}

fn normalize_ui_state(value: Value) -> Result<serde_json::Map<String, Value>, AppError> {
    match value {
        Value::Object(mut object) => {
            let mut states = object
                .remove("values")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            object.remove("version");
            states.extend(object);
            Ok(states)
        }
        Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|item| {
                let key = item.get("key")?.as_str()?.to_string();
                let value = item.get("value")?.clone();
                Some((key, value))
            })
            .collect()),
        _ => Err(AppError::Serialization("UI 状态文件格式无效".to_string())),
    }
}

fn read_ui_state(app: &AppHandle) -> Result<serde_json::Map<String, Value>, AppError> {
    normalize_ui_state(crate::backend::storage::read_json_object(
        app,
        "ui-state.json",
    )?)
}

fn write_ui_state(app: &AppHandle, states: serde_json::Map<String, Value>) -> Result<(), AppError> {
    write_json_object(app, "ui-state.json", &Value::Object(states))
}

pub fn app_get_ui_state_item(app: &AppHandle, key: String) -> Result<Option<String>, AppError> {
    Ok(read_ui_state(app)?
        .get(&key)
        .and_then(Value::as_str)
        .map(ToString::to_string))
}

pub fn app_set_ui_state_item(app: &AppHandle, key: String, value: String) -> Result<(), AppError> {
    let mut states = read_ui_state(app)?;
    states.insert(key, Value::String(value));
    write_ui_state(app, states)
}

pub fn app_remove_ui_state_item(app: &AppHandle, key: String) -> Result<(), AppError> {
    let mut states = read_ui_state(app)?;
    states.remove(&key);
    write_ui_state(app, states)
}

pub fn app_get_terminal_command_history(
    app: &AppHandle,
    profile_id: String,
) -> Result<Vec<TerminalCommandHistoryEntry>, AppError> {
    let value = crate::backend::storage::read_json_object(app, "command-history.json")?;
    Ok(value
        .get(&profile_id)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<TerminalCommandHistoryEntry>(entry).ok())
        .filter(|entry| !entry.command.trim().is_empty())
        .collect())
}

pub fn app_set_terminal_command_history(
    app: &AppHandle,
    profile_id: String,
    entries: Vec<TerminalCommandHistoryEntry>,
) -> Result<(), AppError> {
    let mut value = crate::backend::storage::read_json_object(app, "command-history.json")?;
    let sanitized = entries
        .into_iter()
        .filter(|entry| !entry.command.trim().is_empty())
        .take(500)
        .collect::<Vec<_>>();
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::Serialization("命令历史文件格式无效".to_string()))?;
    object.insert(
        profile_id,
        serde_json::to_value(sanitized)
            .map_err(|error| AppError::Serialization(error.to_string()))?,
    );
    write_json_object(app, "command-history.json", &value)
}

pub fn app_get_command_send_preferences(
    app: &AppHandle,
) -> Result<CommandSendPreferences, AppError> {
    let value = crate::backend::storage::read_json_object(app, "command-send-preferences.json")?;
    let preferences = serde_json::from_value::<CommandSendPreferences>(value).unwrap_or_default();
    Ok(CommandSendPreferences {
        send_scope: match preferences.send_scope.as_str() {
            "current" | "all-ssh" | "selected-ssh" => preferences.send_scope,
            _ => "current".to_string(),
        },
        ..preferences
    })
}

pub fn app_set_command_send_preferences(
    app: &AppHandle,
    preferences: CommandSendPreferences,
) -> Result<(), AppError> {
    if !matches!(
        preferences.send_scope.as_str(),
        "current" | "all-ssh" | "selected-ssh"
    ) {
        return Err(AppError::Command("命令发送范围无效".to_string()));
    }
    let selected_tab_ids = preferences
        .selected_tab_ids
        .into_iter()
        .filter(|id| !id.trim().is_empty())
        .take(200)
        .collect::<Vec<_>>();
    write_json_object(
        app,
        "command-send-preferences.json",
        &serde_json::to_value(CommandSendPreferences {
            selected_tab_ids,
            ..preferences
        })
        .map_err(|error| AppError::Serialization(error.to_string()))?,
    )
}

// TODO(G3): migrate original body from Tauri (needs `services::transfers`
// hydration + `WorkspaceState::library_mutation`).
async fn lock_library_after_transfer_hydration(
    app: &AppHandle,
) -> Result<tokio::sync::OwnedMutexGuard<()>, AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState/transfers"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState`).
pub async fn app_get_snapshot(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState"))
}

pub async fn app_get_connection_library(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    let (profiles, folders) = crate::services::profile_ops::read_public_connection_library(app)?;
    Ok(serde_json::json!({
        "profiles": profiles,
        "folders": folders,
    }))
}

// TODO(G3): migrate original body from Tauri (needs `services::ssh_keys`).
pub fn app_list_ssh_keys(app: &AppHandle) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs ssh_keys service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::ssh_keys`).
pub async fn app_select_ssh_key_file(
    app: &AppHandle,
) -> Result<Option<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs ssh_keys service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::ssh_keys` +
// `emit_ssh_keys_changed`).
pub fn app_import_ssh_key(
    app: &AppHandle,
    input: Option<ImportSshKeyInput>,
) -> Result<Option<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs ssh_keys service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::ssh_keys` +
// `emit_ssh_keys_changed`).
pub fn app_update_ssh_key_note(
    app: &AppHandle,
    key_id: String,
    note: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs ssh_keys service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::ssh_keys` +
// `emit_ssh_keys_changed`).
pub fn app_delete_ssh_key(app: &AppHandle, key_id: String) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs ssh_keys service"))
}

// TODO(G3): migrate original body from Tauri (needs `app.emit` +
// `services::ssh_keys`).
fn emit_ssh_keys_changed(app: &AppHandle) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs event emitter"))
}

// TODO(G3): migrate original body from Tauri (needs `rfd` +
// `services::connections`).
pub async fn app_preview_connection_import(
    app: &AppHandle,
    source: Option<String>,
) -> Result<Option<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs connections service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::connections`).
pub async fn app_commit_connection_json_import(
    app: &AppHandle,
    plan_id: String,
    options: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs connections service"))
}

// TODO(G3): migrate original body from Tauri (needs `rfd` +
// `services::connections`).
pub async fn app_export_connections(app: &AppHandle, format: String) -> Result<bool, AppError> {
    Err(AppError::Unsupported("G3: needs connections service"))
}

// TODO(G3): migrate original body from Tauri (needs `rfd` +
// `services::profile_ops` + `services::connections` + `services::webdav`).
pub async fn app_export_connections_as_files(
    app: &AppHandle,
    format: String,
) -> Result<bool, AppError> {
    Err(AppError::Unsupported("G3: needs connections service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::webdav`).
pub fn app_get_webdav_sync_config(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs webdav service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::webdav`).
pub fn app_set_webdav_sync_config(
    app: &AppHandle,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs webdav service"))
}

// TODO(G3): migrate original body from Tauri (needs `services::webdav`).
pub async fn app_upload_webdav_sync(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs webdav service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::webdav` + `app.emit`).
pub async fn app_download_webdav_sync(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs webdav service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_workspace_mutation(
    app: &AppHandle,
    operation: String,
    payload: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenWindowInput {
    pub kind: String,
    pub mode: Option<String>,
    pub profile_id: Option<String>,
    pub command_id: Option<String>,
    pub folder_id: Option<String>,
    pub source: Option<String>,
    pub path: Option<String>,
    pub name: Option<String>,
    pub tab_id: Option<String>,
    pub encoding: Option<String>,
}

// TODO(G2): migrate original body from Tauri (needs WindowRegistry +
// `crate::open_child_window`).
pub async fn app_open_window(app: &AppHandle, input: OpenWindowInput) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

fn renderer_approved_close_should_destroy(window_label: &str) -> bool {
    window_label != "main"
}

/// detached-session 窗口的 label 前缀。这些窗口持有会话标签，
/// 关闭前需要 renderer 弹确认对话框（与主窗口一致），不能直接 destroy。
fn is_detached_session_label(window_label: &str) -> bool {
    window_label.starts_with("detached-")
}

// TODO(G2): migrate original body from Tauri (needs `WebviewWindow` +
// `tauri::async_runtime`).
fn destroy_child_window_after_invoke_reply() {
    // TODO(G2): migrate original body from Tauri
}

// TODO(G2): migrate original body from Tauri (needs `WebviewWindow` +
// `app.emit` + `services::transfers` + window lifecycle helpers).
pub async fn app_window_action(app: &AppHandle, action: String) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

// TODO(G2): migrate original body from Tauri (needs `WebviewWindow`).
pub fn app_is_window_maximized() -> bool {
    // TODO(G2): migrate original body from Tauri
    false
}

// TODO(G2): migrate original body from Tauri (needs `WebviewWindow` +
// `crate::cancel_file_editor_close`).
pub fn app_cancel_file_editor_close(app: &AppHandle) {
    // TODO(G2): migrate original body from Tauri
}

// TODO(G2): migrate original body from Tauri (needs `WebviewWindow` +
// `crate::show_window_context_menu`).
pub fn app_show_window_menu(
    app: &AppHandle,
    menu_type: String,
    x: f64,
    y: f64,
) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

// ==========================================
// Phase 3 commands implementation
// ==========================================

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `services::profile_ops`).
pub(crate) async fn get_workspace_snapshot_unlocked(
    app: &AppHandle,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration`).
pub async fn get_workspace_snapshot(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState"))
}

// TODO(G3): migrate original body from Tauri (needs `app.emit` +
// `services::logging`).
async fn get_workspace_snapshot_and_emit(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs event emitter"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `WorkerCmd` + `oneshot`).
async fn send_worker_cmd<T>(app: &AppHandle, tab_id: &str) -> Result<T, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `WorkspaceState`).
async fn refresh_remote_files(app: &AppHandle, tab_id: &str, path: &str) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

fn create_tab_layout(profile_type: &str) -> String {
    match profile_type {
        "ssh" => "terminal-file".to_string(),
        "ftp" => "file-only".to_string(),
        _ => "terminal-only".to_string(),
    }
}

// TODO(G3): migrate original body from Tauri (needs `crate::sessions::*`
// workers + `mpsc` + `CancellationToken`).
fn start_session_worker(tab_id: String, profile: serde_json::Value, app: &AppHandle) {
    // TODO(G3): migrate original body from Tauri
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `WorkerCmd`).
async fn stop_session_worker(tab_id: &str) {
    // TODO(G3): migrate original body from Tauri
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `WorkerCmd`).
pub async fn shutdown_session_workers(app: &AppHandle) {
    // TODO(G3): migrate original body from Tauri
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `services::profile_ops` + `crate::sessions::*`).
pub async fn app_open_profile(
    app: &AppHandle,
    profile_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState`).
pub async fn app_activate_tab(
    app: &AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `crate::sessions::*` + `services::profile_ops`).
pub async fn app_reconnect_tab(
    app: &AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceTab` type from
// `services::workspace`).
fn claim_reconnect_tab(tab_id: &str) -> bool {
    // TODO(G3): migrate original body from Tauri
    false
}

// TODO(G3): migrate original body from Tauri (needs `services::transfers` +
// `WorkspaceState` + `crate::sessions::terminal`).
pub async fn app_disconnect_tab(
    app: &AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `services::transfers` +
// `WorkspaceState`).
pub async fn app_close_tab(app: &AppHandle, tab_id: String) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `send_terminal_input`).
pub async fn app_write_terminal(
    app: &AppHandle,
    tab_id: String,
    data: String,
) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `ipc::Channel`).
pub fn app_subscribe_terminal_data(app: &AppHandle) {
    // TODO(G3): migrate original body from Tauri
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `WorkerCmd`).
pub async fn app_resize_terminal(
    app: &AppHandle,
    tab_id: String,
    cols: u32,
    rows: u32,
    width: u32,
    height: u32,
) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `refresh_remote_files` +
// `WorkspaceState`).
pub async fn app_open_remote_path(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `refresh_remote_files` + `services::logging`).
pub async fn app_set_follow_shell_cwd(
    app: &AppHandle,
    tab_id: String,
    enabled: bool,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `WorkerCmd`).
pub async fn app_read_remote_file(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    encoding: Option<String>,
) -> Result<String, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_write_remote_file(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    content: String,
    encoding: Option<String>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_create_remote_directory(
    app: &AppHandle,
    tab_id: String,
    parent_path: String,
    name: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_create_remote_file(
    app: &AppHandle,
    tab_id: String,
    parent_path: String,
    name: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_copy_remote_path(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    destination_path: String,
    target_type: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_move_remote_path(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    destination_path: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_rename_remote_path(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    new_name: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_delete_remote_path(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    target_type: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PermissionApplyTarget {
    All,
    Files,
    Directories,
}

impl PermissionApplyTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Files => "files",
            Self::Directories => "directories",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemotePermissionChangeOptions {
    mode: String,
    #[serde(default)]
    recursive: bool,
    #[serde(default)]
    apply_to: Option<PermissionApplyTarget>,
}

fn parse_remote_permission_mode(mode: &str) -> Result<u32, AppError> {
    let trimmed = mode.trim();
    if !(3..=4).contains(&trimmed.len())
        || !trimmed
            .chars()
            .all(|character| matches!(character, '0'..='7'))
    {
        return Err(AppError::Command(
            "权限值必须是 3 到 4 位八进制数字，例如 755".to_string(),
        ));
    }
    u32::from_str_radix(trimmed, 8).map_err(|error| AppError::Command(error.to_string()))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd` +
// `refresh_remote_files`).
pub async fn app_change_remote_permissions(
    app: &AppHandle,
    tab_id: String,
    target_path: String,
    options: RemotePermissionChangeOptions,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_set_remote_file_access_mode(
    app: &AppHandle,
    tab_id: String,
    mode: String,
    options: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_queue_upload(
    app: &AppHandle,
    file_names: Vec<String>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_upload_file(
    app: &AppHandle,
    tab_id: String,
    local_path: String,
    remote_directory: String,
    options: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_download_file(
    app: &AppHandle,
    tab_id: String,
    remote_path: String,
    local_directory: String,
    options: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_download_remote_path(
    app: &AppHandle,
    tab_id: String,
    remote_path: String,
    target_type: String,
    local_directory: String,
    options: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_cancel_transfer(
    app: &AppHandle,
    transfer_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_pause_transfer(
    app: &AppHandle,
    transfer_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_resume_transfer(
    app: &AppHandle,
    transfer_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_discard_transfer(
    app: &AppHandle,
    transfer_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G4): migrate original body from Tauri (needs `services::transfers`).
pub async fn app_clear_transfers(
    app: &AppHandle,
    transfer_ids: Vec<String>,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G4: needs TransferService"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_list_ssh_tunnels(
    app: &AppHandle,
    tab_id: String,
) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_create_ssh_tunnel(
    app: &AppHandle,
    tab_id: String,
    rule: serde_json::Value,
) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_start_ssh_tunnel(
    app: &AppHandle,
    tab_id: String,
    rule_id: String,
) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_stop_ssh_tunnel(
    app: &AppHandle,
    tab_id: String,
    rule_id: String,
) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `send_worker_cmd`).
pub async fn app_delete_ssh_tunnel(
    app: &AppHandle,
    tab_id: String,
    rule_id: String,
) -> Result<Vec<serde_json::Value>, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

// TODO(G3): migrate original body from Tauri (needs `WorkspaceState`).
pub async fn app_resolve_ssh_interaction(
    app: &AppHandle,
    request_id: String,
    response: serde_json::Value,
) -> Result<(), AppError> {
    Err(AppError::Unsupported("G3: needs WorkspaceState"))
}

// ==========================================
// Phase 2 commands: profile / folder / command CRUD
// ==========================================
//
// These commands delegate to `services::profile_ops`, which mirrors the
// Electron `FileProfileRepository` semantics (group/parentId self-healing,
// secrets stripping, cascade rename / delete, ordering).

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_create_profile(
    app: &AppHandle,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops` +
// `WorkspaceState`).
pub async fn app_update_profile(
    app: &AppHandle,
    profile_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_delete_profile(
    app: &AppHandle,
    profile_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_update_folder(
    app: &AppHandle,
    folder_id: String,
    updates: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_delete_folder(
    app: &AppHandle,
    folder_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_update_entity_order(
    app: &AppHandle,
    id: String,
    new_parent_id: Option<String>,
    new_order: f64,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_update_command_folder(
    app: &AppHandle,
    folder_id: String,
    updates: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_delete_command_folder(
    app: &AppHandle,
    folder_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_update_command_order(
    app: &AppHandle,
    id: String,
    new_parent_id: Option<String>,
    new_order: f64,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_update_command_template(
    app: &AppHandle,
    command_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

// TODO(G3): migrate original body from Tauri (needs
// `lock_library_after_transfer_hydration` + `services::profile_ops`).
pub async fn app_delete_command_template(
    app: &AppHandle,
    command_id: String,
) -> Result<serde_json::Value, AppError> {
    Err(AppError::Unsupported("G3: needs profile_ops service"))
}

/// Render and send a command template to an active SSH session.
///
/// This intentionally performs the rendering in the main process: the command
/// source is durable storage, while the renderer only supplies positional
/// arguments and whether the final carriage return is desired. It mirrors the
/// Electron workspace service and keeps arbitrary command text out of the IPC
/// surface.
//
// TODO(G3): migrate original body from Tauri (needs `WorkspaceState` +
// `send_terminal_input`). `render_command_template` below is migrated (G1)
// because it is a pure regex substitution; only the session dispatch half
// waits on G3.
pub async fn app_execute_command_template(
    app: &AppHandle,
    tab_id: String,
    command_id: String,
    args: Option<Vec<String>>,
    options: Option<Value>,
) -> Result<Value, AppError> {
    Err(AppError::Unsupported("G3: needs sessions/services"))
}

fn render_command_template(template: &str, args: &[String]) -> String {
    // `[p#1]` is the durable command-template placeholder format shared with
    // Electron. Invalid/out-of-range references deliberately render as an
    // empty string so existing command libraries retain their behavior.
    let placeholder = Regex::new(r"\[p#(\d+)\]").expect("constant placeholder regex must compile");
    placeholder
        .replace_all(template, |captures: &regex::Captures<'_>| {
            captures
                .get(1)
                .and_then(|index| index.as_str().parse::<usize>().ok())
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| args.get(index))
                .cloned()
                .unwrap_or_default()
        })
        .into_owned()
}

#[cfg(test)]
mod external_url_tests {
    use super::validate_external_url;

    #[test]
    fn accepts_only_http_and_https_urls() {
        assert!(validate_external_url("https://fileterm.example/docs").is_ok());
        assert!(validate_external_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_external_url("file:///tmp/secret").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("not a url").is_err());
    }
}

#[cfg(test)]
mod command_template_tests {
    use super::render_command_template;

    #[test]
    fn renders_positional_command_template_arguments() {
        assert_eq!(
            render_command_template(
                "deploy [p#1] --region [p#2] --empty=[p#3]",
                &["api".to_string(), "cn-north".to_string(),]
            ),
            "deploy api --region cn-north --empty="
        );
    }
}

#[cfg(test)]
mod architecture_tests {
    use super::resolve_native_arch;

    #[test]
    fn reports_apple_silicon_when_x64_process_runs_under_rosetta() {
        assert_eq!(resolve_native_arch("macos", "x86_64", true), "arm64");
    }

    #[test]
    fn canonicalizes_native_rust_architecture_names() {
        assert_eq!(resolve_native_arch("macos", "aarch64", true), "arm64");
        assert_eq!(resolve_native_arch("macos", "x86_64", false), "x64");
        assert_eq!(resolve_native_arch("linux", "x86_64", false), "x64");
    }
}

#[cfg(test)]
mod window_lifecycle_tests {
    use super::renderer_approved_close_should_destroy;

    #[test]
    fn main_window_close_keeps_the_lifecycle_guard() {
        assert!(!renderer_approved_close_should_destroy("main"));
        assert!(renderer_approved_close_should_destroy(
            "file-editor-local-1"
        ));
        assert!(renderer_approved_close_should_destroy("connection-manager"));
    }
}

#[cfg(test)]
mod ui_state_tests {
    use super::normalize_ui_state;

    #[test]
    fn reads_current_object_ui_state() {
        let states = normalize_ui_state(serde_json::json!({ "main.tab-ui": "tabs" })).unwrap();
        assert_eq!(
            states.get("main.tab-ui").and_then(|value| value.as_str()),
            Some("tabs")
        );
    }

    #[test]
    fn migrates_electron_and_legacy_array_ui_state() {
        let electron = normalize_ui_state(serde_json::json!({
            "version": 1,
            "values": { "ssh-key-manager-ui": "folders" }
        }))
        .unwrap();
        assert_eq!(
            electron
                .get("ssh-key-manager-ui")
                .and_then(|value| value.as_str()),
            Some("folders")
        );

        let legacy = normalize_ui_state(serde_json::json!([
            { "key": "ssh-key-manager-ui", "value": "legacy-folders" }
        ]))
        .unwrap();
        assert_eq!(
            legacy
                .get("ssh-key-manager-ui")
                .and_then(|value| value.as_str()),
            Some("legacy-folders")
        );
    }
}

#[cfg(test)]
mod permission_contract_tests {
    use super::{
        parse_remote_permission_mode, PermissionApplyTarget, RemotePermissionChangeOptions,
    };

    #[test]
    fn reads_shared_camel_case_permission_contract() {
        let options: RemotePermissionChangeOptions = serde_json::from_value(serde_json::json!({
            "mode": "0640",
            "recursive": true,
            "applyTo": "files"
        }))
        .expect("shared permission options should deserialize");

        assert_eq!(parse_remote_permission_mode(&options.mode).unwrap(), 0o640);
        assert!(options.recursive);
        assert!(matches!(
            options.apply_to,
            Some(PermissionApplyTarget::Files)
        ));
    }

    #[test]
    fn rejects_legacy_permissions_field_instead_of_defaulting_to_0755() {
        let options = serde_json::from_value::<RemotePermissionChangeOptions>(serde_json::json!({
            "permissions": 384,
            "recursive": false
        }));
        assert!(options.is_err());
    }

    #[test]
    fn validates_octal_permission_modes() {
        assert_eq!(parse_remote_permission_mode("600").unwrap(), 0o600);
        assert_eq!(parse_remote_permission_mode("755").unwrap(), 0o755);
        assert!(parse_remote_permission_mode("888").is_err());
        assert!(parse_remote_permission_mode("75").is_err());
    }
}

#[cfg(test)]
mod detached_window_label_tests {
    use super::{is_detached_session_label, renderer_approved_close_should_destroy};

    #[test]
    fn detached_label_prefix_is_recognized() {
        // detached-<n> 是 registry 注册的独立窗口 label 格式
        assert!(is_detached_session_label("detached-1"));
        assert!(is_detached_session_label("detached-42"));
        // starts_with 匹配：detached- 本身也算（registry 不会生成空后缀，
        // 但函数只做前缀判断，调用方负责传入合法 label）
        assert!(is_detached_session_label("detached-"));
        // 主窗口与其他子窗口不算
        assert!(!is_detached_session_label("main"));
        assert!(!is_detached_session_label("connection-manager"));
        assert!(!is_detached_session_label("file-editor"));
        assert!(!is_detached_session_label(""));
    }

    #[test]
    fn renderer_approved_close_distinguishes_main_and_children() {
        // 主窗口关闭需经 CloseRequested guard（hide/quit/cancel）
        assert!(!renderer_approved_close_should_destroy("main"));
        // 子窗口（form/editor/manager）已由 renderer 批准，直接 destroy
        assert!(renderer_approved_close_should_destroy("connection-form"));
        assert!(renderer_approved_close_should_destroy("file-editor"));
        // detached-session 走独立路径（is_detached_session_label），
        // 但 renderer_approved_close_should_destroy 仍返回 true：
        // 调用方需先检查 is_detached_session_label 再判断此函数。
        assert!(renderer_approved_close_should_destroy("detached-1"));
    }
}
