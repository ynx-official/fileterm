//! PTY bridge: spawn a child shell, pump its output into a `broadcast` channel,
//! expose `write_input` / `resize` to the renderer side.
//!
//! Phase G-1.2 of `docs/plans/active/gpui-spike.md`.
//!
//! Deviations from the spike skeleton, all forced by the real `portable-pty`
//! 0.8.1 API surface:
//!   * The trait is `MasterPty`, not `Master`.
//!   * The reader API is `try_clone_reader()` (returns `Box<dyn Read + Send>`),
//!     not `take_reader()`. The master still owns its read endpoint; we just
//!     clone a handle. Multiple readers are allowed but we only take one.
//!   * `SlavePty` has no `close()` method. The canonical pattern (see
//!     `portable-pty`'s `examples/whoami.rs`) is to `drop(pair.slave)` after
//!     spawning the child, which signals EOF to the kernel side cleanly.
//!   * `spawn_command` returns `Box<dyn Child + Send + Sync>`; we keep it on
//!     the `PtyHandle` so the child is reaped when the handle drops.
//!   * `SlavePty` / `PtySystem` traits don't need explicit `use` imports —
//!     Rust resolves `pair.slave.spawn_command()` and `system.openpty()`
//!     through the concrete types returned by `native_pty_system()`.
//!
//! Reading happens on a dedicated std thread (not a tokio task): the reader
//! is a blocking `std::io::Read`, and gpui's foreground executor can't park
//! blocking reads without stalling the render loop. The thread pushes
//! `TermChunk`es into a `tokio::sync::broadcast` channel; receivers can run
//! on either the gpui main thread or a tokio runtime.

use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::broadcast;

/// A chunk of bytes emitted by the PTY's slave side (shell output).
///
/// Cloned per receiver when popped from the broadcast channel. The `seq`
/// counter lets a receiver detect a `Lagged` gap and decide whether to
/// repaint from scratch or skip ahead.
#[derive(Clone, Debug)]
pub struct TermChunk {
    /// Monotonic sequence number assigned by the writer thread.
    pub seq: u64,
    /// Raw bytes from the PTY. UTF-8 boundaries are not guaranteed here;
    /// the parser (G-1.3) is responsible for handling partial code points.
    pub bytes: Vec<u8>,
}

/// Handle to a running PTY session.
///
/// Owns the master, the spawned child, and a `broadcast::Sender` for output
/// chunks. There is exactly one owner per session; readers take a
/// `broadcast::Receiver` from [`PtyHandle::subscribe`].
pub struct PtyHandle {
    /// Master end of the pty. `MasterPty` is `Send` but not `Sync` in
    /// general, so we wrap in `Arc` and only call from one thread at a time
    /// (the gpui main thread for `write_input` / `resize`). If we later need
    /// cross-thread writes, swap this for `Arc<Mutex<dyn MasterPty + Send>>`.
    master: Arc<dyn MasterPty + Send>,
    /// The child process. Kept here so it gets reaped on drop rather than
    /// turning into a zombie.
    _child: Box<dyn Child + Send + Sync>,
    /// Output broadcast. Receivers come and go; the writer thread keeps
    /// sending even if there are zero receivers (broadcast drops the chunk).
    tx: broadcast::Sender<TermChunk>,
    /// Writer to the slave's stdin. `portable_pty`'s `take_writer()` is
    /// one-shot per master instance (bails with "cannot take writer more
    /// than once"), so we take it once in `spawn` and reuse it for every
    /// `write_input` call. Wrapped in `Mutex` because `Write::write_all`
    /// needs `&mut self` and callers may issue writes from any thread.
    writer: Mutex<Option<Box<dyn Write + Send>>>,
}

impl PtyHandle {
    /// Spawn `shell` (e.g. `bash`, `zsh`, `sh`) into a new PTY at the given
    /// size. Returns the handle plus a receiver for the first subscriber.
    ///
    /// On Windows the caller should pass `cmd.exe` or `powershell.exe`;
    /// `portable-pty` will pick ConPTY automatically when available.
    pub fn spawn(shell: &str, cols: u16, rows: u16) -> Result<(Self, broadcast::Receiver<TermChunk>)> {
        let system = native_pty_system();
        let pair = system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .with_context(|| format!("openpty failed for {shell}"))?;

        let mut cmd = CommandBuilder::new(shell);
        // xterm-256color matches what our vte parser (G-1.3) will assume;
        // truecolor advertises 24-bit SGR support so shells like fish/zsh
        // emit RGB escape sequences instead of indexed palette hacks.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn_command failed for {shell}"))?;

        // Drop the slave handle now that the child has it; this is the
        // pattern from `portable-pty/examples/whoami.rs`. Holding the slave
        // open here would prevent EOF propagation when the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("try_clone_reader failed")?;

        // Take the writer ONCE here: `MasterPty::take_writer` is one-shot
        // per master instance (see `portable-pty`'s `unix.rs:319` / `serial.rs:229`).
        // Reusing the same writer for every `write_input` call is the
        // intended pattern — `Write::write_all` is fine to call repeatedly
        // on the same handle.
        let writer = pair.master.take_writer().context("take_writer failed")?;

        let (tx, rx) = broadcast::channel(256);

        // Spawn the read pump. We use a plain std thread, not a tokio task:
        // the reader is a blocking `Read` and we don't want to occupy an
        // async worker thread on a syscall that never yields. The seq
        // counter is local to the thread; we don't need atomics because
        // only this thread ever touches it.
        let tx_clone = tx.clone();
        std::thread::Builder::new()
            .name("fileterm-pty-read".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 8192];
                let mut seq: u64 = 0;
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            seq = seq.wrapping_add(1);
                            let chunk = TermChunk {
                                seq,
                                bytes: buf[..n].to_vec(),
                            };
                            // `send` errors only when there are zero
                            // receivers; that's fine — we keep pumping so
                            // a late subscriber still gets fresh output.
                            let _ = tx_clone.send(chunk);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                // Drop sender last so subscribers see RecvError::Closed
                // rather than hanging on a dead receiver.
                drop(tx_clone);
            })
            .context("failed to spawn pty read thread")?;

        Ok((
            Self {
                master: Arc::from(pair.master),
                _child: child,
                tx,
                writer: Mutex::new(Some(writer)),
            },
            rx,
        ))
    }

    /// Get a new receiver for output chunks. Each receiver independently
    /// tracks its own position in the stream; a slow receiver going Lagged
    /// does not affect others.
    pub fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.tx.subscribe()
    }

    /// Write user input (keystrokes, paste) to the PTY master. The bytes
    /// are flushed before returning.
    ///
    /// Returns `Err` if the writer has already been consumed by a previous
    /// call (which shouldn't happen since `spawn` is the only taker) or if
    /// the underlying `write_all` / `flush` fails.
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut guard = self.writer.lock();
        let writer = guard
            .as_mut()
            .context("pty writer was already taken (shouldn't happen — spawn owns it)")?;
        writer
            .write_all(bytes)
            .with_context(|| format!("write_all {} bytes", bytes.len()))?;
        writer.flush().context("flush failed")?;
        Ok(())
    }

    /// Resize the PTY. The kernel forwards the new size to the child via
    /// SIGWINCH on Unix; on Windows ConPTY updates the console buffer.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("pty resize failed")
    }
}
