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
pub use commands::{UiPreferences, UiPreferencesInput};

use crate::{
    error::Result,
    ftp::{FtpProfile, FtpSession},
    ssh::SshController,
    term::{SerialConfig, StreamController, TermChunk},
};

const MAX_SSH_AUTHENTICATION_ATTEMPTS: u8 = 3;

fn authentication_challenge(
    prompts: Vec<crate::error::SshAuthenticationPrompt>,
    attempts: u8,
) -> crate::error::AppError {
    if attempts >= MAX_SSH_AUTHENTICATION_ATTEMPTS {
        crate::error::AppError::Command(format!(
            "SSH authentication failed after {MAX_SSH_AUTHENTICATION_ATTEMPTS} attempts"
        ))
    } else {
        crate::error::AppError::SshAuthenticationRequired { prompts }
    }
}

pub struct ConnectedSshSession {
    pub controller: Arc<SshController>,
    pub output: tokio::sync::broadcast::Receiver<TermChunk>,
    pub transfer_journal_path: std::path::PathBuf,
}

pub struct ConnectedStreamSession {
    pub controller: Arc<StreamController>,
    pub protocol: String,
    pub endpoint: String,
}

pub struct ConnectedFtpSession {
    pub session: Arc<FtpSession>,
    pub remote_path: String,
    pub transfer_journal_path: std::path::PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct SshConnectOptions {
    pub accepted_host_fingerprint: Option<String>,
    pub save_host_fingerprint: bool,
    pub transient_password: Option<String>,
    pub transient_passphrase: Option<String>,
    pub keyboard_interactive_answers: Vec<String>,
    pub authentication_attempts: u8,
}

impl SshConnectOptions {
    pub(crate) fn clear_transient_secrets(&mut self) {
        self.transient_password = None;
        self.transient_passphrase = None;
        self.keyboard_interactive_answers.clear();
    }
}

impl Drop for SshConnectOptions {
    fn drop(&mut self) {
        self.clear_transient_secrets();
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConnectionLibrary {
    pub profiles: Vec<Value>,
    pub folders: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CommandLibrary {
    pub commands: Vec<Value>,
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

    /// Read and write plain text through the operating-system clipboard.
    async fn app_read_clipboard_text(&self) -> Result<String>;
    async fn app_write_clipboard_text(&self, text: String) -> Result<()>;

    /// Open a validated HTTP(S) URL or the FileTerm logs directory.
    async fn app_open_external_url(&self, url: String) -> Result<()>;
    async fn app_open_logs_directory(&self) -> Result<()>;

    /// Read the shared Tauri/GPUI connection library with secrets redacted.
    async fn app_get_connection_library(&self) -> Result<ConnectionLibrary>;

    /// Read product UI preferences from the shared runtime store.
    async fn app_get_ui_preferences(&self) -> Result<UiPreferences>;

    /// Persist validated product UI preferences to the shared runtime store.
    async fn app_set_ui_preferences(&self, input: UiPreferencesInput) -> Result<UiPreferences>;

    /// Read the shared command template library.
    async fn app_get_command_library(&self) -> Result<CommandLibrary>;

    /// Create or update one command template.
    async fn app_save_command_template(
        &self,
        command_id: Option<String>,
        command: String,
    ) -> Result<Value>;

    /// Delete one command template.
    async fn app_delete_command_template(&self, command_id: String) -> Result<()>;

    /// Read and persist WebDAV profile-bundle synchronization settings.
    async fn webdav_get_config(&self) -> Result<Value>;
    async fn webdav_save_config(&self, input: Value) -> Result<Value>;
    async fn webdav_upload(&self) -> Result<Value>;
    async fn webdav_download(&self) -> Result<Value>;

    // Window creation and tab placement deliberately stay in the GPUI view
    // layer because they require `&mut App`; protocol/file operations remain
    // behind session-scoped services with the concrete controller they need.

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

    /// Open a stored Telnet or Serial profile as a terminal byte stream.
    async fn stream_connect(&self, profile_id: &str) -> Result<ConnectedStreamSession>;

    /// Open a stored FTP/FTPS profile as a file session. The control channel
    /// stays session-scoped; transfers open dedicated authenticated channels.
    async fn ftp_connect(&self, profile_id: &str) -> Result<ConnectedFtpSession>;

    // SFTP and transfer operations are session-scoped services. Their API
    // accepts a live `SshController`/`SftpClient`; keeping parameterless bridge
    // methods here would create an invalid second ownership boundary.
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

    async fn app_read_clipboard_text(&self) -> Result<String> {
        commands::app_read_clipboard_text()
    }

    async fn app_write_clipboard_text(&self, text: String) -> Result<()> {
        commands::app_write_clipboard_text(text)
    }

    async fn app_open_external_url(&self, url: String) -> Result<()> {
        commands::app_open_external_url(url)
    }

    async fn app_open_logs_directory(&self) -> Result<()> {
        commands::app_open_logs_directory(&self.app)
    }

    async fn app_get_connection_library(&self) -> Result<ConnectionLibrary> {
        let (profiles, folders) =
            crate::services::profile_ops::read_public_connection_library(&self.app)?;
        Ok(ConnectionLibrary { profiles, folders })
    }

    async fn app_get_ui_preferences(&self) -> Result<UiPreferences> {
        commands::app_get_ui_preferences(&self.app)
    }

    async fn app_set_ui_preferences(&self, input: UiPreferencesInput) -> Result<UiPreferences> {
        commands::app_set_ui_preferences(&self.app, input)
    }

    async fn app_get_command_library(&self) -> Result<CommandLibrary> {
        let (commands, folders) = crate::services::profile_ops::read_command_library(&self.app)?;
        Ok(CommandLibrary { commands, folders })
    }

    async fn app_save_command_template(
        &self,
        command_id: Option<String>,
        command: String,
    ) -> Result<Value> {
        crate::services::profile_ops::save_command_template(
            &self.app,
            command_id.as_deref(),
            &command,
        )
    }

    async fn app_delete_command_template(&self, command_id: String) -> Result<()> {
        crate::services::profile_ops::delete_command_template(&self.app, &command_id)
    }

    async fn webdav_get_config(&self) -> Result<Value> {
        crate::services::webdav::get_config(&self.app)
    }

    async fn webdav_save_config(&self, input: Value) -> Result<Value> {
        crate::services::webdav::save_config(&self.app, input)
    }

    async fn webdav_upload(&self) -> Result<Value> {
        crate::services::webdav::upload(&self.app).await
    }

    async fn webdav_download(&self) -> Result<Value> {
        crate::services::webdav::download(&self.app).await
    }

    async fn ssh_connect(
        &self,
        profile_id: &str,
        cols: u16,
        rows: u16,
        options: SshConnectOptions,
    ) -> Result<ConnectedSshSession> {
        let mut profile =
            crate::services::profile_ops::read_connection_profile(&self.app, profile_id)?;
        let auth_type = profile
            .get("authType")
            .and_then(Value::as_str)
            .unwrap_or("password")
            .to_string();
        if matches!(auth_type.as_str(), "password" | "keyboard-interactive")
            && profile
                .get("password")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            && options.transient_password.is_none()
        {
            return Err(authentication_challenge(
                vec![crate::error::SshAuthenticationPrompt {
                    kind: crate::error::SshAuthenticationPromptKind::Password,
                    label: "Password".to_string(),
                    echo: false,
                }],
                options.authentication_attempts,
            ));
        }
        if let Some(object) = profile.as_object_mut() {
            if let Some(password) = options.transient_password.as_ref() {
                object.insert("password".to_string(), Value::String(password.clone()));
            }
            if let Some(passphrase) = options.transient_passphrase.as_ref() {
                object.insert("passphrase".to_string(), Value::String(passphrase.clone()));
            }
        }
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
            crate::services::ssh_profile::ssh_config_from_profile(&self.app, &profile, cols, rows)
                .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
        if auth_type == "privateKey"
            && config.private_keys.iter().any(|credential| {
                credential.passphrase.is_none()
                    && russh_keys::decode_secret_key(&credential.private_key, None).is_err()
            })
            && options.transient_passphrase.is_none()
        {
            return Err(authentication_challenge(
                vec![crate::error::SshAuthenticationPrompt {
                    kind: crate::error::SshAuthenticationPromptKind::PrivateKeyPassphrase,
                    label: "Private key passphrase".to_string(),
                    echo: false,
                }],
                options.authentication_attempts,
            ));
        }
        config.keyboard_interactive_answers = options.keyboard_interactive_answers.clone();
        if let Some(fingerprint) = options.accepted_host_fingerprint.as_ref() {
            config.trusted_host_fingerprint = Some(fingerprint.clone());
        }
        let (controller, output) = match SshController::connect(config).await {
            Ok(session) => session,
            Err(error) => {
                if let Some(challenge) =
                    error.downcast_ref::<crate::ssh::controller::SshAuthenticationChallenge>()
                {
                    return Err(authentication_challenge(
                        challenge.prompts.clone(),
                        options.authentication_attempts,
                    ));
                }
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
            transfer_journal_path: self.app.app_data_dir().join("transfer-journal.json"),
        })
    }

    async fn stream_connect(&self, profile_id: &str) -> Result<ConnectedStreamSession> {
        let profile = crate::services::profile_ops::read_connection_profile(&self.app, profile_id)?;
        let protocol = profile
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (controller, endpoint) = match protocol.as_str() {
            "telnet" => {
                let host = profile
                    .get("host")
                    .and_then(Value::as_str)
                    .filter(|host| !host.trim().is_empty())
                    .ok_or_else(|| {
                        crate::error::AppError::Command("Telnet host is required".into())
                    })?;
                let port = profile.get("port").and_then(Value::as_u64).unwrap_or(23) as u16;
                let controller = StreamController::connect_telnet(host, port)
                    .await
                    .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
                (controller, format!("{host}:{port}"))
            }
            "serial" => {
                let device_path = profile
                    .get("devicePath")
                    .and_then(Value::as_str)
                    .filter(|path| !path.trim().is_empty())
                    .ok_or_else(|| {
                        crate::error::AppError::Command("Serial device path is required".into())
                    })?
                    .to_string();
                let baud_rate = profile
                    .get("baudRate")
                    .and_then(Value::as_u64)
                    .unwrap_or(115_200) as u32;
                let controller = StreamController::connect_serial(SerialConfig {
                    device_path: device_path.clone(),
                    baud_rate,
                    data_bits: profile.get("dataBits").and_then(Value::as_u64).unwrap_or(8) as u8,
                    stop_bits: profile.get("stopBits").and_then(Value::as_u64).unwrap_or(1) as u8,
                    parity: profile
                        .get("parity")
                        .and_then(Value::as_str)
                        .unwrap_or("none")
                        .to_string(),
                    flow_control: profile
                        .get("flowControl")
                        .and_then(Value::as_str)
                        .unwrap_or("none")
                        .to_string(),
                })
                .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
                (controller, format!("{device_path} @ {baud_rate}"))
            }
            _ => {
                return Err(crate::error::AppError::Command(format!(
                    "profile {profile_id} is not a Telnet or Serial profile"
                )))
            }
        };
        Ok(ConnectedStreamSession {
            controller: Arc::new(controller),
            protocol,
            endpoint,
        })
    }

    async fn ftp_connect(&self, profile_id: &str) -> Result<ConnectedFtpSession> {
        let profile = crate::services::profile_ops::read_connection_profile(&self.app, profile_id)?;
        if profile.get("type").and_then(Value::as_str) != Some("ftp") {
            return Err(crate::error::AppError::Command(format!(
                "profile {profile_id} is not an FTP profile"
            )));
        }
        let profile = FtpProfile::from_value(&profile)
            .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
        let remote_path = profile.remote_path.clone();
        let session = FtpSession::connect(profile)
            .await
            .map_err(|error| crate::error::AppError::Command(error.to_string()))?;
        Ok(ConnectedFtpSession {
            session,
            remote_path,
            transfer_journal_path: self.app.app_data_dir().join("transfer-journal.json"),
        })
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
    async fn missing_ssh_profile_returns_error() {
        let api = GpuiDesktopApi::default();
        assert!(api
            .ssh_connect("missing", 80, 24, SshConnectOptions::default())
            .await
            .is_err());
    }
}
