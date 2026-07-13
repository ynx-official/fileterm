// SSH worker based on russh (pure-Rust async SSH implementation).
//
// Migration from ssh2 (libssh2 C bindings) to russh 0.62 was performed to:
//  1. Enable true in-handshake host key verification via async
//     `check_server_key` handler (the renderer can prompt the user while
//     the handshake is in flight, and accept/reject before it completes).
//  2. Support MFA multi-prompt keyboard-interactive flows.
//  3. Drop the `vendored-openssl` C dependency and unify the build across
//     macOS / Windows / Linux.
//  4. Move from a manual `set_blocking(true/false)` juggle to a native
//     tokio task per session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use russh::client::{Handle, Handler};
use russh::keys::PrivateKeyWithHashAlg;
use russh::{Channel, ChannelMsg};
use russh_sftp::client::SftpSession;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot, RwLock};

use super::WorkerCmd;

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Merge network sample history from the previous metrics into the next.
///
/// Mirrors `mergeSystemMetricsHistory` from `packages/core` so the session
/// snapshot retains the rolling `networkSamples` / `networkSamplesByInterface`
/// history. Other fields (cpu, memory, etc.) are taken from `next` verbatim.
fn merge_system_metrics_history(
    previous: Option<&serde_json::Value>,
    next: serde_json::Value,
    history_limit: usize,
) -> serde_json::Value {
    let mut merged = next.clone();
    if let Some(prev) = previous {
        let prev_samples = prev
            .get("networkSamples")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let next_point = next
            .get("networkSamples")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .cloned()
            .unwrap_or(serde_json::json!({ "rx": 0, "tx": 0 }));

        let mut combined = prev_samples;
        combined.push(next_point);
        if combined.len() > history_limit {
            combined = combined[combined.len() - history_limit..].to_vec();
        }
        merged["networkSamples"] = serde_json::Value::Array(combined);

        // Per-interface accumulation
        let prev_by_iface = prev
            .get("networkSamplesByInterface")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        if let Some(next_by_iface) = next
            .get("networkSamplesByInterface")
            .and_then(|v| v.as_object())
            .cloned()
        {
            let mut merged_by_iface = serde_json::Map::new();
            for (name, samples_val) in next_by_iface.iter() {
                let next_iface_point = samples_val
                    .as_array()
                    .and_then(|arr| arr.last())
                    .cloned()
                    .unwrap_or(serde_json::json!({ "rx": 0, "tx": 0 }));
                let prev_iface_samples = prev_by_iface
                    .get(name)
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut combined = prev_iface_samples;
                combined.push(next_iface_point);
                if combined.len() > history_limit {
                    combined = combined[combined.len() - history_limit..].to_vec();
                }
                merged_by_iface.insert(name.clone(), serde_json::Value::Array(combined));
            }
            merged["networkSamplesByInterface"] = serde_json::Value::Object(merged_by_iface);
        }
    }
    merged
}

