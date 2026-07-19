//! In-process API boundary used by GPUI views to reach persistent services and
//! create live protocol sessions.
//!
//! The trait keeps framework-specific rendering and window ownership out of
//! backend code while allowing tests to substitute the system boundary. Live
//! SFTP and transfer operations remain session-scoped because they require a
//! concrete authenticated controller rather than a process-global API.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroize;

pub mod app_handle;
pub mod commands;
pub mod sessions;
pub mod storage;

pub use app_handle::AppHandle;
pub use commands::{UiPreferences, UiPreferencesInput};

use crate::{
    error::Result,
    ftp::{FtpProfile, FtpSession},
    services::ssh_keys::{SshKeyFileSelection, SshKeyImportResult, SshKeyLayout, SshKeyMetadata},
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
        if let Some(password) = self.transient_password.as_mut() {
            password.zeroize();
        }
        if let Some(passphrase) = self.transient_passphrase.as_mut() {
            passphrase.zeroize();
        }
        self.transient_password = None;
        self.transient_passphrase = None;
        self.keyboard_interactive_answers.zeroize();
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
/// The trait is `Send + Sync` because views retain it in shared entities and
/// foreground tasks may capture it across await points.
#[async_trait]
pub trait FileTermDesktopApi: Send + Sync {
    async fn app_get_platform(&self) -> Result<String>;

    /// Read and write plain text through the operating-system clipboard.
    async fn app_read_clipboard_text(&self) -> Result<String>;
    async fn app_write_clipboard_text(&self, text: String) -> Result<()>;

    /// Open a validated HTTP(S) URL or the FileTerm logs directory.
    async fn app_open_external_url(&self, url: String) -> Result<()>;
    async fn app_open_logs_directory(&self) -> Result<()>;

    /// Read the shared Tauri/GPUI connection library with secrets redacted.
    async fn app_get_connection_library(&self) -> Result<ConnectionLibrary>;

    /// Create, update, or delete a persisted connection profile. Secret fields
    /// are split into the confidential profile store by the service layer.
    async fn app_create_connection_profile(&self, input: Value) -> Result<Value>;
    async fn app_update_connection_profile(
        &self,
        profile_id: String,
        input: Value,
    ) -> Result<Value>;
    async fn app_delete_connection_profile(&self, profile_id: String) -> Result<()>;

    /// Manage the process-local SSH key library. Private key bytes remain in
    /// the service and are never returned through this API.
    async fn ssh_keys_list(&self) -> Result<Vec<SshKeyMetadata>>;
    async fn ssh_keys_select_file(&self) -> Result<Option<SshKeyFileSelection>>;
    async fn ssh_keys_import(
        &self,
        source_path: String,
        note: String,
    ) -> Result<SshKeyImportResult>;
    async fn ssh_keys_update_note(&self, key_id: String, note: String) -> Result<SshKeyMetadata>;
    async fn ssh_keys_rename(&self, key_id: String, name: String) -> Result<SshKeyMetadata>;
    async fn ssh_keys_delete(&self, key_id: String) -> Result<()>;
    async fn ssh_keys_get_layout(&self) -> Result<SshKeyLayout>;
    async fn ssh_keys_save_layout(&self, layout: SshKeyLayout) -> Result<SshKeyLayout>;

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

/// Concrete in-process desktop API backed by the application data directory.
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

    async fn app_create_connection_profile(&self, input: Value) -> Result<Value> {
        crate::services::profile_ops::create_profile(&self.app, input)
    }

    async fn app_update_connection_profile(
        &self,
        profile_id: String,
        input: Value,
    ) -> Result<Value> {
        crate::services::profile_ops::update_profile(&self.app, &profile_id, input)
    }

    async fn app_delete_connection_profile(&self, profile_id: String) -> Result<()> {
        crate::services::profile_ops::delete_profile(&self.app, &profile_id)
    }

    async fn ssh_keys_list(&self) -> Result<Vec<SshKeyMetadata>> {
        crate::services::ssh_keys::list(&self.app)
    }

    async fn ssh_keys_select_file(&self) -> Result<Option<SshKeyFileSelection>> {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("导入 SSH 私钥")
            .pick_file()
            .await
        else {
            return Ok(None);
        };
        crate::services::ssh_keys::select_file(&self.app, file.path()).map(Some)
    }

    async fn ssh_keys_import(
        &self,
        source_path: String,
        note: String,
    ) -> Result<SshKeyImportResult> {
        crate::services::ssh_keys::import(&self.app, &source_path, &note)
    }

    async fn ssh_keys_update_note(&self, key_id: String, note: String) -> Result<SshKeyMetadata> {
        crate::services::ssh_keys::update_note(&self.app, &key_id, &note)
    }

    async fn ssh_keys_rename(&self, key_id: String, name: String) -> Result<SshKeyMetadata> {
        crate::services::ssh_keys::rename(&self.app, &key_id, &name)
    }

    async fn ssh_keys_delete(&self, key_id: String) -> Result<()> {
        crate::services::ssh_keys::delete(&self.app, &key_id)
    }

    async fn ssh_keys_get_layout(&self) -> Result<SshKeyLayout> {
        crate::services::ssh_keys::get_layout(&self.app)
    }

    async fn ssh_keys_save_layout(&self, layout: SshKeyLayout) -> Result<SshKeyLayout> {
        crate::services::ssh_keys::save_layout(&self.app, layout)
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

    #[test]
    fn ssh_connect_options_clear_all_transient_secrets() {
        let mut options = SshConnectOptions::default();
        options.transient_password = Some("password".to_string());
        options.transient_passphrase = Some("passphrase".to_string());
        options.keyboard_interactive_answers = vec!["otp".to_string()];

        options.clear_transient_secrets();

        assert!(options.transient_password.is_none());
        assert!(options.transient_passphrase.is_none());
        assert!(options.keyboard_interactive_answers.is_empty());
    }

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
    async fn connection_profile_crud_splits_secrets_and_heals_groups() {
        let directory =
            std::env::temp_dir().join(format!("fileterm-gpui-crud-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let app = Arc::new(AppHandle::new(directory.clone()));
        storage::write_json_array(
            &app,
            "folders.json",
            &[serde_json::json!({ "id": "folder-1", "name": "生产" })],
        )
        .unwrap();
        let api = GpuiDesktopApi::new(app.clone());

        let created = api
            .app_create_connection_profile(serde_json::json!({
                "type": "ssh",
                "name": "server",
                "host": "example.test",
                "port": 22,
                "username": "root",
                "password": "secret",
                "group": "生产"
            }))
            .await
            .unwrap();
        let profile_id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["parentId"], "folder-1");
        assert_eq!(created["hasSavedPassword"], true);
        assert!(created.get("password").is_none());

        let public_profiles: Vec<Value> = serde_json::from_str(
            &std::fs::read_to_string(directory.join("profiles.json")).unwrap(),
        )
        .unwrap();
        assert!(public_profiles[0].get("password").is_none());
        let secrets = storage::read_json_object(&app, "profile-secrets.json").unwrap();
        assert_eq!(
            secrets["profiles"][&profile_id]["password"]["value"],
            "secret"
        );

        let updated = api
            .app_update_connection_profile(
                profile_id.clone(),
                serde_json::json!({
                    "type": "ssh",
                    "name": "renamed",
                    "host": "example.test",
                    "port": 2222,
                    "username": "root",
                    "password": "",
                    "group": "不存在"
                }),
            )
            .await
            .unwrap();
        assert_eq!(updated["name"], "renamed");
        assert_eq!(updated["group"], "默认");
        assert_eq!(updated["hasSavedPassword"], true);

        api.app_delete_connection_profile(profile_id.clone())
            .await
            .unwrap();
        assert!(api
            .app_get_connection_library()
            .await
            .unwrap()
            .profiles
            .is_empty());
        let secrets = storage::read_json_object(&app, "profile-secrets.json").unwrap();
        assert!(secrets["profiles"].get(&profile_id).is_none());
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
