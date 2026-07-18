//! Phase G-1.2 spike: spawn a PTY, drive it, print the bytes it emits.
//!
//! Run with:
//!   cargo run -p fileterm-gpui --example term_pty
//!
//! Acceptance: stdout shows at least one chunk of bytes from the spawned
//! shell (its prompt), then we send `echo hello-g-1-2\nexit\n` and the
//! process exits 0 after the channel closes.
//!
//! This example does NOT open a gpui window — that keeps it CI-friendly
//! (no display / GPU needed). The window integration lands in G-1.4.

use anyhow::Result;
use tokio::time::{sleep, Duration};

use fileterm_gpui::term::PtyHandle;

#[tokio::main]
async fn main() -> Result<()> {
    // bash is the spike default; fall back to sh if bash is missing so the
    // example still runs on minimal containers.
    let shell = which_bash().unwrap_or_else(|| "sh".to_string());
    eprintln!("[term_pty] spawning shell: {shell}");

    let (pty, mut rx) = PtyHandle::spawn(&shell, 80, 24)?;
    eprintln!("[term_pty] spawned, waiting for first chunk...");

    // Wait up to 2s for the shell to emit its prompt / first bytes.
    let first = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    let first_chunk = match first {
        Ok(Ok(chunk)) => {
            eprintln!(
                "[term_pty] first chunk: seq={} {} bytes",
                chunk.seq,
                chunk.bytes.len()
            );
            print_chunk(&chunk.bytes);
            chunk
        }
        Ok(Err(e)) => anyhow::bail!("channel closed before first chunk: {e}"),
        Err(_) => anyhow::bail!("timeout waiting for first chunk from {shell}"),
    };

    // Drive the shell: echo a marker line, then exit. The marker lets the
    // test assert the round-trip (input -> shell echo -> output).
    pty.write_input(b"echo hello-g-1-2\n")?;
    pty.write_input(b"exit\n")?;

    // Drain remaining chunks until the channel closes (child exited).
    let mut got_marker = first_chunk.bytes.windows(11).any(|w| w == b"hello-g-1-2");
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            eprintln!("[term_pty] drain timeout, exiting");
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(chunk)) => {
                print_chunk(&chunk.bytes);
                if chunk.bytes.windows(11).any(|w| w == b"hello-g-1-2") {
                    got_marker = true;
                }
            }
            Ok(Err(_closed)) => {
                eprintln!("[term_pty] channel closed (shell exited)");
                break;
            }
            Err(_timeout) => {
                eprintln!("[term_pty] drain timeout, exiting");
                break;
            }
        }
    }

    // Give the child a moment to be reaped. Not strictly needed since
    // PtyHandle holds the Child and drops it on scope exit.
    sleep(Duration::from_millis(50)).await;

    if got_marker {
        eprintln!("[term_pty] ACCEPTANCE: marker 'hello-g-1-2' observed in output");
        Ok(())
    } else {
        anyhow::bail!("marker 'hello-g-1-2' never appeared in shell output")
    }
}

fn print_chunk(bytes: &[u8]) {
    // escape_debug so non-printable bytes (color escapes, CR/LF) are visible
    // without corrupting the terminal. One chunk per line.
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = write!(lock, "chunk: ");
    for c in std::str::from_utf8(bytes).unwrap_or("").escape_debug() {
        let _ = write!(lock, "{c}");
    }
    let _ = writeln!(lock);
}

fn which_bash() -> Option<String> {
    // Avoids pulling in the `which` crate; PATH lookup via std.
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("bash");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}
