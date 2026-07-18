use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast;

use crate::ssh::SshController;

use super::{PtyHandle, TermChunk};

/// Terminal byte-stream boundary shared by local PTY and remote SSH shells.
///
/// Views own this interface rather than a protocol-specific handle, keeping
/// keyboard input, resize, feed throttling, and grid rendering identical for
/// every terminal transport.
pub trait TerminalTransport: 'static {
    fn subscribe(&self) -> broadcast::Receiver<TermChunk>;
    fn write_input(&self, bytes: &[u8]) -> Result<()>;
    fn resize(&self, cols: u16, rows: u16) -> Result<()>;
}

impl TerminalTransport for PtyHandle {
    fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.subscribe()
    }

    fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.write_input(bytes)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.resize(cols, rows)
    }
}

impl TerminalTransport for SshController {
    fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.subscribe()
    }

    fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.write_input(bytes)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.resize(cols, rows)
    }
}

pub fn local_transport(handle: Arc<PtyHandle>) -> Arc<dyn TerminalTransport> {
    handle
}

pub fn ssh_transport(handle: Arc<SshController>) -> Arc<dyn TerminalTransport> {
    handle
}
