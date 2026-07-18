//! SSH shell controller used by the GPUI terminal transport.

use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use russh::{client::Handler, ChannelMsg};
use russh_keys::PublicKeyBase64;
use russh_sftp::client::SftpSession;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

use crate::term::TermChunk;

const SSH_STAGE_TIMEOUT: Duration = Duration::from_secs(30);
const SHELL_CWD_SETUP: &str = "test -z \"${FISH_VERSION-}\" && eval '__tdcwd() { printf \"\\033]7;file://%s\\007\\033]1337;RemoteUser=%s\\007\" \"$(pwd -P 2>/dev/null)\" \"$(id -un 2>/dev/null)\"; }; if [ -n \"${ZSH_VERSION-}\" ]; then autoload -Uz add-zsh-hook 2>/dev/null; add-zsh-hook -D precmd __tdcwd 2>/dev/null; add-zsh-hook precmd __tdcwd 2>/dev/null; elif [ -n \"${BASH_VERSION-}\" ]; then case \"${PROMPT_COMMAND-}\" in *\"__tdcwd\"*) ;; *) PROMPT_COMMAND=\"__tdcwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}\" ;; esac; else case \"${PS1-}\" in *\"__tdcwd\"*) ;; *) PS1=\"\\$(__tdcwd)${PS1-}\" ;; esac; fi; __tdcwd'; stty echo 2>/dev/null\n";
const BUSYBOX_SHELL_CWD_SETUP: &str = "__tdcwd(){ printf '\\033]7;file://%s\\007\\033]1337;RemoteUser=%s\\007' \"$(pwd -P 2>/dev/null)\" \"$(id -un 2>/dev/null)\";};PS1='$(__tdcwd)'\"${PS1-}\";__tdcwd;stty echo 2>/dev/null\n";

#[derive(Debug, Clone)]
pub struct PrivateKeyCredential {
    pub private_key: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_keys: Vec<PrivateKeyCredential>,
    pub keyboard_interactive_answers: Vec<String>,
    pub trusted_host_fingerprint: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, thiserror::Error)]
