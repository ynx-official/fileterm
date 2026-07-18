//! Bridge layer: the in-process async API surface that GPUI views call to
//! reach backend services.
//!
//! The bridge is migrated incrementally by runnable product capability. Shared
//! storage and the public connection library are live; protocol/session,
//! transfer, and multi-window methods remain explicit `Unsupported` boundaries
//! until their service implementation is connected.
//!
//! ## Why a trait at all?
//!
//! Tauri's `invoke_handler!` macro registers 108 commands against a concrete
//! `AppHandle`. In GPUI there's no IPC boundary, so views could in principle
//! call backend fns directly. We still introduce a trait for three reasons:
//!
//! 1. **Testability**: views can be constructed against a mock
//!    `FileTermDesktopApi` in unit tests, without spinning up real SSH/SFTP.
//! 2. **Migration ordering**: the trait is the contract the view layer
//!    compiles against today, so as G1–G5 fill in methods one by one, the
//!    view layer never needs to change its call sites — only the impl
//!    behind the trait grows.
//! 3. **Future flexibility**: if we later split the backend into a separate
//!    process (e.g. for sandboxing), the trait is the natural seam.
//!
//! ## Method shape
//!
//! Every method is `async fn` and returns `Result<T>`. This matches the
//! Tauri-side `#[tauri::command] async fn` shape line-for-line, minus the
//! `app: AppHandle` / `state: State<...>` / `window: WebviewWindow`
//! framework params (which don't exist in-process). The inventory's 108
//! commands map 1:1 to trait methods in the same domain groupings.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod app_handle;
pub mod commands;
pub mod sessions;
pub mod storage;

pub use app_handle::AppHandle;

use crate::{error::Result, ssh::SshController, term::TermChunk};

pub struct ConnectedSshSession {
    pub controller: Arc<SshController>,
    pub output: tokio::sync::broadcast::Receiver<TermChunk>,
}

#[derive(Clone, Debug, Default)]
pub struct SshConnectOptions {
    pub accepted_host_fingerprint: Option<String>,
    pub save_host_fingerprint: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConnectionLibrary {
    pub profiles: Vec<Value>,
    pub folders: Vec<Value>,
}

/// In-process bridge between GPUI views and FileTerm backend services.
///
/// Method categories mirror `docs/plans/active/gpui-migration-inventory.md`
/// section 1's 14 groups. Methods land with the service and product view that
/// consume them, keeping unsupported capabilities visible at the bridge seam.
///
/// The trait is `Send + Sync` because it's held inside gpui entities
/// (`Arc<dyn FileTermDesktopApi>`) which must be `Send` for the foreground
/// executor's `Task`s to capture them.
#[async_trait]
pub trait FileTermDesktopApi: Send + Sync {
    // ===== G1 (storage fork) — inventory 1.1 / 1.14 =====
    // Profile-folder CRUD + local-files storage core land first because
    // every other feature reads from / writes to storage.

    /// Inventory 1.1 stub. Real signature lands in G1 when storage is forked.
    ///
    /// G0 returns `Unsupported` so view-layer scaffolding can compile
    /// against the trait without waiting for storage migration.
    async fn app_get_platform(&self) -> Result<String>;

    /// Read the shared Tauri/GPUI connection library with secrets redacted.
    async fn app_get_connection_library(&self) -> Result<ConnectionLibrary>;

    // ===== G2 (window/tray/menu) — inventory 1.2 / 1.6 =====
    // WindowRegistry + 7 window kinds + tray + native menu.

    /// Inventory 1.2 stub. Real signature lands in G2 with WindowRegistry.
    async fn workspace_list_windows(&self) -> Result<Vec<()>>;

    // ===== G3 (SSH terminal) — inventory 1.3 =====
    // russh shell channel + TermView + SystemSidebar + CWD 跟随.

    /// Open an authenticated SSH shell for a stored profile. Secrets remain
    /// inside the backend and are never projected into GPUI view state.
    async fn ssh_connect(
        &self,
        profile_id: &str,
        cols: u16,
        rows: u16,
        options: SshConnectOptions,
    ) -> Result<ConnectedSshSession>;

    // ===== G4 (SFTP + transfer) — inventory 1.4 / 1.5 =====
    // russh-sftp + TransferService + TransferCenter.

    /// Inventory 1.4 stub. Real signature lands in G4 with russh-sftp.
    async fn sftp_list(&self) -> Result<Vec<()>>;

    /// Inventory 1.5 stub. Real signature lands in G4 with TransferService.
    async fn transfer_create(&self) -> Result<()>;

    // ===== G5 (detach + release) — inventory 1.2 (detach) =====
    // detached-session windows + tab drag + 3-platform packaging.

    /// Inventory 1.2 (detach) stub. Real signature lands in G5.
    async fn workspace_detach_tab(&self) -> Result<()>;

    // ===== Lower-priority groups (P1/P2) — inventory 1.7–1.13 =====
    // tunnel / webdav / ssh-key / command-template / ui-state / update /
    // clipboard / external. These get their method declarations as each
    // phase needs them; G0 leaves them out entirely to keep the trait
    // surface minimal.
}

/// Concrete in-process desktop API.
///
/// Implemented capabilities delegate to framework-independent services;
/// remaining methods return an explicit phase-tagged `Unsupported` error.
pub struct GpuiDesktopApi {
    app: Arc<AppHandle>,
}

impl GpuiDesktopApi {
    pub fn new(app: Arc<AppHandle>) -> Self {
        Self { app }
    }

