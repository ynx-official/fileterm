use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::{
    backend::AppHandle,
    services::ssh_keys,
    ssh::controller::{PrivateKeyCredential, SshConfig},
};

pub fn ssh_config_from_profile(
    app: &AppHandle,
    profile: &Value,
    cols: u16,
    rows: u16,
) -> Result<SshConfig> {
    if profile.get("type").and_then(Value::as_str) != Some("ssh") {
        bail!("connection profile is not SSH");
    }

    let host = required_string(profile, "host")?;
    let username = required_string(profile, "username")?;
    let port = profile
        .get("port")
        .and_then(Value::as_u64)
        .unwrap_or(22)
        .try_into()
        .context("SSH port is outside u16 range")?;
    let auth_type = profile
        .get("authType")
        .and_then(Value::as_str)
        .unwrap_or("password");

    let (password, private_keys) = match auth_type {
        "password" | "keyboard-interactive" => {
            (Some(required_string(profile, "password")?), Vec::new())
        }
        "privateKey" => {
            let profile_passphrase = optional_string(profile, "passphrase");
            let credential = if let Some(key_id) = optional_string(profile, "privateKeyId") {
                let managed = ssh_keys::resolve_managed_key(app, &key_id)?;
                PrivateKeyCredential {
                    private_key: managed.private_key,
                    passphrase: profile_passphrase.or(managed.passphrase),
                }
            } else {
                let path = expand_home(&required_string(profile, "privateKeyPath")?);
                let private_key = fs::read_to_string(&path)
                    .with_context(|| format!("read SSH private key {}", path.display()))?;
                PrivateKeyCredential {
                    private_key,
                    passphrase: profile_passphrase,
                }
            };
            (None, vec![credential])
        }
        "system" => {
            let keys = ssh_keys::discover_default_keys()
                .into_iter()
                .map(|key| PrivateKeyCredential {
                    private_key: key.private_key,
                    passphrase: key.passphrase,
                })
                .collect::<Vec<_>>();
            if keys.is_empty() {
                bail!("no default SSH private keys were found in ~/.ssh");
            }
            (None, keys)
        }
        other => bail!("unsupported SSH authentication type: {other}"),
    };

    Ok(SshConfig {
        host,
        port,
        username,
        password,
        private_keys,
        keyboard_interactive_answers: Vec::new(),
        trusted_host_fingerprint: optional_string(profile, "trustedHostFingerprint"),
        cols,
        rows,
    })
}

fn required_string(profile: &Value, key: &str) -> Result<String> {
    optional_string(profile, key).with_context(|| format!("SSH {key} is required"))
}

fn optional_string(profile: &Value, key: &str) -> Option<String> {
    profile
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_profile_maps_without_exposing_view_secrets() {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-ssh-profile-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let app = AppHandle::new(directory.clone());
        let config = ssh_config_from_profile(
            &app,
            &serde_json::json!({
                "type": "ssh",
                "host": "example.test",
                "port": 2222,
                "username": "deploy",
                "authType": "password",
                "password": "secret",
                "trustedHostFingerprint": "SHA256:test"
            }),
            100,
            30,
        )
        .unwrap();

        assert_eq!(config.host, "example.test");
        assert_eq!(config.port, 2222);
        assert_eq!(config.password.as_deref(), Some("secret"));
        assert_eq!(config.cols, 100);
        assert_eq!(config.rows, 30);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn non_ssh_profile_is_rejected() {
        let directory = std::env::temp_dir().join(format!(
            "fileterm-gpui-non-ssh-profile-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let result = ssh_config_from_profile(
            &AppHandle::new(directory.clone()),
            &serde_json::json!({ "type": "ftp", "host": "example.test" }),
            80,
            24,
        );
        assert!(result.is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