#[error("SSH keyboard-interactive input required")]
pub struct SshAuthenticationChallenge {
    pub prompts: Vec<crate::error::SshAuthenticationPrompt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshSessionEvent {
    Connected,
    Closed,
    Error(String),
}

#[derive(Debug)]
enum SshCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

pub struct SshController {
    tx: broadcast::Sender<TermChunk>,
    event_tx: broadcast::Sender<SshSessionEvent>,
    command_tx: mpsc::UnboundedSender<SshCommand>,
    handle: Arc<russh::client::Handle<ClientHandler>>,
    _task: tokio::task::JoinHandle<()>,
}

impl SshController {
    pub async fn connect(config: SshConfig) -> Result<(Self, broadcast::Receiver<TermChunk>)> {
        validate_config(&config)?;

        let observed_fingerprint = Arc::new(parking_lot::Mutex::new(None));
        let handler = ClientHandler {
            host: config.host.clone(),
            port: config.port,
            trusted_fingerprint: config.trusted_host_fingerprint.clone(),
            observed_fingerprint: observed_fingerprint.clone(),
        };
        let client_config = Arc::new(russh::client::Config {
            inactivity_timeout: Some(Duration::from_secs(300)),
            ..Default::default()
        });

        let address = (config.host.as_str(), config.port);
        let mut handle = timeout(
            SSH_STAGE_TIMEOUT,
            russh::client::connect(client_config, address, handler),
        )
        .await
        .context("SSH handshake timed out")?
        .map_err(|error| {
            let fingerprint = observed_fingerprint.lock().clone();
            match fingerprint {
                Some(fingerprint) => anyhow::anyhow!(
                    "SSH handshake failed: {error}; server fingerprint: {fingerprint}"
                ),
                None => anyhow::anyhow!("SSH handshake failed: {error}"),
            }
        })?;

        authenticate(&mut handle, &config).await?;
        let platform = detect_remote_platform(&handle).await;
        let handle = Arc::new(handle);

        let cwd_setup = shell_cwd_setup_for_platform(&platform);
        let mut terminal_modes = vec![
            (russh::Pty::TTY_OP_ISPEED, 115200),
            (russh::Pty::TTY_OP_OSPEED, 115200),
        ];
        if cwd_setup.is_some() {
            terminal_modes.push((russh::Pty::ECHO, 0));
        }

        let mut channel = timeout(SSH_STAGE_TIMEOUT, handle.channel_open_session())
            .await
            .context("SSH shell channel timed out")?
            .context("open SSH shell channel")?;
        channel
            .request_pty(
                true,
                "xterm-256color",
                config.cols.into(),
                config.rows.into(),
                0,
                0,
                &terminal_modes,
            )
            .await
            .context("request SSH pty")?;
        channel
            .request_shell(true)
            .await
            .context("request SSH shell")?;

        let (tx, rx) = broadcast::channel(256);
        let (event_tx, _) = broadcast::channel(32);
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let output_tx = tx.clone();
        let output_event_tx = event_tx.clone();
        let task_handle = handle.clone();

        let task = tokio::spawn(async move {
            let mut seq = 0u64;
            let _ = output_event_tx.send(SshSessionEvent::Connected);

            if let Some(setup) = cwd_setup {
                let _ = channel.data(setup.as_bytes()).await;
            }

            loop {
                tokio::select! {
                    message = channel.wait() => {
                        match message {
                            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                                seq = seq.wrapping_add(1);
                                let _ = output_tx.send(TermChunk { seq, bytes: data.to_vec() });
                            }
                            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                                let _ = output_event_tx.send(SshSessionEvent::Closed);
                                break;
                            }
                            _ => {}
                        }
                    }
                    command = command_rx.recv() => {
                        let result = match command {
                            Some(SshCommand::Input(bytes)) => channel.data(bytes.as_slice()).await,
                            Some(SshCommand::Resize { cols, rows }) => {
                                channel.window_change(cols.into(), rows.into(), 0, 0).await
                            }
                            Some(SshCommand::Shutdown) | None => {
                                let _ = channel.eof().await;
                                let _ = channel.close().await;
                                break;
                            }
                        };
                        if let Err(error) = result {
                            let _ = output_event_tx.send(SshSessionEvent::Error(error.to_string()));
                            break;
                        }
                    }
                }
            }
            let _ = task_handle
                .disconnect(
                    russh::Disconnect::ByApplication,
                    "FileTerm session closed",
                    "",
                )
                .await;
            drop(task_handle);
        });

