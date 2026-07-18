//! System sidebar — remote host CPU / memory / network / process table.
//!
//! G3 phase of `docs/plans/active/gpui-refactor.md` section 6.4.
//!
//! Periodically runs `top` / `free` / `ps` / `netstat` on the remote host
//! via the SSH channel's exec subsystem, parses the output, and exposes
//! a typed `SystemMetrics` snapshot. `TermView` renders this in the
//! sidebar next to the terminal.
//!
//! ## Collection strategy
//!
//! Per `AGENTS.md` hard boundary: "系统指标解析入口必须对远端输出做
//! `replace(/\r\n?/g, '\n')` 归一化" — so all parsers normalize CRLF
//! before splitting on `\n`.
//!
//! Platform detection follows the same `supportsPosixShellSetup()` rule:
//! `linux` / `busybox` get POSIX shell commands; `windows` gets
//! PowerShell `Get-Counter` / `Get-Process` via multi-level fallback.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::controller::SshController;

/// One snapshot of remote system state.
///
/// All fields are `Option<...>` because any individual metric may fail
/// to collect (different OS, missing command, parse error) without
/// invalidating the whole snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub platform: Option<String>,
    pub cpu_usage: Option<f32>,
    pub cpu_count: Option<u32>,
    pub memory_total: Option<u64>,
    pub memory_used: Option<u64>,
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub load_avg_1: Option<f32>,
    pub load_avg_5: Option<f32>,
    pub load_avg_15: Option<f32>,
    pub network_rx_bytes: Option<u64>,
    pub network_tx_bytes: Option<u64>,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
}

impl SystemMetrics {
    /// Normalize CRLF → LF per the workspace hard boundary.
    ///
    /// Without this, `'windows\r'` from the platform string would poison
    /// the platform detection (the `\r` makes `starts_with("linux")`
    /// fail). Applied to raw output before any parsing.
    pub fn normalize_crlf(raw: &str) -> String {
        raw.replace("\r\n", "\n").replace('\r', "\n")
    }

    /// Detect platform from `uname -s` output.
    ///
    /// Returns `"linux"`, `"busybox"`, `"windows"`, or `"unknown"`.
    /// `busybox` is detected by checking for `busybox` in the output of
    /// `uname -a` (some embedded systems report `Linux` but use BusyBox
    /// utils with different flag syntax).
    pub fn detect_platform(uname_s: &str, uname_a: &str) -> &'static str {
        let s = Self::normalize_crlf(uname_s).trim().to_lowercase();
        let a = Self::normalize_crlf(uname_a).trim().to_lowercase();
        if s.contains("windows") || s.contains("mingw") || s.contains("cygwin") {
            "windows"
        } else if s.contains("linux") {
            if a.contains("busybox") {
                "busybox"
            } else {
                "linux"
            }
        } else {
            "unknown"
        }
    }

    /// Parse `free -b` output (Linux / BusyBox) into memory metrics.
    ///
    /// Expects normalized (CRLF → LF) output. Returns `None` for fields
    /// that can't be parsed rather than failing the whole snapshot.
    pub fn parse_free(free_output: &str) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        let normalized = Self::normalize_crlf(free_output);
        let mut mem_total = None;
        let mut mem_used = None;
        let mut swap_total = None;
        let mut swap_used = None;

        for line in normalized.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }
            match parts[0] {
                "Mem:" => {
                    mem_total = parts.get(1).and_then(|s| s.parse().ok());
                    mem_used = parts.get(2).and_then(|s| s.parse().ok());
                }
                "Swap:" => {
                    swap_total = parts.get(1).and_then(|s| s.parse().ok());
                    swap_used = parts.get(2).and_then(|s| s.parse().ok());
                }
                _ => {}
            }
        }

        (mem_total, mem_used, swap_total, swap_used)
    }

    /// Parse `/proc/loadavg` (Linux) or `uptime` output (BusyBox) into
    /// the three load-average numbers.
    pub fn parse_loadavg(loadavg_output: &str) -> (Option<f32>, Option<f32>, Option<f32>) {
        let normalized = Self::normalize_crlf(loadavg_output);
        let parts: Vec<&str> = normalized.split_whitespace().collect();
        (
            parts.first().and_then(|s| s.parse().ok()),
            parts.get(1).and_then(|s| s.parse().ok()),
            parts.get(2).and_then(|s| s.parse().ok()),
        )
    }
}

