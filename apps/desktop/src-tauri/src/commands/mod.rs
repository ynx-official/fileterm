use tauri::{AppHandle, Manager, WebviewWindow};
use serde::{Serialize, Deserialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use crate::AppError;
use crate::storage::{read_json_array, write_json_array, new_id};
use crate::sessions::WorkerCmd;

#[derive(Serialize, Deserialize, Debug)]
pub struct UiPreferences {
    pub theme: String,
    pub locale: String,
}

#[derive(Deserialize, Debug)]
pub struct UiPreferencesInput {
    pub theme: Option<String>,
    pub locale: Option<String>,
}

#[tauri::command]
pub fn app_get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub fn app_get_arch() -> String {
    std::env::consts::ARCH.to_string()
}

#[tauri::command]
pub fn app_read_clipboard_text() -> Result<String, AppError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .get_text()
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

#[tauri::command]
pub fn app_write_clipboard_text(text: String) -> Result<(), AppError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

#[tauri::command]
pub fn app_open_external_url(url: String) -> Result<(), AppError> {
    open::that(url).map_err(|error| AppError::Command(error.to_string()))
}

#[tauri::command]
pub fn app_get_ui_preferences(app: AppHandle) -> Result<UiPreferences, AppError> {
    let path = crate::storage::state_path(&app)?;
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .map_err(|error| AppError::Storage(error.to_string()))?;
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

#[tauri::command]
pub fn app_set_ui_preferences(app: AppHandle, input: UiPreferencesInput) -> Result<(), AppError> {
    let path = crate::storage::state_path(&app)?;
    let mut preferences = app_get_ui_preferences(app)?;
    if let Some(theme) = input.theme {
        preferences.theme = theme;
    }
    if let Some(locale) = input.locale {
        preferences.locale = locale;
    }
    let content = serde_json::to_string_pretty(&preferences)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    std::fs::write(path, content).map_err(|error| AppError::Storage(error.to_string()))
}

#[tauri::command]
pub fn app_get_ui_state_item(app: AppHandle, key: String) -> Result<Option<String>, AppError> {
    let states = read_json_array(&app, "ui-state.json")?;
    for state in states {
        if state.get("key").and_then(|k| k.as_str()) == Some(&key) {
            return Ok(state.get("value").and_then(|v| v.as_str()).map(ToString::to_string));
        }
    }
    Ok(None)
}

#[tauri::command]
pub fn app_set_ui_state_item(app: AppHandle, key: String, value: String) -> Result<(), AppError> {
    let mut states = read_json_array(&app, "ui-state.json")?;
    let mut found = false;
    for state in &mut states {
        if state.get("key").and_then(|k| k.as_str()) == Some(&key) {
            if let Some(obj) = state.as_object_mut() {
                obj.insert("value".to_string(), Value::String(value.clone()));
                found = true;
                break;
            }
        }
    }
    if !found {
        states.push(serde_json::json!({ "key": key, "value": value }));
    }
    write_json_array(&app, "ui-state.json", &states)
}

#[tauri::command]
pub fn app_remove_ui_state_item(app: AppHandle, key: String) -> Result<(), AppError> {
    let states = read_json_array(&app, "ui-state.json")?;
    let next_states: Vec<Value> = states
        .into_iter()
        .filter(|state| state.get("key").and_then(|k| k.as_str()) != Some(&key))
        .collect();
    write_json_array(&app, "ui-state.json", &next_states)
}

#[tauri::command]
pub async fn app_get_snapshot(app: AppHandle) -> Result<serde_json::Value, AppError> {
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub fn app_get_connection_library(app: AppHandle) -> Result<serde_json::Value, AppError> {
    Ok(serde_json::json!({
        "profiles": read_json_array(&app, "profiles.json")?,
        "folders": read_json_array(&app, "folders.json")?,
    }))
}

#[tauri::command]
pub fn app_get_webdav_sync_config(app: AppHandle) -> Result<serde_json::Value, AppError> {
    let path = crate::storage::workspace_file(&app, "webdav-sync.json")?;
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .map_err(|error| AppError::Storage(error.to_string()))?;
        let config: serde_json::Value = serde_json::from_str(&content)
            .map_err(|error| AppError::Serialization(error.to_string()))?;
        Ok(config)
    } else {
        Ok(serde_json::json!({ "enabled": false, "url": "", "remotePath": "" }))
    }
}

#[tauri::command]
pub fn app_set_webdav_sync_config(app: AppHandle, input: serde_json::Value) -> Result<(), AppError> {
    let path = crate::storage::workspace_file(&app, "webdav-sync.json")?;
    let content = serde_json::to_string_pretty(&input)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    std::fs::write(path, content).map_err(|error| AppError::Storage(error.to_string()))
}

#[tauri::command]
pub async fn app_workspace_mutation(
    app: AppHandle,
    operation: String,
    payload: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    match operation.as_str() {
        "create-profile" => {
            let mut profiles = read_json_array(&app, "profiles.json")?;
            if let Some(input) = payload.get("input").cloned() {
                let mut profile = input.clone();
                let port = profile.get("port").cloned().unwrap_or(serde_json::json!(22));
                if let Some(obj) = profile.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(new_id("profile")));
                    obj.insert("port".to_string(), port);
                }
                profiles.push(profile);
                write_json_array(&app, "profiles.json", &profiles)?;
            }
        }
        "create-folder" => {
            let mut folders = read_json_array(&app, "folders.json")?;
            let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("新建分类");
            let parent_id = payload.get("parentId").and_then(|id| id.as_str());
            folders.push(serde_json::json!({
                "id": new_id("folder"),
                "name": name,
                "parentId": parent_id
            }));
            write_json_array(&app, "folders.json", &folders)?;
        }
        "create-command-folder" => {
            let mut folders = read_json_array(&app, "command-folders.json")?;
            let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("新建命令分类");
            let parent_id = payload.get("parentId").and_then(|id| id.as_str());
            folders.push(serde_json::json!({
                "id": new_id("cmd-folder"),
                "name": name,
                "parentId": parent_id
            }));
            write_json_array(&app, "command-folders.json", &folders)?;
        }
        "create-command" => {
            let mut commands = read_json_array(&app, "commands.json")?;
            if let Some(input) = payload.get("input").cloned() {
                let mut command = input.clone();
                if let Some(obj) = command.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(new_id("cmd")));
                }
                commands.push(command);
                write_json_array(&app, "commands.json", &commands)?;
            }
        }
        _ => return Err(AppError::Command(format!("Unsupported operation: {operation}"))),
    }
    get_workspace_snapshot(app).await
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenWindowInput {
    pub kind: String,
    pub mode: Option<String>,
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
    #[serde(rename = "commandId")]
    pub command_id: Option<String>,
    #[serde(rename = "folderId")]
    pub folder_id: Option<String>,
    pub source: Option<String>,
    pub path: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "tabId")]
    pub tab_id: Option<String>,
    pub encoding: Option<String>,
}