        Ok((
            Self {
                tx,
                event_tx,
                command_tx,
                handle,
                _task: task,
            },
            rx,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<SshSessionEvent> {
        self.event_tx.subscribe()
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.command_tx
            .send(SshCommand::Input(bytes.to_vec()))
            .context("SSH session closed")
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.command_tx
            .send(SshCommand::Resize { cols, rows })
            .context("SSH session closed")
    }

    /// Request a graceful shell/channel shutdown. The background task sends
    /// EOF, closes the channel and disconnects the SSH transport.
    pub fn shutdown(&self) {
        let _ = self.command_tx.send(SshCommand::Shutdown);
    }

    pub async fn exec(&self, command: &str) -> Result<Vec<u8>> {
        let mut channel = timeout(SSH_STAGE_TIMEOUT, self.handle.channel_open_session())
            .await
            .context("SSH exec channel timed out")?
            .context("open SSH exec channel")?;
        channel
            .exec(true, command)
            .await
            .with_context(|| format!("execute remote command: {command}"))?;

        let mut output = Vec::new();
        loop {
            match timeout(SSH_STAGE_TIMEOUT, channel.wait()).await {
                Ok(Some(ChannelMsg::Data { data }))
                | Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    output.extend_from_slice(&data);
                }
                Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => break,
                Ok(Some(_)) => {}
                Err(_) => bail!("remote command timed out: {command}"),
            }
        }
        Ok(output)
    }

    pub async fn open_sftp(&self) -> Result<SftpSession> {
        let channel = timeout(SSH_STAGE_TIMEOUT, self.handle.channel_open_session())
            .await
            .context("SFTP channel timed out")?
            .context("open SFTP channel")?;
        timeout(SSH_STAGE_TIMEOUT, channel.request_subsystem(true, "sftp"))
            .await
            .context("SFTP subsystem request timed out")?
            .context("request SFTP subsystem")?;
        timeout(SSH_STAGE_TIMEOUT, SftpSession::new(channel.into_stream()))
            .await
            .context("SFTP protocol handshake timed out")?
            .context("initialize SFTP session")
    }
}

impl Drop for SshController {
    fn drop(&mut self) {
        let _ = self.command_tx.send(SshCommand::Shutdown);
    }
}

fn validate_config(config: &SshConfig) -> Result<()> {
    if config.host.trim().is_empty() {
        bail!("SSH host is required");
    }
    if config.username.trim().is_empty() {
        bail!("SSH username is required");
    }
    if config.password.is_none() && config.private_keys.is_empty() {
        bail!("configure at least one SSH authentication method");
    }
    Ok(())
}

async fn authenticate(
    handle: &mut russh::client::Handle<ClientHandler>,
    config: &SshConfig,
) -> Result<()> {
    let none_accepted = timeout(
        SSH_STAGE_TIMEOUT,
        handle.authenticate_none(config.username.clone()),
    )
    .await
    .context("SSH authentication negotiation timed out")?
    .context("SSH authentication negotiation")?;
    if none_accepted {
        return Ok(());
    }

    let mut key_decode_errors = Vec::new();
    for credential in &config.private_keys {
        let private_key = match russh_keys::decode_secret_key(
            &credential.private_key,
            credential.passphrase.as_deref(),
        ) {
            Ok(private_key) => private_key,
            Err(error) => {
                key_decode_errors.push(error.to_string());
                continue;
            }
        };
        let accepted = timeout(
            SSH_STAGE_TIMEOUT,
            handle.authenticate_publickey(config.username.clone(), Arc::new(private_key)),
        )
        .await
        .context("SSH public-key authentication timed out")?
        .context("SSH public-key authentication")?;
        if accepted {
            return Ok(());
        }
    }

    if let Some(password) = config.password.as_deref() {
        let accepted = timeout(
            SSH_STAGE_TIMEOUT,
            handle.authenticate_password(config.username.clone(), password),
        )
        .await
        .context("SSH password authentication timed out")?
        .context("SSH password authentication")?;
        if accepted {
            return Ok(());
        }
    }

    if try_keyboard_interactive(
        handle,
        &config.username,
        config.password.as_deref().unwrap_or_default(),
        &config.keyboard_interactive_answers,
    )
    .await?
    {
        return Ok(());
    }

    if config.password.is_none() && !key_decode_errors.is_empty() {
        bail!(
            "could not decode any SSH private key: {}",
            key_decode_errors.join("; ")
        );
    }
    bail!("SSH authentication rejected")
}

async fn try_keyboard_interactive(
    handle: &mut russh::client::Handle<ClientHandler>,
    username: &str,
    password: &str,
    supplied_answers: &[String],
) -> Result<bool> {
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut response = timeout(
        SSH_STAGE_TIMEOUT,
        handle.authenticate_keyboard_interactive_start(username, None),
    )
    .await
    .context("SSH keyboard-interactive authentication timed out")?
    .context("start SSH keyboard-interactive authentication")?;
    let mut supplied_answers = supplied_answers.iter();
    let mut password_used = false;
    let mut last_prompts = Vec::new();

    for _ in 0..16 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure => {
                if last_prompts.is_empty() {
                    return Ok(false);
                }
                return Err(anyhow::Error::new(SshAuthenticationChallenge {
                    prompts: last_prompts,
                }));
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                last_prompts = prompts
                    .iter()
                    .map(|prompt| crate::error::SshAuthenticationPrompt {
                        kind: crate::error::SshAuthenticationPromptKind::KeyboardInteractive,
                        label: prompt.prompt.clone(),
                        echo: prompt.echo,
                    })
                    .collect();
                let mut answers = Vec::with_capacity(prompts.len());
                let mut missing = Vec::new();
                for prompt in prompts {
                    if !password_used && !password.is_empty() && is_password_prompt(&prompt.prompt)
                    {
                        answers.push(password.to_string());
                        password_used = true;
                    } else if let Some(answer) = supplied_answers.next() {
                        answers.push(answer.clone());
                    } else {
                        missing.push(crate::error::SshAuthenticationPrompt {
                            kind: crate::error::SshAuthenticationPromptKind::KeyboardInteractive,
                            label: prompt.prompt,
                            echo: prompt.echo,
                        });
                    }
                }
                if !missing.is_empty() {
                    return Err(anyhow::Error::new(SshAuthenticationChallenge {
                        prompts: missing,
                    }));
                }
                response = timeout(
                    SSH_STAGE_TIMEOUT,
                    handle.authenticate_keyboard_interactive_respond(answers),
                )
                .await
                .context("SSH keyboard-interactive response timed out")?
                .context("respond to SSH keyboard-interactive authentication")?;
            }
        }
    }
    Ok(false)
}

