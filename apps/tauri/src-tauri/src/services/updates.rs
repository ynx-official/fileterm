//! Platform-specific application updates.
//!
//! Windows uses Tauri's signed updater and keeps a verified package in memory
//! until the user confirms the restart. macOS intentionally remains on the
//! GitHub Release-page path so the user explicitly downloads the DMG/ZIP.

use tauri::{AppHandle, Emitter, Manager};

use crate::AppError;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/St0ff3l/fileterm/releases/latest";
const LATEST_RELEASE_PAGE: &str = "https://github.com/St0ff3l/fileterm/releases/latest";

#[cfg(target_os = "windows")]
pub struct WindowsDownloadedUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

#[cfg(target_os = "windows")]
const fn primary_update_mode() -> &'static str {
    "in-app"
}

#[cfg(not(target_os = "windows"))]
const fn primary_update_mode() -> &'static str {
    "release-page"
}

fn initial_status(app: &AppHandle) -> serde_json::Value {
    #[cfg(target_os = "windows")]
    let message = "Windows 将下载并验证签名，重启后安装更新。";
    #[cfg(not(target_os = "windows"))]
    let message = "检查 GitHub Release；安装将通过发布页完成。";

    serde_json::json!({
        "state": "idle",
        "currentVersion": current_version(app),
        "updateMode": primary_update_mode(),
        "message": message,
    })
}

async fn set_status(app: &AppHandle, status: serde_json::Value) {
    *app.state::<crate::services::workspace::WorkspaceState>()
        .update_status
        .write()
        .await = Some(status.clone());
    let _ = app.emit("app:update-status", status);
}

pub async fn get_status(app: &AppHandle) -> serde_json::Value {
    app.state::<crate::services::workspace::WorkspaceState>()
        .update_status
        .read()
        .await
        .clone()
        .unwrap_or_else(|| initial_status(app))
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .trim_start_matches('v')
        .split(|character: char| !(character.is_ascii_digit()))
        .filter(|part| !part.is_empty())
        .take(4)
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let candidate = version_parts(candidate);
    let current = version_parts(current);
    let width = candidate.len().max(current.len());
    (0..width).find_map(|index| {
        let left = candidate.get(index).copied().unwrap_or(0);
        let right = current.get(index).copied().unwrap_or(0);
        (left != right).then_some(left > right)
    }) == Some(true)
}

async fn check_release_page_update(app: &AppHandle) -> serde_json::Value {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("FileTerm-Tauri")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return serde_json::json!({
                "state": "error", "currentVersion": current_version(app), "updateMode": "release-page",
                "message": format!("更新检查初始化失败: {error}"),
            });
        }
    };

    let response = client.get(LATEST_RELEASE_API).send().await;
    match response {
        Ok(response) if response.status().is_success() => {
            let release: serde_json::Value = match response.json().await {
                Ok(release) => release,
                Err(error) => {
                    return serde_json::json!({
                        "state": "error", "currentVersion": current_version(app), "updateMode": "release-page",
                        "message": format!("更新元数据无效: {error}"),
                    });
                }
            };
            let version = release
                .get("tag_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let release_url = release
                .get("html_url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(LATEST_RELEASE_PAGE);
            if !version.is_empty() && is_newer(version, &current_version(app)) {
                serde_json::json!({
                    "state": "available", "currentVersion": current_version(app), "updateMode": "release-page",
                    "availableVersion": version.trim_start_matches('v'), "releaseUrl": release_url,
                })
            } else {
                serde_json::json!({ "state": "not-available", "currentVersion": current_version(app), "updateMode": "release-page" })
            }
        }
        Ok(response) => serde_json::json!({
            "state": "error", "currentVersion": current_version(app), "updateMode": "release-page",
            "message": format!("更新检查失败 ({})", response.status()),
        }),
        Err(error) => serde_json::json!({
            "state": "error", "currentVersion": current_version(app), "updateMode": "release-page",
            "message": format!("更新检查失败: {error}"),
        }),
    }
}

#[cfg(target_os = "windows")]
async fn check_windows_update(app: &AppHandle) -> serde_json::Value {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            crate::services::logging::warn(
                app,
                "update",
                format!("in-app updater unavailable, using release page fallback: {error}"),
            );
            return check_release_page_update(app).await;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => serde_json::json!({
            "state": "available",
            "currentVersion": current_version(app),
            "updateMode": "in-app",
            "availableVersion": update.version,
            "releaseUrl": LATEST_RELEASE_PAGE,
        }),
        Ok(None) => serde_json::json!({
            "state": "not-available", "currentVersion": current_version(app), "updateMode": "in-app"
        }),
        Err(error) => {
            crate::services::logging::warn(
                app,
                "update",
                format!("in-app update check failed, using release page fallback: {error}"),
            );
            let mut fallback = check_release_page_update(app).await;
            if fallback.get("state").and_then(serde_json::Value::as_str) == Some("available") {
                fallback["message"] = serde_json::Value::String(
                    "Windows 自动更新暂不可用，已切换到 GitHub 下载。".to_string(),
                );
            }
            fallback
        }
    }
}