pub fn start_ssh_worker(
    tab_id: String,
    profile: Value,
    mut cmd_rx: mpsc::Receiver<WorkerCmd>,
    app: AppHandle,
) {
    tokio::spawn(async move {
        let tid = tab_id.clone();
        // The initial "连接主机...\r\n" notice is already in the session
        // snapshot's `terminal_transcript` (set by `app_open_profile`), so
        // the renderer hydrates it via `bootText` — no need to emit it here.
        // Emitting here would race the renderer's listener registration.
        match run_worker_loop(&tab_id, &profile, &mut cmd_rx, &app).await {
            Ok(()) => {
                emit_terminal_data(&app, &tid, "连接已断开\r\n").await;
            }
            Err(e) => {
                eprintln!("[SSH Worker] error for tab {}: {}", tid, e);
                emit_terminal_data(&app, &tid, &format!("连接失败: {}\r\n", e)).await;
            }
        }
        update_tab_status_and_emit(&app, &tid, "disconnected").await;
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler implementation
// ─────────────────────────────────────────────────────────────────────────────

pub struct ClientHandler {
    app: AppHandle,
    tab_id: String,
    profile_id: String,
    host: String,
    port: u16,
    trusted_fingerprint: Option<String>,
}

pub type ClientHandle = Handle<ClientHandler>;

impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = fingerprint_sha256_base64(server_public_key);
        eprintln!(
            "[SSH host-key] tab={} profile={} host={} fp='{}' trusted={:?}",
            self.tab_id, self.profile_id, self.host, fp, self.trusted_fingerprint
        );
        // Short-circuit: if the profile already trusts this exact
        // fingerprint, accept without prompting. This is the common path
        // after the user previously chose "accept-and-save".
        if let Some(known) = &self.trusted_fingerprint {
            eprintln!(
                "[SSH host-key] comparing known='{}' (len={}) vs fp='{}' (len={}) equal={}",
                known,
                known.len(),
                fp,
                fp.len(),
                known == &fp
            );
            if known == &fp {
                return Ok(true);
            }
            eprintln!(
                "[SSH host-key] mismatch — byte diff: known_bytes={:?} fp_bytes={:?}",
                known.as_bytes(),
                fp.as_bytes()
            );
        }
        let known = self.trusted_fingerprint.clone();
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<Value>();
        {
            let state = self
                .app
                .state::<crate::services::workspace::WorkspaceState>();
            let mut pending = state.pending_interactions.write().await;
            pending.insert(request_id.clone(), tx);
        }
        // Emit a `host-verification` interaction request. The payload shape
        // matches `SshHostVerificationRequest` in packages/core so the
        // renderer's `useSshInteractions` hook recognises it and shows the
        // accept/reject dialog. The renderer resolves via
        // `app_resolve_ssh_interaction`, which forwards the response back
        // through the oneshot channel.
        let _ = self.app.emit(
            "ssh:interaction",
            serde_json::json!({
                "requestId": request_id,
                "kind": "host-verification",
                "tabId": self.tab_id,
                "profileId": self.profile_id,
                "host": self.host,
                "port": self.port,
                "fingerprint": fp,
                "knownFingerprint": known,
            }),
        );
        let decision = match rx.await {
            Ok(response) => response
                .get("decision")
                .and_then(|v| v.as_str())
                .unwrap_or("cancel")
                .to_string(),
            Err(_) => "cancel".to_string(),
        };
        match decision.as_str() {
            "accept-and-save" => {
                // Persist the trusted fingerprint so future connects
                // short-circuit the prompt.
                let _ = crate::services::profile_ops::update_trusted_host_fingerprint(
                    &self.app,
                    &self.profile_id,
                    &fp,
                )
                .await;
                self.trusted_fingerprint = Some(fp);
                Ok(true)
            }
            "accept-once" => Ok(true),
            _ => Ok(false),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker loop
// ─────────────────────────────────────────────────────────────────────────────

async fn update_tab_status_and_emit(app: &AppHandle, tab_id: &str, status: &str) {
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let connected = status == "connected";
    let mut summary = "连接已断开".to_string();
    let mut transcript = String::new();
    {
        let mut tabs = state.tabs.write().await;
        if let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.status = status.to_string();
        }
    }
    {
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(tab_id) {
            session.connected = connected;
            summary = session.summary.clone();
            transcript = session.terminal_transcript.clone();
        }
    }
    let payload = serde_json::json!({
        "tabId": tab_id.to_string(),
        "summary": summary,
        "transcript": transcript,
        "connected": connected,
    });
    let _ = app.emit("terminal:state", payload);

    if let Ok(snapshot) = crate::commands::get_workspace_snapshot(app.clone()).await {
        let _ = app.emit("workspace:snapshot", snapshot);
    }
}

/// Emit a terminal data chunk to the renderer and append it to the session
/// snapshot's `terminal_transcript` so later `terminal:state` / snapshot
/// refreshes surface the full history (handles the case where the renderer
/// missed the live `terminal:data` event, e.g. during a fast-fail connect).
async fn emit_terminal_data(app: &AppHandle, tab_id: &str, chunk: &str) {
    let _ = app.emit(
        "terminal:data",
        serde_json::json!({
            "tabId": tab_id,
            "chunk": chunk,
        }),
    );
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    let mut sessions = state.sessions.write().await;
    if let Some(s) = sessions.get_mut(tab_id) {
        s.terminal_transcript.push_str(chunk);
        // Cap transcript to 200k chars (matches Electron's BoundedTextBuffer).
        if s.terminal_transcript.len() > 200_000 {
            let cut = s.terminal_transcript.len() - 180_000;
            s.terminal_transcript = s.terminal_transcript[cut..].to_string();
        }
    }
}

/// Flush the batch buffer to the renderer and append to the session
/// transcript in one step. Used by every flush path in the main event
/// loop (cmd exit, shell close, 16ms timer) so they stay consistent.
async fn flush_batch(batch: &mut Vec<u8>, app: &AppHandle, tab_id: &str) {
    if batch.is_empty() {
        return;
    }
    let chunk = String::from_utf8_lossy(batch).into_owned();
    batch.clear();
    emit_terminal_data(app, tab_id, &chunk).await;
}

fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                result.push(hex);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn track_cwd_and_user(chunk: &str, buffer: &mut String) -> (Option<String>, Option<String>) {
    buffer.push_str(chunk);
    if buffer.len() > 8192 {
        *buffer = buffer[buffer.len() - 4096..].to_string();
    }

    let mut cwd = None;
    let mut user = None;

    let re_cwd =
        regex::Regex::new(r"\x1b\]7;file://([^\x07\x1b]*)(?:\x07|\x1b\\)").unwrap();
    let re_user =
        regex::Regex::new(r"\x1b\]1337;RemoteUser=([^\x07\x1b]*)(?:\x07|\x1b\\)").unwrap();

    for cap in re_cwd.captures_iter(buffer) {
        let raw_path = &cap[1];
        if let Some(slash_idx) = raw_path.find('/') {
            let path_part = &raw_path[slash_idx..];
            cwd = Some(percent_decode(path_part));
        }
    }
    for cap in re_user.captures_iter(buffer) {
        user = Some(cap[1].to_string());
    }
    (cwd, user)
}

/// Compute the OpenSSH-style SHA256 fingerprint of a host key.
///
/// Matches Electron's `computeHostFingerprint`:
/// `SHA256:` + base64(sha256(ssh_wire_encoded_public_key)) with `=` padding
/// stripped. The `ssh-key` crate's `Fingerprint` `Display` impl produces
/// exactly this format, so we defer to it instead of re-encoding manually.
fn fingerprint_sha256_base64(key: &russh::keys::PublicKey) -> String {
    format!("{}", key.fingerprint(russh::keys::HashAlg::Sha256))
}

/// Open an SSH session using the profile credentials. `trusted_fingerprint`
/// flows into the Handler's `check_server_key` so it can short-circuit the
/// accept/reject prompt when the fingerprint already matches.
async fn open_session(
    profile: &Value,
    app: &AppHandle,
    tab_id: &str,
) -> Result<Handle<ClientHandler>, String> {
    let host = profile
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or("127.0.0.1")
        .to_string();
    let port = profile.get("port").and_then(|p| p.as_i64()).unwrap_or(22) as u16;
    let username = profile
        .get("username")
        .and_then(|u| u.as_str())
        .unwrap_or("root")
        .to_string();
    let auth_type = profile
        .get("authType")
        .and_then(|a| a.as_str())
        .unwrap_or("password")
        .to_string();
    let trusted = profile
        .get("trustedHostFingerprint")
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());
    eprintln!(
        "[SSH host-key] open_session tab={} profile_id='{}' trustedHostFingerprint_from_profile={:?}",
        tab_id,
        profile.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        trusted
    );

    let profile_id = profile
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    let handler = ClientHandler {
        app: app.clone(),
        tab_id: tab_id.to_string(),
        profile_id,
        host: host.clone(),
        port,
        trusted_fingerprint: trusted,
    };

    let mut config = russh::client::Config::default();
    config.inactivity_timeout = Some(Duration::from_secs(300));
    let config = Arc::new(config);

    let addr = format!("{}:{}", host, port);
    let mut handle = russh::client::connect(config, &addr, handler)
        .await
        .map_err(|e| format!("SSH connect failed: {}", e))?;

    let authed = try_authenticate(&mut handle, &username, &auth_type, profile, app, tab_id).await?;
    if !authed {
        return Err("SSH Authentication failed".to_string());
    }
    Ok(handle)
}

async fn try_authenticate(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    auth_type: &str,
    profile: &Value,
    app: &AppHandle,
    tab_id: &str,
) -> Result<bool, String> {
    let host = profile
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string();
    let port = profile.get("port").and_then(|p| p.as_i64()).unwrap_or(22) as u16;
    let profile_id = profile
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    match auth_type {
        "password" => {
            let password = profile.get("password").and_then(|p| p.as_str()).unwrap_or("");
            let res = handle
                .authenticate_password(username, password)
                .await
                .map_err(|e| e.to_string())?;
            if res.success() {
                return Ok(true);
            }
            // Fallback: keyboard-interactive with the same password.
            let password_owned = password.to_string();
            try_keyboard_interactive(
                handle,
                username,
                &password_owned,
                app,
                tab_id,
                &profile_id,
                &host,
                port,
            )
            .await
        }
        "privateKey" => {
            let private_key_path = profile
                .get("privateKeyPath")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let passphrase = profile.get("passphrase").and_then(|p| p.as_str());

            let mut resolved = private_key_path.to_string();
            if resolved.starts_with("~/") || resolved == "~" {
                if let Ok(home) = app.path().home_dir() {
                    let rest = if resolved == "~" { "" } else { &resolved[2..] };
                    resolved = home.join(rest).to_string_lossy().into_owned();
                }
            }

            // Try to read the key from disk, then authenticate via memory key.
            let key_content = std::fs::read_to_string(&resolved).map_err(|e| e.to_string())?;
            let key_pair = russh::keys::decode_secret_key(&key_content, passphrase)
                .map_err(|e| e.to_string())?;
            // Best-effort: pick the strongest RSA hash the server advertises.
            // For non-RSA keys, hash_alg is ignored by PrivateKeyWithHashAlg::new.
            let hash_alg: Option<russh::keys::HashAlg> = if key_pair.algorithm().is_rsa() {
                match handle.best_supported_rsa_hash().await {
                    Ok(Some(Some(h))) => Some(h),
                    _ => Some(russh::keys::HashAlg::Sha512),
                }
            } else {
                None
            };
            let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg);
            let res = handle
                .authenticate_publickey(username, key_with_hash)
                .await
                .map_err(|e| e.to_string())?;
            if res.success() {
                return Ok(true);
            }
            // Fallback to keyboard-interactive if a password is present.
            if let Some(password) = profile.get("password").and_then(|p| p.as_str()) {
                return try_keyboard_interactive(
                    handle,
                    username,
                    password,
                    app,
                    tab_id,
                    &profile_id,
                    &host,
                    port,
                )
                .await;
            }
            Ok(false)
        }
        _ => {
            // agent
            let mut agent = russh::keys::agent::client::AgentClient::connect_env()
                .await
                .map_err(|e| e.to_string())?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|e| e.to_string())?;
            for identity in identities {
                let pub_key = identity.public_key().into_owned();
                let res = handle
                    .authenticate_publickey_with(username, pub_key, None, &mut agent)
                    .await
                    .map_err(|e| e.to_string())?;
                if res.success() {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

async fn try_keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    password: &str,
    app: &AppHandle,
    tab_id: &str,
    profile_id: &str,
    host: &str,
    port: u16,
) -> Result<bool, String> {
    let password_owned = password.to_string();
    let res = handle
        .authenticate_keyboard_interactive_start(username, None)
        .await
        .map_err(|e| e.to_string())?;

    let mut current = match res {
        russh::client::KeyboardInteractiveAuthResponse::Success => return Ok(true),
        russh::client::KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
        russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
            name,
            instructions,
            prompts,
        } => (name, instructions, prompts),
    };

    // Multi-round OTP loop: keep answering prompts until success/failure or a
    // user-initiated cancel from the renderer. The first round answers every
    // prompt with the configured password. Subsequent rounds emit
    // `ssh:interaction` events with `kind: "keyboard-interactive"` and await
    // the user. The payload matches `SshKeyboardInteractiveRequest` in
    // packages/core so the renderer's `useSshInteractions` hook shows the
    // MFA prompt dialog.
    let mut first_round = true;
    loop {
        let answers: Vec<String> = if first_round {
            first_round = false;
            current.2.iter().map(|_| password_owned.clone()).collect()
        } else {
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = oneshot::channel::<Value>();
            {
                let state = app
                    .state::<crate::services::workspace::WorkspaceState>();
                let mut pending = state.pending_interactions.write().await;
                pending.insert(request_id.clone(), tx);
            }
            let _ = app.emit(
                "ssh:interaction",
                serde_json::json!({
                    "requestId": request_id,
                    "kind": "keyboard-interactive",
                    "tabId": tab_id,
                    "profileId": profile_id,
                    "host": host,
                    "port": port,
                    "name": current.0,
                    "instructions": current.1,
                    "prompts": current.2.iter().map(|p| {
                        serde_json::json!({ "prompt": p.prompt, "echo": p.echo })
                    }).collect::<Vec<_>>(),
                }),
            );
            match rx.await {
                Ok(response) => {
                    if response
                        .get("canceled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        return Ok(false);
                    }
                    response
                        .get("answers")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .map(|v| v.as_str().unwrap_or("").to_string())
                                .collect()
                        })
                        .unwrap_or_default()
                }
                Err(_) => return Ok(false),
            }
        };

        let res = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await
            .map_err(|e| e.to_string())?;
        current = match res {
            russh::client::KeyboardInteractiveAuthResponse::Success => return Ok(true),
            russh::client::KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => (name, instructions, prompts),
        };
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    for i in 0..=haystack.len() - needle.len() {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i);
        }
    }
    None
}