pub struct SystemSidebarCollector {
    controller: Arc<SshController>,
    interval_secs: u64,
    last_snapshot: Option<SystemMetrics>,
    previous_cpu: Option<(u64, u64)>,
}

impl SystemSidebarCollector {
    pub fn new(controller: Arc<SshController>, interval_secs: u64) -> Self {
        Self {
            controller,
            interval_secs: interval_secs.max(1),
            last_snapshot: None,
            previous_cpu: None,
        }
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    pub fn last_snapshot(&self) -> Option<&SystemMetrics> {
        self.last_snapshot.as_ref()
    }

    pub async fn collect(&mut self) -> SystemMetrics {
        const COMMAND: &str = "printf '__FT_UNAME_S__\\n'; uname -s 2>/dev/null; printf '__FT_UNAME_A__\\n'; uname -a 2>/dev/null; printf '__FT_CPU_COUNT__\\n'; (getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null); printf '__FT_FREE__\\n'; free -b 2>/dev/null; printf '__FT_LOAD__\\n'; cat /proc/loadavg 2>/dev/null; printf '__FT_CPU__\\n'; head -n 1 /proc/stat 2>/dev/null; printf '__FT_NET__\\n'; cat /proc/net/dev 2>/dev/null; printf '__FT_PS__\\n'; ps -eo pid=,comm=,%cpu=,rss= --sort=-%cpu 2>/dev/null | head -n 10";
        let Ok(bytes) = self.controller.exec(COMMAND).await else {
            return self.last_snapshot.clone().unwrap_or_default();
        };
        let raw = SystemMetrics::normalize_crlf(&String::from_utf8_lossy(&bytes));
        let sections = split_sections(&raw);
        let uname_s = sections.get("UNAME_S").copied().unwrap_or_default();
        let uname_a = sections.get("UNAME_A").copied().unwrap_or_default();
        let platform = SystemMetrics::detect_platform(uname_s, uname_a).to_string();
        let (memory_total, memory_used, swap_total, swap_used) =
            SystemMetrics::parse_free(sections.get("FREE").copied().unwrap_or_default());
        let (load_avg_1, load_avg_5, load_avg_15) =
            SystemMetrics::parse_loadavg(sections.get("LOAD").copied().unwrap_or_default());
        let cpu_sample = parse_cpu_sample(sections.get("CPU").copied().unwrap_or_default());
        let cpu_usage = cpu_sample.and_then(|current| {
            let usage = self
                .previous_cpu
                .and_then(|previous| cpu_usage(previous, current));
            self.previous_cpu = Some(current);
            usage
        });
        let (network_rx_bytes, network_tx_bytes) =
            parse_network(sections.get("NET").copied().unwrap_or_default());
        let snapshot = SystemMetrics {
            platform: Some(platform),
            cpu_usage,
            cpu_count: sections
                .get("CPU_COUNT")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok()),
            memory_total,
            memory_used,
            swap_total,
            swap_used,
            load_avg_1,
            load_avg_5,
            load_avg_15,
            network_rx_bytes,
            network_tx_bytes,
            top_processes: parse_processes(sections.get("PS").copied().unwrap_or_default()),
        };
        self.last_snapshot = Some(snapshot.clone());
        snapshot
    }
}

fn split_sections(raw: &str) -> std::collections::HashMap<&str, &str> {
    let mut sections = std::collections::HashMap::new();
    let mut current = None;
    let mut start = 0;
    for (offset, _) in raw.match_indices('\n') {
        let line = &raw[start..offset];
        if let Some(name) = line
            .strip_prefix("__FT_")
            .and_then(|value| value.strip_suffix("__"))
        {
            if let Some((previous, content_start)) = current.replace((name, offset + 1)) {
                sections.insert(previous, raw[content_start..start].trim());
            }
        }
        start = offset + 1;
    }
    if let Some((name, content_start)) = current {
        sections.insert(name, raw[content_start..].trim());
    }
    sections
}

