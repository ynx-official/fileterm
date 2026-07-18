//! Terminal subsystem: PTY bridge, model, view.
//!
//! Phase G-1.2 wires up `pty`; later phases add model/view/perform/osc.

pub mod pty;

pub use pty::{PtyHandle, TermChunk};