pub async fn check(app: &AppHandle) -> Result<serde_json::Value, AppError> {
    let update_check = app
        .state::<crate::services::workspace::WorkspaceState>()
        .update_check
        .clone();
    let check_guard = match update_check.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            // Another window already started the network request. Wait for it
            // instead of issuing a duplicate release/updater request.
            let guard = update_check.lock().await;
            drop(guard);
            return Ok(get_status(app).await);
        }
    };

    #[cfg(target_os = "windows")]
    {
        app.state::<crate::services::workspace::WorkspaceState>()
            .windows_downloaded_update
            .lock()
            .await
            .take();
    }

    crate::services::logging::info(app, "update", "check started");
    set_status(
        app,
        serde_json::json!({
            "state": "checking",
            "currentVersion": current_version(app),
            "updateMode": primary_update_mode(),
        }),
    )
    .await;

    #[cfg(target_os = "windows")]
    let status = check_windows_update(app).await;
    #[cfg(not(target_os = "windows"))]
    let status = check_release_page_update(app).await;

    let state = status
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if state == "error" {
        crate::services::logging::warn(app, "update", "check completed state=error");
    } else {
        crate::services::logging::info(app, "update", format!("check completed state={state}"));
    }
    set_status(app, status.clone()).await;
    drop(check_guard);
    Ok(status)
}

pub async fn open_release_page(app: &AppHandle) -> Result<(), AppError> {
    let status = get_status(app).await;
    let url = status
        .get("releaseUrl")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(LATEST_RELEASE_PAGE);
    let result = open::that(url).map_err(|error| AppError::Command(error.to_string()));
    match &result {
        Ok(()) => crate::services::logging::info(app, "update", "release page opened"),
        Err(error) => crate::services::logging::error(
            app,
            "update",
            format!("open release page failed: {error}"),
        ),
    }
    result
}