    pub fn app_handle(&self) -> &AppHandle {
        &self.app
    }
}

impl Default for GpuiDesktopApi {
    fn default() -> Self {
        let app = AppHandle::platform_default().expect("resolve FileTerm app data directory");
        Self::new(Arc::new(app))
    }
}

#[async_trait]
impl FileTermDesktopApi for GpuiDesktopApi {
    async fn app_get_platform(&self) -> Result<String> {
        Ok(commands::app_get_platform())
    }

    async fn app_get_connection_library(&self) -> Result<ConnectionLibrary> {
        let (profiles, folders) =
            crate::services::profile_ops::read_public_connection_library(&self.app)?;
        Ok(ConnectionLibrary { profiles, folders })
    }

    async fn workspace_list_windows(&self) -> Result<Vec<()>> {
        Err(crate::error::AppError::Unsupported(
            "workspace_list_windows (G2: WindowRegistry)",
        ))
    }

    async fn ssh_connect(
        &self,
        profile_id: &str,
        cols: u16,
        rows: u16,
        options: SshConnectOptions,
    ) -> Result<ConnectedSshSession> {
        let profile = crate::services::profile_ops::read_connection_profile(&self.app, profile_id)?;
        let host = profile
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let port = profile.get("port").and_then(Value::as_u64).unwrap_or(22) as u16;
        let changed = profile
            .get("trustedHostFingerprint")
            .and_then(Value::as_str)
            .is_some();
        let mut config =
            crate::services::ssh_profile::ssh_config_from_profile(&profile, cols, rows)
                .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
        if let Some(fingerprint) = options.accepted_host_fingerprint.as_ref() {
            config.trusted_host_fingerprint = Some(fingerprint.clone());
        }
        let (controller, output) = match SshController::connect(config).await {
            Ok(session) => session,
            Err(error) => {
                let message = error.to_string();
                if let Some(fingerprint) = message
                    .rsplit_once("server fingerprint: ")
                    .map(|(_, fingerprint)| fingerprint.trim().to_string())
                {
                    return Err(crate::error::AppError::SshHostVerification {
                        host,
                        port,
                        fingerprint,
                        changed,
                    });
                }
                return Err(crate::error::AppError::Command(message));
            }
        };
        if options.save_host_fingerprint {
            if let Some(fingerprint) = options.accepted_host_fingerprint.as_deref() {
                crate::services::profile_ops::update_trusted_host_fingerprint(
                    &self.app,
                    profile_id,
                    fingerprint,
                )?;
            }
        }
        Ok(ConnectedSshSession {
            controller: Arc::new(controller),
            output,
        })
    }

    async fn sftp_list(&self) -> Result<Vec<()>> {
        Err(crate::error::AppError::Unsupported(
            "sftp_list (G4: russh-sftp)",
        ))
    }

    async fn transfer_create(&self) -> Result<()> {
        Err(crate::error::AppError::Unsupported(
            "transfer_create (G4: TransferService)",
        ))
    }

    async fn workspace_detach_tab(&self) -> Result<()> {
        Err(crate::error::AppError::Unsupported(
            "workspace_detach_tab (G5: detach + release)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn implementation_is_usable_as_dyn_arc() {
        let api: Arc<dyn FileTermDesktopApi> = Arc::new(GpuiDesktopApi::default());
        assert_eq!(api.app_get_platform().await.unwrap(), std::env::consts::OS);
    }

    #[tokio::test]
    async fn connection_library_reads_shared_store_and_redacts_secrets() {
        let directory =
            std::env::temp_dir().join(format!("fileterm-gpui-library-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let app = Arc::new(AppHandle::new(directory.clone()));
        storage::write_json_array(
            &app,
            "folders.json",
            &[serde_json::json!({ "id": "folder-1", "name": "生产" })],
        )
        .unwrap();
        storage::write_json_array(
            &app,
            "profiles.json",
            &[serde_json::json!({
                "id": "profile-1",
                "type": "ssh",
                "name": "server",
                "host": "example.test",
                "port": 22,
                "username": "root",
                "group": "生产",
                "parentId": null
            })],
        )
        .unwrap();
        std::fs::write(
            directory.join("profile-secrets.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "profiles": {
                    "profile-1": { "password": { "value": "secret" } }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let library = GpuiDesktopApi::new(app)
            .app_get_connection_library()
            .await
            .unwrap();

        assert_eq!(library.profiles.len(), 1);
        assert_eq!(library.profiles[0]["parentId"], "folder-1");
        assert_eq!(library.profiles[0]["hasSavedPassword"], true);
        assert!(library.profiles[0].get("password").is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn remaining_unwired_methods_return_unsupported() {
        let api = GpuiDesktopApi::default();
        assert!(api.workspace_list_windows().await.is_err());
        assert!(api
            .ssh_connect("missing", 80, 24, SshConnectOptions::default())
            .await
            .is_err());
        assert!(api.sftp_list().await.is_err());
        assert!(api.transfer_create().await.is_err());
        assert!(api.workspace_detach_tab().await.is_err());
    }
}
