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

use serde::{Deserialize, Serialize};

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

/// Collector that runs commands on the remote host via SSH exec and
/// parses the output into `SystemMetrics` snapshots.
///
/// G3 stub — the actual exec + parse pipeline lands in G3.3. The struct
/// is here so view code can hold a handle to it today.
#[allow(dead_code)]
pub struct SystemSidebarCollector {
    /// Interval between snapshots, in seconds. Default 5s matches
    /// Tauri's `system-metrics` worker.
    interval_secs: u64,
    /// Last collected snapshot. `None` until the first collection
    /// completes.
    last_snapshot: Option<SystemMetrics>,
}

impl SystemSidebarCollector {
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval_secs,
            last_snapshot: None,
        }
    }

    /// Collect one snapshot. G3 stub returns empty metrics.
    ///
    /// G3.3 TODO: run `uname -s`, `uname -a`, `free -b`, `cat /proc/loadavg`,
    /// `ps aux --sort=-%cpu | head -10` via SSH exec, parse each into
    /// `SystemMetrics` fields.
    pub async fn collect(&mut self) -> SystemMetrics {
        // G3.3 stub.
        SystemMetrics::default()
    }
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
}