#[tauri::command]
pub fn app_open_window(
    app: AppHandle,
    input: OpenWindowInput,
) -> Result<(), AppError> {
    crate::open_child_window(&app, input)
}

#[tauri::command]
pub fn app_window_action(app: AppHandle, window: WebviewWindow, action: String) -> Result<(), AppError> {
    match action.as_str() {
        "minimize" => {
            let _ = window.minimize();
        }
        "toggle-maximize" => {
            if let Ok(true) = window.is_maximized() {
                let _ = window.unmaximize();
            } else {
                let _ = window.maximize();
            }
        }
        "close" => {
            // User confirmed close in the renderer — bypass the
            // `CloseRequested` guard (which would re-emit
            // `app:window-close-request` and loop forever) by destroying the
            // window directly via the raw handle. `window.close()` re-fires
            // `CloseRequested`, so we use `window.destroy()` instead.
            let _ = window.destroy();
        }
        "quit" => {
            // Quit the entire app. Used by the renderer when the user
            // confirms a Cmd+Q / tray-quit request.
            app.exit(0);
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn app_is_window_maximized(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

// ==========================================
// Phase 3 commands implementation
// ==========================================

pub async fn get_workspace_snapshot(app: AppHandle) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();

    let tabs = state.tabs.read().await.clone();
    let active_tab_id = state.active_tab_id.read().await.clone();
    let sessions = state.sessions.read().await.clone();

    // Read + heal profiles, then strip secrets before exposing in snapshot.
    let (profiles_with_secrets, folders) = crate::services::profile_ops::read_and_heal_profiles(&app)?;
    let profiles: Vec<serde_json::Value> = profiles_with_secrets
        .iter()
        .map(|p| crate::services::profile_ops::strip_secret_fields_public(p))
        .collect();
    let command_folders = read_json_array(&app, "command-folders.json")?;
    let commands = read_json_array(&app, "commands.json")?;

    Ok(serde_json::json!({
        "profiles": profiles,
        "folders": folders,
        "commandFolders": command_folders,
        "commandTemplates": commands,
        "tabs": tabs,
        "activeTabId": active_tab_id,
        "transfers": [],
        "sessions": sessions,
    }))
}

async fn send_worker_cmd<T>(
    app: &AppHandle,
    tab_id: &str,
    make_cmd: impl FnOnce(oneshot::Sender<Result<T, String>>) -> WorkerCmd,
) -> Result<T, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let workers = state.workers.read().await;
    let sender = workers.get(tab_id).ok_or_else(|| AppError::Storage("Session not found".to_string()))?;
    
    let (tx, rx) = oneshot::channel();
    let cmd = make_cmd(tx);
    sender.send(cmd).await.map_err(|e| AppError::Storage(e.to_string()))?;
    
    let res = rx.await.map_err(|e| AppError::Storage(e.to_string()))?.map_err(|e| AppError::Storage(e))?;
    Ok(res)
}

async fn refresh_remote_files(app: &AppHandle, tab_id: &str, path: &str) -> Result<(), AppError> {
    let files = send_worker_cmd(app, tab_id, |tx| WorkerCmd::ListRemoteFiles {
        path: path.to_string(),
        respond_to: tx,
    }).await?;
    
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let mut sessions = state.sessions.write().await;
    if let Some(session) = sessions.get_mut(tab_id) {
        session.remote_files = files;
    }
    Ok(())
}

fn create_tab_layout(profile_type: &str) -> String {
    match profile_type {
        "ssh" => "terminal-file".to_string(),
        "ftp" => "file-only".to_string(),
        _ => "terminal-only".to_string(),
    }
}

#[tauri::command]
pub async fn app_open_profile(
    app: AppHandle,
    profile_id: String,
) -> Result<serde_json::Value, AppError> {
    let profiles = read_json_array(&app, "profiles.json")?;
    let profile = profiles.iter().find(|p| p.get("id").and_then(|id| id.as_str()) == Some(&profile_id))
        .ok_or_else(|| AppError::Storage("Profile not found".to_string()))?;
    
    let profile_type = profile.get("type").and_then(|t| t.as_str()).unwrap_or("ssh");
    let name = profile.get("name").and_then(|n| n.as_str()).unwrap_or("SSH Session");
    
    let tab_id = format!("tab-{}", uuid::Uuid::new_v4());
    let new_tab = crate::services::WorkspaceTab {
        id: tab_id.clone(),
        profile_id: profile_id.clone(),
        session_type: profile_type.to_string(),
        title: name.to_string(),
        layout: create_tab_layout(profile_type),
        status: "connecting".to_string(),
    };

    let host = profile.get("host").and_then(|h| h.as_str()).unwrap_or("127.0.0.1");
    let port = profile.get("port").and_then(|p| p.as_i64()).unwrap_or(22) as u16;
    let username = profile.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut tabs = state.tabs.write().await;
        tabs.push(new_tab);
        let mut active = state.active_tab_id.write().await;
        *active = Some(tab_id.clone());

        let mut sessions = state.sessions.write().await;
        sessions.insert(tab_id.clone(), crate::services::SessionSnapshot {
            profile_id: profile_id.clone(),
            access_host: format!("{}:{}", host, port),
            summary: format!("{}@{}", username, host),
            terminal_transcript: "连接主机...\r\n".to_string(),
            remote_path: "/".to_string(),
            shell_cwd: Some("/".to_string()),
            follow_shell_cwd: true,
            remote_files: Vec::new(),
            file_access_mode: "user".to_string(),
            sudo_user: None,
            has_reusable_sudo_auth: false,
            connected: false,
            system_metrics: None,
        });
    }

    let (tx, rx) = mpsc::channel(100);
    {
        let mut workers = state.workers.write().await;
        workers.insert(tab_id.clone(), tx);
    }

    crate::sessions::ssh::start_ssh_worker(tab_id, profile.clone(), rx, app.clone());

    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_activate_tab(
    app: AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut active = state.active_tab_id.write().await;
        *active = Some(tab_id);
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_reconnect_tab(
    app: AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let profile_id = {
        let tabs = state.tabs.read().await;
        tabs.iter().find(|t| t.id == tab_id).map(|t| t.profile_id.clone())
    };

    if let Some(pid) = profile_id {
        let profiles = read_json_array(&app, "profiles.json")?;
        if let Some(profile) = profiles.iter().find(|p| p.get("id").and_then(|id| id.as_str()) == Some(&pid)) {
            // Terminate existing worker
            {
                let mut workers = state.workers.write().await;
                workers.remove(&tab_id);
            }
            
            // Set connecting status and reset transcript so the renderer
            // shows "连接主机..." via bootText hydration.
            {
                let mut tabs = state.tabs.write().await;
                if let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.status = "connecting".to_string();
                }
                let mut sessions = state.sessions.write().await;
                if let Some(session) = sessions.get_mut(&tab_id) {
                    session.connected = false;
                    session.terminal_transcript = "连接主机...\r\n".to_string();
                    session.remote_files = Vec::new();
                    session.system_metrics = None;
                }
            }

            let (tx, rx) = mpsc::channel(100);
            {
                let mut workers = state.workers.write().await;
                workers.insert(tab_id.clone(), tx);
            }

            crate::sessions::ssh::start_ssh_worker(tab_id, profile.clone(), rx, app.clone());
        }
    }

    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_disconnect_tab(
    app: AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut workers = state.workers.write().await;
        workers.remove(&tab_id);
    }
    {
        let mut tabs = state.tabs.write().await;
        if let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.status = "disconnected".to_string();
        }
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(&tab_id) {
            session.connected = false;
        }
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_close_tab(
    app: AppHandle,
    tab_id: String,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut workers = state.workers.write().await;
        workers.remove(&tab_id);
    }
    {
        let mut tabs = state.tabs.write().await;
        tabs.retain(|t| t.id != tab_id);
        
        let mut active = state.active_tab_id.write().await;
        if *active == Some(tab_id.clone()) {
            *active = tabs.last().map(|t| t.id.clone());
        }
        
        let mut sessions = state.sessions.write().await;
        sessions.remove(&tab_id);
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_write_terminal(
    app: AppHandle,
    tab_id: String,
    data: String,
) -> Result<(), AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let workers = state.workers.read().await;
    if let Some(sender) = workers.get(&tab_id) {
        let _ = sender.send(WorkerCmd::WriteTerminal(data)).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn app_resize_terminal(
    app: AppHandle,
    tab_id: String,
    cols: u32,
    rows: u32,
    width: u32,
    height: u32,
) -> Result<(), AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let workers = state.workers.read().await;
    if let Some(sender) = workers.get(&tab_id) {
        let _ = sender.send(WorkerCmd::ResizeTerminal { cols, rows, width, height }).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn app_open_remote_path(
    app: AppHandle,
    tab_id: String,
    target_path: String,
) -> Result<serde_json::Value, AppError> {
    refresh_remote_files(&app, &tab_id, &target_path).await?;
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(&tab_id) {
            session.remote_path = target_path;
        }
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_set_follow_shell_cwd(
    app: AppHandle,
    tab_id: String,
    enabled: bool,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(&tab_id) {
            session.follow_shell_cwd = enabled;
        }
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_read_remote_file(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    encoding: Option<String>,
) -> Result<String, AppError> {
    let enc = encoding.unwrap_or_else(|| "utf-8".to_string());
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::ReadRemoteFile {
        path: target_path,
        encoding: enc,
        respond_to: tx,
    }).await
}

#[tauri::command]
pub async fn app_write_remote_file(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    content: String,
    encoding: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let enc = encoding.unwrap_or_else(|| "utf-8".to_string());
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::WriteRemoteFile {
        path: target_path.clone(),
        content,
        encoding: enc,
        respond_to: tx,
    }).await?;
    
    let parent = std::path::Path::new(&target_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let _ = refresh_remote_files(&app, &tab_id, &parent).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_create_remote_directory(
    app: AppHandle,
    tab_id: String,
    parent_path: String,
    name: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::CreateRemoteDirectory {
        parent_path: parent_path.clone(),
        name,
        respond_to: tx,
    }).await?;
    
    let _ = refresh_remote_files(&app, &tab_id, &parent_path).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_create_remote_file(
    app: AppHandle,
    tab_id: String,
    parent_path: String,
    name: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::CreateRemoteFile {
        parent_path: parent_path.clone(),
        name,
        respond_to: tx,
    }).await?;
    
    let _ = refresh_remote_files(&app, &tab_id, &parent_path).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_copy_remote_path(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    destination_path: String,
    target_type: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::CopyRemotePath {
        target_path,
        destination_path: destination_path.clone(),
        target_type,
        respond_to: tx,
    }).await?;
    
    let parent = std::path::Path::new(&destination_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let _ = refresh_remote_files(&app, &tab_id, &parent).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_move_remote_path(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    destination_path: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::MoveRemotePath {
        target_path: target_path.clone(),
        destination_path: destination_path.clone(),
        respond_to: tx,
    }).await?;
    
    let parent_src = std::path::Path::new(&target_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let parent_dest = std::path::Path::new(&destination_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
        
    let _ = refresh_remote_files(&app, &tab_id, &parent_src).await;
    if parent_src != parent_dest {
        let _ = refresh_remote_files(&app, &tab_id, &parent_dest).await;
    }
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_rename_remote_path(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    new_name: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::RenameRemotePath {
        target_path: target_path.clone(),
        new_name,
        respond_to: tx,
    }).await?;
    
    let parent = std::path::Path::new(&target_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let _ = refresh_remote_files(&app, &tab_id, &parent).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_delete_remote_path(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    target_type: String,
) -> Result<serde_json::Value, AppError> {
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::DeleteRemotePath {
        target_path: target_path.clone(),
        target_type,
        respond_to: tx,
    }).await?;
    
    let parent = std::path::Path::new(&target_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let _ = refresh_remote_files(&app, &tab_id, &parent).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_change_remote_permissions(
    app: AppHandle,
    tab_id: String,
    target_path: String,
    options: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let permissions = options.get("permissions").and_then(|p| p.as_u64()).unwrap_or(0o755) as u32;
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::ChangeRemotePermissions {
        target_path: target_path.clone(),
        permissions,
        respond_to: tx,
    }).await?;
    
    let parent = std::path::Path::new(&target_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let _ = refresh_remote_files(&app, &tab_id, &parent).await;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_set_remote_file_access_mode(
    app: AppHandle,
    tab_id: String,
    mode: String,
    options: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    let sudo_user = options
        .as_ref()
        .and_then(|o| o.get("sudoUser"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let sudo_password = options
        .as_ref()
        .and_then(|o| o.get("sudoPassword"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    send_worker_cmd(&app, &tab_id, |tx| WorkerCmd::SetRemoteFileAccessMode {
        mode,
        sudo_user,
        sudo_password,
        respond_to: tx,
    }).await?;

    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_resolve_ssh_interaction(
    app: AppHandle,
    request_id: String,
    response: serde_json::Value,
) -> Result<(), AppError> {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let sender = {
        let mut pending = state.pending_interactions.write().await;
        pending.remove(&request_id)
    };
    if let Some(tx) = sender {
        // Sender error means the receiver was dropped (handshake timed out
        // or the worker exited) — not actionable, ignore.
        let _ = tx.send(response);
    }
    Ok(())
}

// ==========================================
// Phase 2 commands: profile / folder / command CRUD
// ==========================================
//
// These commands delegate to `services::profile_ops`, which mirrors the
// Electron `FileProfileRepository` semantics (group/parentId self-healing,
// secrets stripping, cascade rename / delete, ordering).

#[tauri::command]
pub async fn app_create_profile(
    app: AppHandle,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::create_profile(&app, input)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_profile(
    app: AppHandle,
    profile_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_profile(&app, &profile_id, input)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_delete_profile(
    app: AppHandle,
    profile_id: String,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::delete_profile(&app, &profile_id)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_folder(
    app: AppHandle,
    folder_id: String,
    updates: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_folder(&app, &folder_id, updates)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_delete_folder(
    app: AppHandle,
    folder_id: String,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::delete_folder(&app, &folder_id)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_entity_order(
    app: AppHandle,
    id: String,
    new_parent_id: Option<String>,
    new_order: f64,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_entity_order(&app, &id, new_parent_id, new_order)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_command_folder(
    app: AppHandle,
    folder_id: String,
    updates: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_command_folder(&app, &folder_id, updates)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_delete_command_folder(
    app: AppHandle,
    folder_id: String,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::delete_command_folder(&app, &folder_id)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_command_order(
    app: AppHandle,
    id: String,
    new_parent_id: Option<String>,
    new_order: f64,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_command_order(&app, &id, new_parent_id, new_order)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_update_command_template(
    app: AppHandle,
    command_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::update_command_template(&app, &command_id, input)?;
    get_workspace_snapshot(app).await
}

#[tauri::command]
pub async fn app_delete_command_template(
    app: AppHandle,
    command_id: String,
) -> Result<serde_json::Value, AppError> {
    crate::services::profile_ops::delete_command_template(&app, &command_id)?;
    get_workspace_snapshot(app).await
}