fn parse_cpu_sample(raw: &str) -> Option<(u64, u64)> {
    let values = raw
        .split_whitespace()
        .skip_while(|value| *value == "cpu")
        .filter_map(|value| value.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((values.iter().sum(), idle))
}

fn cpu_usage(previous: (u64, u64), current: (u64, u64)) -> Option<f32> {
    let total = current.0.checked_sub(previous.0)?;
    let idle = current.1.checked_sub(previous.1)?;
    (total > 0).then(|| ((total - idle) as f32 / total as f32) * 100.0)
}

fn parse_network(raw: &str) -> (Option<u64>, Option<u64>) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    let mut found = false;
    for line in raw.lines().filter(|line| line.contains(':')) {
        let Some((_, values)) = line.split_once(':') else {
            continue;
        };
        let values = values.split_whitespace().collect::<Vec<_>>();
        if values.len() < 9 {
            continue;
        }
        if let (Ok(received), Ok(sent)) = (values[0].parse::<u64>(), values[8].parse::<u64>()) {
            rx = rx.saturating_add(received);
            tx = tx.saturating_add(sent);
            found = true;
        }
    }
    if found {
        (Some(rx), Some(tx))
    } else {
        (None, None)
    }
}

fn parse_processes(raw: &str) -> Vec<ProcessInfo> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some(ProcessInfo {
                pid: fields.next()?.parse().ok()?,
                name: fields.next()?.to_string(),
                cpu: fields.next()?.parse().ok()?,
                memory: fields.next()?.parse::<u64>().ok()?.saturating_mul(1024),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_crlf_strips_carriage_returns() {
        assert_eq!(SystemMetrics::normalize_crlf("windows\r\n"), "windows\n");
        assert_eq!(SystemMetrics::normalize_crlf("linux\r"), "linux\n");
        assert_eq!(SystemMetrics::normalize_crlf("unix\n"), "unix\n");
    }

    #[test]
    fn detect_platform_linux() {
        assert_eq!(
            SystemMetrics::detect_platform("Linux\n", "Linux 5.15.0 #1 SMP"),
            "linux"
        );
    }

    #[test]
    fn detect_platform_busybox() {
        assert_eq!(
            SystemMetrics::detect_platform("Linux", "Linux 4.9.0 busybox"),
            "busybox"
        );
    }

    #[test]
    fn detect_platform_windows() {
        assert_eq!(SystemMetrics::detect_platform("Windows_NT", ""), "windows");
    }

    #[test]
    fn detect_platform_unknown() {
        assert_eq!(SystemMetrics::detect_platform("Darwin", ""), "unknown");
    }

    #[test]
    fn parse_free_extracts_memory_and_swap() {
        let output = "              total        used        free      shared\n\
                      Mem:       16384000     8192000     8192000      102400\n\
                      Swap:       2097152           0     2097152\n";
        let (mem_total, mem_used, swap_total, swap_used) = SystemMetrics::parse_free(output);
        assert_eq!(mem_total, Some(16384000));
        assert_eq!(mem_used, Some(8192000));
        assert_eq!(swap_total, Some(2097152));
        assert_eq!(swap_used, Some(0));
    }

    #[test]
    fn parse_loadavg_extracts_three_numbers() {
        let output = "0.52 0.41 0.35 1/234 5678\n";
        let (a, b, c) = SystemMetrics::parse_loadavg(output);
        assert_eq!(a, Some(0.52));
        assert_eq!(b, Some(0.41));
        assert_eq!(c, Some(0.35));
    }

    #[test]
    fn parse_loadavg_handles_crlf() {
        let output = "0.52 0.41 0.35 1/234 5678\r\n";
        let (a, _b, _c) = SystemMetrics::parse_loadavg(output);
        assert_eq!(a, Some(0.52));
    }

    #[test]
    fn tagged_sections_and_linux_samples_are_parsed() {
        let raw = "__FT_CPU__\ncpu  100 0 50 850 0\n__FT_NET__\neth0: 10 0 0 0 0 0 0 0 20 0\n__FT_PS__\n7 sshd 2.5 100\n";
        let sections = split_sections(raw);
        assert_eq!(parse_cpu_sample(sections["CPU"]), Some((1000, 850)));
        assert_eq!(parse_network(sections["NET"]), (Some(10), Some(20)));
        let processes = parse_processes(sections["PS"]);
        assert_eq!(processes[0].pid, 7);
        assert_eq!(processes[0].memory, 102_400);
    }

    #[test]
    fn cpu_usage_uses_sample_delta() {
        assert_eq!(cpu_usage((100, 80), (200, 140)), Some(40.0));
    }
}