async fn run_worker_loop(
    tab_id: &str,
    profile: &Value,
    cmd_rx: &mut mpsc::Receiver<WorkerCmd>,
    app: &AppHandle,
) -> Result<(), String> {
    let host = profile
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or("127.0.0.1")
        .to_string();
    let port = profile.get("port").and_then(|p| p.as_i64()).unwrap_or(22) as u16;
    let username = profile
        .get("username")
        .and_then(|u| u.as_str())
        .unwrap_or("root")
        .to_string();

    // ── Main session (single SSH session multiplexes shell + SFTP + metrics) ─
    // Servers with strict MaxSessions reject parallel sessions, so we reuse
    // one authenticated handle for every channel. The handle is wrapped in
    // `Arc` so the background metrics task can share it with the main loop.
    let handle: Arc<Handle<ClientHandler>> = Arc::new(open_session(profile, app, tab_id).await?);

    // ── Shell channel ──────────────────────────────────────────────────────
    let mut shell_channel = handle
        .channel_open_session()
        .await
        .map_err(|e| e.to_string())?;
    shell_channel
        .request_pty(
            true,
            "xterm-256color",
            80,
            24,
            0,
            0,
            &[(russh::Pty::TTY_OP_ISPEED, 115200), (russh::Pty::TTY_OP_OSPEED, 115200)],
        )
        .await
        .map_err(|e| e.to_string())?;
    shell_channel
        .request_shell(true)
        .await
        .map_err(|e| e.to_string())?;

    // ── Probe platform ─────────────────────────────────────────────────────
    let platform = super::system_metrics::probe_remote_platform(&handle).await;
    eprintln!(
        "[SSH metrics] tab={} platform='{}' — probe completed",
        tab_id, platform
    );

    // ── SFTP subsystem ─────────────────────────────────────────────────────
    // russh-sftp 2.3 does not send `request_subsystem` itself — the caller
    // must request the "sftp" subsystem on the channel BEFORE converting it
    // to a stream. Without this, `SftpSession::new` blocks forever waiting
    // for the INIT reply and eventually times out.
    let sftp_channel = handle.channel_open_session().await.map_err(|e| e.to_string())?;
    sftp_channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("SFTP subsystem request failed: {}", e))?;
    let sftp = SftpSession::new(sftp_channel.into_stream())
        .await
        .map_err(|e| format!("SFTP init failed: {}", e))?;

    update_tab_status_and_emit(app, tab_id, "connected").await;

    // Emit "connected" notice so the user sees confirmation in the terminal.
    // Mirrors Electron's `appendSystemMessage('连接主机成功\r\n')`.
    emit_terminal_data(app, tab_id, "连接主机成功\r\n").await;

    // ── Initialize session snapshot ────────────────────────────────────────
    let state = app.state::<crate::services::workspace::WorkspaceState>();
    {
        let mut sessions = state.sessions.write().await;
        let existing_transcript = sessions
            .get(tab_id)
            .map(|s| s.terminal_transcript.clone())
            .unwrap_or_default();
        sessions.insert(
            tab_id.to_string(),
            crate::services::SessionSnapshot {
                profile_id: profile
                    .get("id")
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string(),
                access_host: format!("{}:{}", host, port),
                summary: format!("{}@{}", username, host),
                terminal_transcript: existing_transcript,
                remote_path: "/".to_string(),
                shell_cwd: Some("/".to_string()),
                follow_shell_cwd: true,
                remote_files: Vec::new(),
                file_access_mode: "user".to_string(),
                sudo_user: None,
                has_reusable_sudo_auth: false,
                connected: true,
                system_metrics: None,
            },
        );
    }

    // ── Initial file listing ───────────────────────────────────────────────
    match list_dir(&sftp, "/").await {
        Ok(files) => {
            let mut sessions = state.sessions.write().await;
            if let Some(s) = sessions.get_mut(tab_id) {
                s.remote_files = files;
            }
        }
        Err(e) => {
            // Surface SFTP listing errors so the user understands why the
            // file panel stays empty (e.g. permission denied on /).
            emit_terminal_data(
                app,
                tab_id,
                &format!("\r\n[files] 列出根目录失败: {}\r\n", e),
            )
            .await;
        }
    }

    // Push the full snapshot (with files) to the renderer
    if let Ok(snapshot) = crate::commands::get_workspace_snapshot(app.clone()).await {
        let _ = app.emit("workspace:snapshot", snapshot);
    }

    // ── Spawn metrics collection task (single persistent channel) ─────────
    // Instead of opening a new exec channel every second (which adds variable
    // SSH overhead and makes the refresh cadence jittery), we open one
    // long-lived shell channel and pipe an infinite-loop script into it.
    // The remote side controls the 1s cadence via `sleep 1`, so data arrives
    // at a rock-steady interval regardless of SSH RTT.
    let metrics_shutdown = Arc::new(tokio::sync::Notify::new());
    let metrics_shutdown_clone = metrics_shutdown.clone();
    {
        let metrics_handle = Arc::clone(&handle);
        let metrics_app = app.clone();
        let metrics_tid = tab_id.to_string();
        let metrics_plat = platform.clone();
        tokio::spawn(async move {
            eprintln!(
                "[SSH metrics] task spawned tab={} plat='{}' — starting streaming collector",
                metrics_tid, metrics_plat
            );

            // Build the infinite-loop script. Each iteration emits a
            // delimited metrics block and sleeps for 1 second. We use a
            // unique marker so the stream parser can reliably slice blocks.
            let marker = "__FILETERM_METRICS_BLOCK__";
            let script_body = if metrics_plat == "windows" {
                // Windows: wrap the metrics script in a loop with Start-Sleep
                let metrics = super::system_metrics::build_windows_metrics_command();
                format!(
                    r#"
while ($true) {{
{0}
  Write-Output '{1}'
  Start-Sleep -Seconds 1
}}
"#,
                    metrics, marker
                )
            } else {
                // POSIX: wrap the metrics script in a while-true loop
                let raw = if metrics_plat == "busybox" {
                    "busybox"
                } else {
                    "linux"
                };
                let metrics = super::system_metrics::build_posix_metrics_command(raw);
                format!(
                    "{}\nwhile true; do\n{}\necho '{}'\nsleep 1\ndone\n",
                    "cd / >/dev/null 2>&1 || true", metrics, marker
                )
            };

            // Open one persistent shell channel for the entire session.
            let mut channel = match metrics_handle.channel_open_session().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "[SSH metrics] tab={} failed to open channel: {}",
                        metrics_tid, e
                    );
                    return;
                }
            };

            // Use a shell so we can pipe the script via stdin and keep it running.
            if let Err(e) = channel.request_shell(true).await {
                eprintln!(
                    "[SSH metrics] tab={} failed to request shell: {}",
                    metrics_tid, e
                );
                return;
            }

            // Feed the script into the shell
            if let Err(e) = channel.data(script_body.as_bytes()).await {
                eprintln!(
                    "[SSH metrics] tab={} failed to write script: {}",
                    metrics_tid, e
                );
                return;
            }

            eprintln!(
                "[SSH metrics] tab={} streaming collector started — waiting for first block",
                metrics_tid
            );

            // Stream reader: accumulate data, split on the marker, parse
            // each complete block and emit it to the renderer.
            let mut buffer: Vec<u8> = Vec::new();
            let marker_bytes = marker.as_bytes();

            loop {
                tokio::select! {
                    biased;
                    _ = metrics_shutdown_clone.notified() => {
                        let _ = channel.close().await;
                        break;
                    }
                    msg = channel.wait() => {
                        match msg {
                            Some(ChannelMsg::Data { data }) => {
                                buffer.extend_from_slice(data.as_ref());
                                // Drain all complete blocks from the buffer.
                                while let Some(idx) = find_subsequence(&buffer, marker_bytes) {
                                    let block = String::from_utf8_lossy(&buffer[..idx]).into_owned();
                                    buffer.drain(..idx + marker_bytes.len());
                                    // Parse and emit this block
                                    let val = super::system_metrics::parse_system_metrics(
                                        &block,
                                        &metrics_plat,
                                    );
                                    let cpu_pct = val.get("cpuPercent").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                                    let mem_pct = val.get("memoryPercent").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                                    if cpu_pct < 0.0 && mem_pct < 0.0 {
                                        // Probably garbage / incomplete block
                                        continue;
                                    }
                                    {
                                        let state = metrics_app
                                            .state::<crate::services::workspace::WorkspaceState>();
                                        let mut sessions = state.sessions.write().await;
                                        if let Some(s) = sessions.get_mut(&metrics_tid) {
                                            s.system_metrics = Some(merge_system_metrics_history(
                                                s.system_metrics.as_ref(),
                                                val.clone(),
                                                600,
                                            ));
                                        }
                                    }
                                    let payload = serde_json::json!({
                                        "tabId": metrics_tid,
                                        "systemMetrics": val,
                                        "mode": "append",
                                    });
                                    let _ = metrics_app.emit("workspace:sessionMetrics", payload);
                                }
                                // Cap buffer to prevent unbounded growth
                                if buffer.len() > 1_000_000 {
                                    buffer.drain(..buffer.len() - 500_000);
                                }
                            }
                            Some(ChannelMsg::ExtendedData { data, .. }) => {
                                buffer.extend_from_slice(data.as_ref());
                            }
                            Some(ChannelMsg::ExitStatus { .. }) | None => {
                                eprintln!(
                                    "[SSH metrics] tab={} channel closed",
                                    metrics_tid
                                );
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            let _ = channel.close().await;
            eprintln!(
                "[SSH metrics] tab={} task ended",
                metrics_tid
            );
        });
    }

    // ── Main event loop: terminal reads + command dispatch ─────────────────
    let mut cwd_buffer = String::new();
    let mut batch_buffer: Vec<u8> = Vec::new();
    let mut last_emit = Instant::now();

    // sudo / root-mode credentials — kept in worker-local state so they
    // never leak into SessionSnapshot (which is serialized to the renderer).
    let mut file_access_mode = "user".to_string();
    let mut sudo_user: Option<String> = None;
    let mut sudo_password: Option<String> = None;

    let sftp_arc = Arc::new(RwLock::new(sftp));

    loop {
        // 16ms batch window for terminal output.
        let next_batch_deadline =
            tokio::time::Instant::from_std(last_emit + Duration::from_millis(16));

        tokio::select! {
            biased;
            // 1. Drain pending IPC commands first so user input never waits
            //    on the network. When the sender is dropped (reconnect /
            //    disconnect / close), `recv()` returns None and we must
            //    exit — otherwise the old worker keeps emitting
            //    `terminal:data` alongside the new worker, producing
            //    duplicated echo ("clear" → "clearclear") and double
            //    newlines.
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let result = handle_worker_cmd(
                            cmd,
                            &handle,
                            &shell_channel,
                            &sftp_arc,
                            &mut file_access_mode,
                            &mut sudo_user,
                            &mut sudo_password,
                            tab_id,
                            app,
                            &state,
                        ).await;
                        match result {
                            Ok(true) => {
                                // WorkerCmd::Disconnect requested — flush and exit.
                                flush_batch(&mut batch_buffer, app, tab_id).await;
                                metrics_shutdown.notify_waiters();
                                return Ok(());
                            }
                            Ok(false) => {}
                            Err(e) => {
                                eprintln!("[SSH Worker] cmd error for tab {}: {}", tab_id, e);
                            }
                        }
                    }
                    None => {
                        // Sender dropped — flush and exit cleanly.
                        flush_batch(&mut batch_buffer, app, tab_id).await;
                        metrics_shutdown.notify_waiters();
                        return Ok(());
                    }
                }
            }
            // 2. Drain shell channel output.
            msg = shell_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let bytes = data.as_ref();
                        let text = String::from_utf8_lossy(bytes);
                        let (new_cwd, new_user) = track_cwd_and_user(&text, &mut cwd_buffer);
                        if new_cwd.is_some() || new_user.is_some() {
                            let mut sessions = state.sessions.write().await;
                            if let Some(s) = sessions.get_mut(tab_id) {
                                if let Some(cwd) = new_cwd { s.shell_cwd = Some(cwd.clone()); s.remote_path = cwd; }
                                if let Some(user) = new_user { s.sudo_user = Some(user); }
                            }
                            drop(sessions);
                            if let Ok(snap) = crate::commands::get_workspace_snapshot(app.clone()).await {
                                let _ = app.emit("workspace:snapshot", snap);
                            }
                        }

                        batch_buffer.extend_from_slice(bytes);
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        batch_buffer.extend_from_slice(data.as_ref());
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        // Shell closed → flush and disconnect.
                        flush_batch(&mut batch_buffer, app, tab_id).await;
                        metrics_shutdown.notify_waiters();
                        return Ok(());
                    }
                    _ => {}
                }
            }
            // 3. Periodic flush if there is buffered output.
            _ = tokio::time::sleep_until(next_batch_deadline) => {
                if !batch_buffer.is_empty() {
                    flush_batch(&mut batch_buffer, app, tab_id).await;
                    last_emit = Instant::now();
                } else {
                    last_emit = Instant::now();
                }
            }
        }
    }
}

