use std::{collections::HashMap, sync::Arc};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Mutex, RwLock},
};

use super::controller::ClientHandler;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelRule {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub kind: String,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelSnapshot {
    #[serde(flatten)]
    pub rule: SshTunnelRule,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteForwardTarget {
    pub bind_host: String,
    pub bind_port: u32,
    pub target_host: String,
    pub target_port: u16,
}

pub(crate) type RemoteForwardTargets = Arc<RwLock<Vec<RemoteForwardTarget>>>;
pub(crate) type SharedSshHandle = Arc<Mutex<russh::client::Handle<ClientHandler>>>;
pub type SharedTunnelManager = Arc<Mutex<TunnelManager>>;

pub struct TunnelManager {
    handle: SharedSshHandle,
    remote_targets: RemoteForwardTargets,
    tunnels: HashMap<String, SshTunnelSnapshot>,
    local_stops: HashMap<String, oneshot::Sender<()>>,
    remote_rules: HashMap<String, (String, u32)>,
}

impl TunnelManager {
    pub(crate) fn shared(
        handle: SharedSshHandle,
        remote_targets: RemoteForwardTargets,
    ) -> SharedTunnelManager {
        Arc::new(Mutex::new(Self {
            handle,
            remote_targets,
            tunnels: HashMap::new(),
            local_stops: HashMap::new(),
            remote_rules: HashMap::new(),
        }))
    }

    pub fn list(&self) -> Vec<SshTunnelSnapshot> {
        let mut tunnels = self.tunnels.values().cloned().collect::<Vec<_>>();
        tunnels.sort_by(|left, right| left.rule.name.cmp(&right.rule.name));
        tunnels
    }

    pub fn register(&mut self, rule: SshTunnelRule) -> Result<()> {
        validate_tunnel_rule(&rule)?;
        if self
            .tunnels
            .get(&rule.id)
            .is_some_and(|existing| matches!(existing.status.as_str(), "running" | "starting"))
        {
            bail!("隧道 {} 正在运行", rule.id);
        }
        let conflict = self.tunnels.values().any(|existing| {
            existing.rule.id != rule.id
                && (existing.rule.kind == "remote") == (rule.kind == "remote")
                && existing.rule.bind_host == rule.bind_host
                && existing.rule.bind_port == rule.bind_port
        });
        if conflict {
            bail!(
                "监听地址 {}:{} 已被其他隧道使用",
                rule.bind_host,
                rule.bind_port
            );
        }
        self.tunnels.insert(
            rule.id.clone(),
            SshTunnelSnapshot {
                rule,
                status: "stopped".to_string(),
                error: None,
            },
        );
        Ok(())
    }

    pub async fn create(&mut self, rule: SshTunnelRule) -> Result<Vec<SshTunnelSnapshot>> {
        self.register(rule.clone())?;
        if let Err(error) = self.start(&rule.id).await {
            self.tunnels.remove(&rule.id);
            return Err(error);
        }
        Ok(self.list())
    }

    pub async fn start(&mut self, rule_id: &str) -> Result<Vec<SshTunnelSnapshot>> {
        if self.local_stops.contains_key(rule_id) || self.remote_rules.contains_key(rule_id) {
            return Ok(self.list());
        }
        let rule = self
            .tunnels
            .get(rule_id)
            .map(|snapshot| snapshot.rule.clone())
            .with_context(|| format!("未找到隧道 {rule_id}"))?;
        validate_tunnel_rule(&rule)?;
        self.set_status(rule_id, "starting", None);
        let result = if rule.kind == "remote" {
            self.start_remote(&rule).await
        } else {
            self.start_local_or_dynamic(&rule).await
        };
        match result {
            Ok(()) => self.set_status(rule_id, "running", None),
            Err(error) => {
                self.set_status(rule_id, "error", Some(error.to_string()));
                return Err(error);
            }
        }
        Ok(self.list())
    }

    async fn start_local_or_dynamic(&mut self, rule: &SshTunnelRule) -> Result<()> {
        let address = tunnel_bind_address(&rule.bind_host, rule.bind_port)?;
        let listener = TcpListener::bind(&address)
            .await
            .with_context(|| format!("无法监听 {address}"))?;
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let handle = self.handle.clone();
        let rule = rule.clone();
        let rule_id = rule.id.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    accepted = listener.accept() => match accepted {
                        Ok((socket, _)) => {
                            let handle = handle.clone();
                            let rule = rule.clone();
                            tokio::spawn(async move {
                                let _ = if rule.kind == "dynamic" {
                                    forward_socks5_connection(socket, handle).await
                                } else {
                                    forward_local_connection(socket, handle, &rule).await
                                };
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        self.local_stops.insert(rule_id, stop_tx);
        Ok(())
    }

    async fn start_remote(&mut self, rule: &SshTunnelRule) -> Result<()> {
        let accepted = self
            .handle
            .lock()
            .await
            .tcpip_forward(rule.bind_host.clone(), u32::from(rule.bind_port))
            .await
            .context("远端端口转发请求失败")?;
        if !accepted {
            bail!("SSH 服务器拒绝远端端口转发请求");
        }
        let actual_port = u32::from(rule.bind_port);
        self.remote_targets.write().await.push(RemoteForwardTarget {
            bind_host: rule.bind_host.clone(),
            bind_port: actual_port,
            target_host: rule.target_host.clone().unwrap_or_default(),
            target_port: rule.target_port.unwrap_or_default(),
        });
        self.remote_rules
            .insert(rule.id.clone(), (rule.bind_host.clone(), actual_port));
        Ok(())
    }

    pub async fn stop(&mut self, rule_id: &str) -> Result<Vec<SshTunnelSnapshot>> {
        if !self.tunnels.contains_key(rule_id) {
            bail!("未找到隧道 {rule_id}");
        }
        self.set_status(rule_id, "stopping", None);
        if let Some(stop) = self.local_stops.remove(rule_id) {
            let _ = stop.send(());
        }
        if let Some((bind_host, bind_port)) = self.remote_rules.remove(rule_id) {
            let cancel_result = {
                let handle = self.handle.lock().await;
                handle
                    .cancel_tcpip_forward(bind_host.clone(), bind_port)
                    .await
            };
            if let Err(error) = cancel_result {
                let message = format!("停止远端隧道失败: {error}");
                self.set_status(rule_id, "error", Some(message.clone()));
                return Err(anyhow::anyhow!(message));
            }
            self.remote_targets
                .write()
                .await
                .retain(|target| !(target.bind_host == bind_host && target.bind_port == bind_port));
        }
        self.set_status(rule_id, "stopped", None);
        Ok(self.list())
    }

    pub async fn delete(&mut self, rule_id: &str) -> Result<Vec<SshTunnelSnapshot>> {
        self.stop(rule_id).await?;
        self.tunnels.remove(rule_id);
        Ok(self.list())
    }

    pub async fn stop_all(&mut self) {
        let ids = self.tunnels.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let _ = self.stop(&id).await;
        }
    }

    fn set_status(&mut self, rule_id: &str, status: &str, error: Option<String>) {
        if let Some(snapshot) = self.tunnels.get_mut(rule_id) {
            snapshot.status = status.to_string();
            snapshot.error = error;
        }
    }
}

pub(crate) fn remote_bind_host_matches(bind_host: &str, connected_address: &str) -> bool {
    bind_host == connected_address || matches!(bind_host, "0.0.0.0" | "::" | "*")
}

fn validate_tunnel_rule(rule: &SshTunnelRule) -> Result<()> {
    if rule.id.trim().is_empty() || !matches!(rule.kind.as_str(), "local" | "remote" | "dynamic") {
        bail!("隧道需要有效的 ID 和类型");
    }
    if rule.bind_host.trim().is_empty() || rule.bind_port == 0 {
        bail!("隧道需要有效的监听地址和端口");
    }
    if rule.kind != "dynamic"
        && (rule.target_host.as_deref().unwrap_or("").trim().is_empty()
            || rule.target_port.unwrap_or_default() == 0)
    {
        bail!("{} 隧道需要有效的目标地址和端口", rule.kind);
    }
    Ok(())
}

fn tunnel_bind_address(host: &str, port: u16) -> Result<String> {
    let host = match host.trim() {
        "*" => "0.0.0.0",
        "" => bail!("监听地址不能为空"),
        value => value,
    };
    Ok(if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    })
}

async fn forward_local_connection(
    mut socket: TcpStream,
    handle: SharedSshHandle,
    rule: &SshTunnelRule,
) -> Result<()> {
    let origin = socket.peer_addr().ok();
    let origin_host = origin
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let origin_port = origin.map(|address| address.port()).unwrap_or_default();
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_direct_tcpip(
                rule.target_host.clone().unwrap_or_default(),
                u32::from(rule.target_port.unwrap_or_default()),
                origin_host,
                u32::from(origin_port),
            )
            .await
            .context("SSH 本地转发失败")?
            .into_stream()
    };
    copy_bidirectional(&mut socket, &mut channel).await?;
    Ok(())
}

async fn forward_socks5_connection(mut socket: TcpStream, handle: SharedSshHandle) -> Result<()> {
    let mut greeting = [0_u8; 2];
    socket.read_exact(&mut greeting).await?;
    if greeting[0] != 5 {
        bail!("仅支持 SOCKS5");
    }
    let mut methods = vec![0_u8; greeting[1] as usize];
    socket.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        socket.write_all(&[5, 0xff]).await?;
        bail!("SOCKS5 客户端不支持无认证模式");
    }
    socket.write_all(&[5, 0]).await?;

    let mut request = [0_u8; 4];
    socket.read_exact(&mut request).await?;
    if request[0] != 5 || request[1] != 1 {
        bail!("仅支持 SOCKS5 CONNECT");
    }
    let target_host = read_socks5_host(&mut socket, request[3]).await?;
    let mut port = [0_u8; 2];
    socket.read_exact(&mut port).await?;
    let target_port = u16::from_be_bytes(port);
    let origin = socket.peer_addr().ok();
    let origin_host = origin
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let origin_port = origin.map(|address| address.port()).unwrap_or_default();
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_direct_tcpip(
                target_host,
                u32::from(target_port),
                origin_host,
                u32::from(origin_port),
            )
            .await
            .context("SSH SOCKS5 转发失败")?
            .into_stream()
    };
    socket.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    copy_bidirectional(&mut socket, &mut channel).await?;
    Ok(())
}

async fn read_socks5_host(socket: &mut TcpStream, address_type: u8) -> Result<String> {
    match address_type {
        1 => {
            let mut address = [0_u8; 4];
            socket.read_exact(&mut address).await?;
            Ok(std::net::Ipv4Addr::from(address).to_string())
        }
        3 => {
            let mut length = [0_u8; 1];
            socket.read_exact(&mut length).await?;
            let mut name = vec![0_u8; length[0] as usize];
            socket.read_exact(&mut name).await?;
            String::from_utf8(name).context("SOCKS5 主机名不是有效 UTF-8")
        }
        4 => {
            let mut address = [0_u8; 16];
            socket.read_exact(&mut address).await?;
            Ok(std::net::Ipv6Addr::from(address).to_string())
        }
        _ => bail!("不支持的 SOCKS5 地址类型"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_rule() -> SshTunnelRule {
        SshTunnelRule {
            id: "local-1".into(),
            name: "database".into(),
            kind: "local".into(),
            bind_host: "127.0.0.1".into(),
            bind_port: 15432,
            target_host: Some("db.internal".into()),
            target_port: Some(5432),
            auto_start: false,
        }
    }

    #[test]
    fn validates_tunnel_kinds_and_targets() {
        assert!(validate_tunnel_rule(&local_rule()).is_ok());
        let mut invalid = local_rule();
        invalid.target_port = None;
        assert!(validate_tunnel_rule(&invalid).is_err());
        let mut dynamic = local_rule();
        dynamic.kind = "dynamic".into();
        dynamic.target_host = None;
        dynamic.target_port = None;
        assert!(validate_tunnel_rule(&dynamic).is_ok());
    }

    #[test]
    fn normalizes_wildcard_and_ipv6_bind_addresses() {
        assert_eq!(tunnel_bind_address("*", 1080).unwrap(), "0.0.0.0:1080");
        assert_eq!(tunnel_bind_address("::1", 1080).unwrap(), "[::1]:1080");
    }

    #[test]
    fn remote_wildcard_matches_server_connected_address() {
        assert!(remote_bind_host_matches("0.0.0.0", "127.0.0.1"));
        assert!(remote_bind_host_matches("*", "localhost"));
        assert!(!remote_bind_host_matches("127.0.0.1", "localhost"));
    }
}