#[cfg(target_os = "windows")]
fn current_update_mode(status: &serde_json::Value) -> &str {
    status
        .get("updateMode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("in-app")
}

#[cfg(target_os = "windows")]
async fn download_windows_update(app: &AppHandle) -> Result<(), AppError> {
    use tauri_plugin_updater::UpdaterExt;

    let update_operation = app
        .state::<crate::services::workspace::WorkspaceState>()
        .update_operation
        .clone();
    let _operation_guard = update_operation.lock().await;
    let existing_status = get_status(app).await;
    if current_update_mode(&existing_status) == "release-page" {
        return open_release_page(app).await;
    }

    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            let status = serde_json::json!({
                "state": "error", "currentVersion": current_version(app), "updateMode": "in-app",
                "message": format!("自动更新不可用: {error}"),
            });
            set_status(app, status).await;
            return Ok(());
        }
    };
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => {
            let status = serde_json::json!({
                "state": "not-available", "currentVersion": current_version(app), "updateMode": "in-app"
            });
            set_status(app, status).await;
            return Ok(());
        }
        Err(error) => {
            let status = serde_json::json!({
                "state": "error", "currentVersion": current_version(app), "updateMode": "in-app",
                "message": format!("重新检查更新失败: {error}"),
            });
            set_status(app, status).await;
            return Ok(());
        }
    };

    let version = update.version.clone();
    let current = current_version(app);
    set_status(
        app,
        serde_json::json!({
            "state": "downloading", "currentVersion": current, "updateMode": "in-app",
            "availableVersion": version, "progress": 0,
        }),
    )
    .await;

    let app_for_progress = app.clone();
    let status_store = app
        .state::<crate::services::workspace::WorkspaceState>()
        .update_status
        .clone();
    let progress_current = current_version(app);
    let progress_version = version.clone();
    let mut received = 0_u64;
    let mut last_progress = 0_u64;
    let bytes = update
        .download(
            move |chunk_length, content_length| {
                received = received.saturating_add(chunk_length as u64);
                let progress = content_length
                    .filter(|total| *total > 0)
                    .map(|total| (received.saturating_mul(100) / total).min(100))
                    .unwrap_or(0);
                if progress == last_progress && progress != 100 {
                    return;
                }
                last_progress = progress;
                let status = serde_json::json!({
                    "state": "downloading", "currentVersion": progress_current,
                    "updateMode": "in-app", "availableVersion": progress_version,
                    "progress": progress,
                });
                if let Ok(mut current_status) = status_store.try_write() {
                    *current_status = Some(status.clone());
                }
                let _ = app_for_progress.emit("app:update-status", status);
            },
            || {},
        )
        .await;

    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            let status = serde_json::json!({
                "state": "error", "currentVersion": current_version(app), "updateMode": "in-app",
                "availableVersion": version, "message": format!("更新包下载或签名验证失败: {error}"),
            });
            set_status(app, status).await;
            crate::services::logging::warn(
                app,
                "update",
                "download or signature verification failed",
            );
            return Ok(());
        }
    };

    *app.state::<crate::services::workspace::WorkspaceState>()
        .windows_downloaded_update
        .lock()
        .await = Some(WindowsDownloadedUpdate { update, bytes });
    let status = serde_json::json!({
        "state": "downloaded", "currentVersion": current_version(app), "updateMode": "in-app",
        "availableVersion": version,
    });
    set_status(app, status).await;
    crate::services::logging::info(app, "update", "signed update downloaded and verified");
    Ok(())
}

#[cfg(target_os = "windows")]
async fn install_windows_update(app: &AppHandle) -> Result<(), AppError> {
    let update_operation = app
        .state::<crate::services::workspace::WorkspaceState>()
        .update_operation
        .clone();
    let _operation_guard = update_operation.lock().await;
    let pending = app
        .state::<crate::services::workspace::WorkspaceState>()
        .windows_downloaded_update
        .lock()
        .await
        .take();
    let Some(pending) = pending else {
        let status = serde_json::json!({
            "state": "error", "currentVersion": current_version(app), "updateMode": "in-app",
            "message": "没有已验证的更新包，请重新检查更新。",
        });
        set_status(app, status).await;
        return Ok(());
    };

    crate::services::logging::info(app, "update", "launching verified Windows installer");
    if let Err(error) = pending.update.install(pending.bytes) {
        let status = serde_json::json!({
            "state": "error", "currentVersion": current_version(app), "updateMode": "in-app",
            "message": format!("启动更新安装器失败: {error}"),
        });
        set_status(app, status).await;
    }
    Ok(())
}

pub async fn download(app: &AppHandle) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        return download_windows_update(app).await;
    }
    #[cfg(not(target_os = "windows"))]
    {
        open_release_page(app).await
    }
}

pub async fn install(app: &AppHandle) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        return install_windows_update(app).await;
    }
    #[cfg(not(target_os = "windows"))]
    {
        open_release_page(app).await
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_release_tags_numerically() {
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(!is_newer("v1.1.1", "1.1.1"));
    }
}