fn is_password_prompt(prompt: &str) -> bool {
    let normalized = prompt.to_ascii_lowercase();
    ![
        "code",
        "otp",
        "mfa",
        "2fa",
        "factor",
        "duo",
        "verification",
        "token",
        "验证码",
        "动态码",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
        && (normalized.contains("password") || normalized.contains("密码"))
}

async fn detect_remote_platform(handle: &russh::client::Handle<ClientHandler>) -> String {
    let Ok(mut channel) = handle.channel_open_session().await else {
        return "unknown".to_string();
    };
    if channel
        .exec(
            true,
            "uname -s 2>/dev/null; command -v busybox >/dev/null 2>&1 && echo busybox",
        )
        .await
        .is_err()
    {
        return "unknown".to_string();
    }

    let mut output = Vec::new();
    while let Ok(Some(message)) = timeout(Duration::from_secs(3), channel.wait()).await {
        match message {
            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                output.extend_from_slice(&data);
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    let normalized = String::from_utf8_lossy(&output)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("busybox") {
        "busybox".to_string()
    } else if lower.contains("linux") {
        "linux".to_string()
    } else {
        "unknown".to_string()
    }
}

fn shell_cwd_setup_for_platform(platform: &str) -> Option<&'static str> {
    match platform {
        "linux" => Some(SHELL_CWD_SETUP),
        "busybox" => Some(BUSYBOX_SHELL_CWD_SETUP),
        _ => None,
    }
}

fn fingerprint_sha256(key: &russh_keys::key::PublicKey) -> String {
    let digest = Sha256::digest(key.public_key_bytes());
    format!("SHA256:{}", STANDARD_NO_PAD.encode(digest))
}

struct ClientHandler {
    host: String,
    port: u16,
    trusted_fingerprint: Option<String>,
    observed_fingerprint: Arc<parking_lot::Mutex<Option<String>>>,
}

#[async_trait]
impl Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<(Self, bool), Self::Error> {
        let fingerprint = fingerprint_sha256(server_public_key);
        *self.observed_fingerprint.lock() = Some(fingerprint.clone());

        let accepted = match self.trusted_fingerprint.as_deref() {
            Some(trusted) => trusted == fingerprint,
            None => russh_keys::check_known_hosts(&self.host, self.port, server_public_key)
                .unwrap_or(false),
        };
        Ok((self, accepted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SshConfig {
        SshConfig {
            host: "localhost".into(),
            port: 22,
            username: "user".into(),
            password: Some("pass".into()),
            private_keys: Vec::new(),
            keyboard_interactive_answers: Vec::new(),
            trusted_host_fingerprint: None,
            cols: 80,
            rows: 24,
        }
    }

    #[test]
    fn config_requires_at_least_one_authentication_method() {
        assert!(validate_config(&config()).is_ok());
        let mut fallback = config();
        fallback.private_keys.push(PrivateKeyCredential {
            private_key: "key".into(),
            passphrase: None,
        });
        assert!(validate_config(&fallback).is_ok());
        fallback.password = None;
        assert!(validate_config(&fallback).is_ok());
        fallback.private_keys.clear();
        assert!(validate_config(&fallback).is_err());
    }

    #[test]
    fn password_prompt_detection_rejects_mfa_prompts() {
        assert!(is_password_prompt("Password:"));
        assert!(is_password_prompt("请输入密码"));
        assert!(!is_password_prompt("Verification code:"));
        assert!(!is_password_prompt("Duo MFA token:"));
    }

    #[test]
    fn cwd_setup_is_fail_closed_for_unknown_platforms() {
        assert!(shell_cwd_setup_for_platform("linux").is_some());
        assert!(shell_cwd_setup_for_platform("busybox").is_some());
        assert!(shell_cwd_setup_for_platform("windows").is_none());
        assert!(shell_cwd_setup_for_platform("unknown").is_none());
    }
}