/// Returns `Ok(true)` when the worker should exit (Disconnect requested),
/// `Ok(false)` otherwise.
#[allow(clippy::too_many_arguments)]
async fn handle_worker_cmd(
    cmd: WorkerCmd,
    handle: &Handle<ClientHandler>,
    shell_channel: &Channel<russh::client::Msg>,
    sftp: &Arc<RwLock<SftpSession>>,
    file_access_mode: &mut String,
    sudo_user: &mut Option<String>,
    sudo_password: &mut Option<String>,
    tab_id: &str,
    app: &AppHandle,
    state: &tauri::State<'_, crate::services::workspace::WorkspaceState>,
) -> Result<bool, String> {
    match cmd {
        WorkerCmd::WriteTerminal(data) => {
            let bytes = data.as_bytes().to_vec();
            shell_channel
                .data(&bytes[..])
                .await
                .map_err(|e| e.to_string())?;
            Ok(false)
        }
        WorkerCmd::ResizeTerminal { cols, rows, .. } => {
            shell_channel
                .window_change(cols, rows, 0, 0)
                .await
                .map_err(|e| e.to_string())?;
            Ok(false)
        }
        WorkerCmd::ListRemoteFiles { path, respond_to } => {
            let res = if file_access_mode == "root" {
                exec_list_dir_via_shell(handle, &path, sudo_user, sudo_password).await
            } else {
                let sftp = sftp.read().await;
                list_dir(&sftp, &path).await
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::ReadRemoteFile { path, encoding: _, respond_to } => {
            let res = if file_access_mode == "root" {
                exec_read_file_via_shell(handle, &path, sudo_user, sudo_password).await
            } else {
                let sftp = sftp.read().await;
                read_file(&sftp, &path).await
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::WriteRemoteFile { path, content, encoding: _, respond_to } => {
            let res = if file_access_mode == "root" {
                exec_write_file_via_shell(handle, &path, &content, sudo_user, sudo_password).await
            } else {
                let sftp = sftp.read().await;
                write_file(&sftp, &path, &content).await
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::CreateRemoteDirectory { parent_path, name, respond_to } => {
            let full_path = format!("{}/{}", parent_path.trim_end_matches('/'), name);
            let res = if file_access_mode == "root" {
                exec_shell_file_command(
                    handle,
                    &format!("mkdir -p {}", shell_quote(&full_path)),
                    sudo_user,
                    sudo_password,
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
            } else {
                let sftp = sftp.read().await;
                create_dir(&sftp, &full_path).await
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::CreateRemoteFile { parent_path, name, respond_to } => {
            let full_path = format!("{}/{}", parent_path.trim_end_matches('/'), name);
            let res = if file_access_mode == "root" {
                exec_write_file_via_shell(handle, &full_path, "", sudo_user, sudo_password).await
            } else {
                let sftp = sftp.read().await;
                write_file(&sftp, &full_path, "").await
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::CopyRemotePath { target_path, destination_path, target_type, respond_to } => {
            let dest_dir = std::path::Path::new(&destination_path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".to_string());
            let cp_cmd = if target_type == "folder" { "cp -R" } else { "cp" };
            let cmd_str = format!(
                "mkdir -p {} && {} {} {}",
                shell_quote(&dest_dir),
                cp_cmd,
                shell_quote(&target_path),
                shell_quote(&destination_path)
            );
            let res = if file_access_mode == "root" {
                exec_shell_file_command(handle, &cmd_str, sudo_user, sudo_password)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                super::system_metrics::exec_command(handle, &cmd_str)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::MoveRemotePath { target_path, destination_path, respond_to } => {
            let res = if file_access_mode == "root" {
                exec_shell_file_command(
                    handle,
                    &format!("mv {} {}", shell_quote(&target_path), shell_quote(&destination_path)),
                    sudo_user,
                    sudo_password,
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
            } else {
                let sftp = sftp.read().await;
                sftp.rename(&target_path, &destination_path)
                    .await
                    .map_err(|e| e.to_string())
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::RenameRemotePath { target_path, new_name, respond_to } => {
            let parent = std::path::Path::new(&target_path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".to_string());
            let dest = format!("{}/{}", parent.trim_end_matches('/'), new_name);
            let res = if file_access_mode == "root" {
                exec_shell_file_command(
                    handle,
                    &format!("mv {} {}", shell_quote(&target_path), shell_quote(&dest)),
                    sudo_user,
                    sudo_password,
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
            } else {
                let sftp = sftp.read().await;
                sftp.rename(&target_path, &dest)
                    .await
                    .map_err(|e| e.to_string())
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::DeleteRemotePath { target_path, target_type, respond_to } => {
            let cmd_str = if target_type == "folder" {
                format!("rm -rf {}", shell_quote(&target_path))
            } else {
                format!("rm -f {}", shell_quote(&target_path))
            };
            let res = if file_access_mode == "root" {
                exec_shell_file_command(handle, &cmd_str, sudo_user, sudo_password)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                super::system_metrics::exec_command(handle, &cmd_str)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::ChangeRemotePermissions { target_path, permissions, respond_to } => {
            let cmd_str = format!("chmod {:o} {}", permissions, shell_quote(&target_path));
            let res = if file_access_mode == "root" {
                exec_shell_file_command(handle, &cmd_str, sudo_user, sudo_password)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                super::system_metrics::exec_command(handle, &cmd_str)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            let _ = respond_to.send(res);
            Ok(false)
        }
        WorkerCmd::SetRemoteFileAccessMode {
            mode,
            sudo_user: new_sudo_user,
            sudo_password: new_sudo_password,
            respond_to,
        } => {
            *file_access_mode = mode.clone();
            *sudo_user = new_sudo_user.clone();
            if let Some(pwd) = new_sudo_password {
                if !pwd.is_empty() {
                    *sudo_password = Some(pwd);
                }
                // empty password ⇒ keep existing (cache reuse)
            } else {
                // No password provided — fall back to `sudo -n`.
                *sudo_password = None;
            }
            let has_reusable = sudo_password.is_some();
            let su_user = sudo_user.clone();
            let mut sessions = state.sessions.write().await;
            if let Some(s) = sessions.get_mut(tab_id) {
                s.file_access_mode = mode;
                s.sudo_user = su_user;
                s.has_reusable_sudo_auth = has_reusable;
            }
            let _ = respond_to.send(Ok(()));
            Ok(false)
        }
        WorkerCmd::Disconnect => {
            // Signal the worker loop to exit.
            let _ = app.emit(
                "terminal:data",
                serde_json::json!({
                    "tabId": tab_id,
                    "chunk": "",
                }),
            );
            Ok(true)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SFTP helpers (russh-sftp 2.x)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn list_dir(sftp: &SftpSession, dir_path: &str) -> Result<Vec<Value>, String> {
    let entries = sftp
        .read_dir(dir_path)
        .await
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let full_path = entry.path();
        let stat = entry.metadata();
        let perm_bits = stat.permissions.unwrap_or(0);
        let is_dir = stat.is_dir();
        let is_link = stat.is_symlink();
        let file_type = if is_dir {
            "folder"
        } else if is_link {
            "symlink"
        } else {
            "file"
        };
        let size_str = if is_dir {
            "-".to_string()
        } else {
            format_bytes(stat.size.unwrap_or(0))
        };
        let modified = format_unix_ts(stat.mtime.unwrap_or(0) as i64);
        let permission = format_perm(perm_bits, is_dir, is_link);
        let uid = stat.uid.unwrap_or(0);
        let gid = stat.gid.unwrap_or(0);
        items.push(serde_json::json!({
            "name": name,
            "path": full_path,
            "type": file_type,
            "size": size_str,
            "modified": modified,
            "permission": permission,
            "ownerGroup": format!("{}/{}", uid, gid),
        }));
    }
    items.sort_by(|a, b| {
        let af = a["type"].as_str() == Some("folder");
        let bf = b["type"].as_str() == Some("folder");
        bf.cmp(&af).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(items)
}

async fn read_file(sftp: &SftpSession, path: &str) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let mut f = sftp
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut s = String::new();
    f.read_to_string(&mut s).await.map_err(|e| e.to_string())?;
    Ok(s)
}

async fn write_file(sftp: &SftpSession, path: &str, content: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut f = sftp
        .create(path)
        .await
        .map_err(|e| e.to_string())?;
    f.write_all(content.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    f.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn create_dir(sftp: &SftpSession, path: &str) -> Result<(), String> {
    sftp.create_dir(path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// sudo / root-mode helpers (exec channel + `sudo -S` / `sudo -n`)
// ─────────────────────────────────────────────────────────────────────────────

/// POSIX shell quoting: wrap in single quotes, escape embedded single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a shell command via the exec channel, with sudo when credentials are
/// present. Returns the combined stdout. Detects sudo auth failures and
/// returns an error so the caller can clear cached credentials.
async fn exec_shell_file_command(
    handle: &Handle<ClientHandler>,
    command: &str,
    sudo_user: &Option<String>,
    sudo_password: &Option<String>,
) -> Result<String, String> {
    let user = sudo_user.as_deref().unwrap_or("root");
    let full_cmd = if sudo_password.is_some() {
        format!(
            "sudo -S -p '' -u {} sh -lc {}",
            shell_quote(user),
            shell_quote(command)
        )
    } else {
        format!(
            "sudo -n -u {} sh -lc {}",
            shell_quote(user),
            shell_quote(command)
        )
    };

    let output = if let Some(pwd) = sudo_password {
        let stdin = format!("{}\n", pwd);
        super::system_metrics::exec_command_with_stdin(handle, &full_cmd, &stdin).await?
    } else {
        super::system_metrics::exec_command(handle, &full_cmd).await?
    };

    let lower = output.to_lowercase();
    if lower.contains("incorrect password")
        || lower.contains("authentication failure")
        || lower.contains("a password is required")
        || lower.contains("sudo: permission denied")
    {
        return Err(
            "sudo authentication failed — password incorrect or sudo not granted".to_string(),
        );
    }
    Ok(output)
}

/// List a directory via `find -printf` under sudo (GNU coreutils, BusyBox).
async fn exec_list_dir_via_shell(
    handle: &Handle<ClientHandler>,
    path: &str,
    sudo_user: &Option<String>,
    sudo_password: &Option<String>,
) -> Result<Vec<Value>, String> {
    let cmd = format!(
        "find {} -maxdepth 1 -mindepth 1 -printf '%y|%s|%T@|%u:%g|%m|%f\\n' 2>/dev/null",
        shell_quote(path)
    );
    let output = exec_shell_file_command(handle, &cmd, sudo_user, sudo_password).await?;
    let path_norm = path.trim_end_matches('/');

    let mut items = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\n');
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 6 {
            continue;
        }
        let type_char = parts[0].chars().next().unwrap_or('f');
        let is_dir = type_char == 'd';
        let is_link = type_char == 'l';
        let size_value = parts[1].parse::<u64>().unwrap_or(0);
        let size_str = if is_dir {
            "-".to_string()
        } else {
            format_bytes(size_value)
        };
        let mtime: i64 = parts[2].split('.').next().unwrap_or("0").parse().unwrap_or(0);
        let owner_group = parts[3].to_string();
        let perm_octal = u32::from_str_radix(parts[4], 8).unwrap_or(0o644);
        let name = parts[5].to_string();
        if name == "." || name == ".." {
            continue;
        }

        let file_type = if is_dir {
            "folder"
        } else if is_link {
            "symlink"
        } else {
            "file"
        };
        let permission = format_perm(perm_octal, is_dir, is_link);
        let full_path = if path_norm.is_empty() || path_norm == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", path_norm, name)
        };
        let modified = format_unix_ts(mtime);

        items.push(serde_json::json!({
            "name": name,
            "path": full_path,
            "type": file_type,
            "size": size_str,
            "modified": modified,
            "permission": permission,
            "ownerGroup": owner_group,
        }));
    }
    items.sort_by(|a, b| {
        let af = a["type"].as_str() == Some("folder");
        let bf = b["type"].as_str() == Some("folder");
        bf.cmp(&af).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(items)
}

/// Read a file via `sudo cat` + base64 (binary-safe over the exec channel).
async fn exec_read_file_via_shell(
    handle: &Handle<ClientHandler>,
    path: &str,
    sudo_user: &Option<String>,
    sudo_password: &Option<String>,
) -> Result<String, String> {
    let cmd = format!("base64 {}", shell_quote(path));
    let output = exec_shell_file_command(handle, &cmd, sudo_user, sudo_password).await?;
    let trimmed: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&trimmed)
        .map_err(|e| format!("base64 decode failed: {}", e))?;
    String::from_utf8(bytes).map_err(|e| format!("utf8 decode failed: {}", e))
}

/// Write a file via `sudo tee` + base64 (binary-safe).
async fn exec_write_file_via_shell(
    handle: &Handle<ClientHandler>,
    path: &str,
    content: &str,
    sudo_user: &Option<String>,
    sudo_password: &Option<String>,
) -> Result<(), String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    let cmd = format!("base64 -d | tee {} > /dev/null", shell_quote(path));
    let user = sudo_user.as_deref().unwrap_or("root");
    let full_cmd = if sudo_password.is_some() {
        format!(
            "sudo -S -p '' -u {} sh -lc {}",
            shell_quote(user),
            shell_quote(&cmd)
        )
    } else {
        format!(
            "sudo -n -u {} sh -lc {}",
            shell_quote(user),
            shell_quote(&cmd)
        )
    };
    let stdin = if let Some(pwd) = sudo_password {
        format!("{}\n{}\n", pwd, encoded)
    } else {
        format!("{}\n", encoded)
    };
    let output = super::system_metrics::exec_command_with_stdin(handle, &full_cmd, &stdin).await?;
    let lower = output.to_lowercase();
    if lower.contains("incorrect password") || lower.contains("authentication failure") {
        return Err("sudo authentication failed".to_string());
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatting helpers
// ─────────────────────────────────────────────────────────────────────────────

pub fn format_unix_ts(secs: i64) -> String {
    if secs == 0 {
        return String::from("1970-01-01T00:00:00Z");
    }
    let mut remaining = secs / 86400;
    let time_secs = secs % 86400;
    let (h, m, s) = (time_secs / 3600, (time_secs % 3600) / 60, time_secs % 60);
    let mut year = 1970i32;
    loop {
        let dy = if leap(year) { 366 } else { 365 };
        if remaining < dy {
            break;
        }
        remaining -= dy;
        year += 1;
    }
    let md: [i64; 12] = if leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u32;
    for &days in &md {
        if remaining < days {
            break;
        }
        remaining -= days;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        remaining + 1,
        h,
        m,
        s
    )
}

fn leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn format_bytes(size: u64) -> String {
    if size == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit_index = 0;
    while value >= 1000.0 && unit_index < units.len() - 1 {
        value /= 1000.0;
        unit_index += 1;
    }
    let digits = if value >= 10.0 || unit_index == 0 { 0 } else { 1 };
    format!("{:.*} {}", digits, value, units[unit_index])
}

fn format_perm(perm: u32, is_dir: bool, is_link: bool) -> String {
    let tc = if is_link {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };
    let bits = perm & 0o777;
    let mut s = String::with_capacity(10);
    s.push(tc);
    for shift in [6u32, 3, 0] {
        let oct = (bits >> shift) & 7;
        s.push(if oct & 4 != 0 { 'r' } else { '-' });
        s.push(if oct & 2 != 0 { 'w' } else { '-' });
        s.push(if oct & 1 != 0 { 'x' } else { '-' });
    }
    s
}
