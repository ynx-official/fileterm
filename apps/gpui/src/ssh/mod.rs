//! SSH session controller + terminal main link.
//!
//! G3 phase of `docs/plans/active/gpui-refactor.md` section 6.4.
//!
//! ## What G3 delivers
//!
//! * [`controller::SshController`] — wraps a `russh` client session,
//!   owns the shell channel, and bridges SSH output → `TermSession`.
//!   Mirrors `apps/tauri/src-tauri/src/sessions/ssh.rs` but replaces
//!   the Tauri event emitter with a `broadcast::Sender<TermChunk>`.
//! * [`system_sidebar`] — CPU / memory / network / process table
//!   collector. Parses `top` / `free` / `ps` output on the remote host.
//! * [`terminal_dock`] — command input bar + history replay + path
//!   completion. Sits below the terminal in the workspace layout.
//!
//! ## What G3 does NOT deliver
//!
//! * Real SSH key agent forwarding — G4+ when SFTP needs it.
//! * Connection form UI — that's a modal window (G2 window kind), the
//!   form itself lands with the connection-manager window in G3.5.
//! * IME + cursor blink — G-1.7 deferred to G3, tracked separately.

pub mod controller;
pub mod system_sidebar;
pub mod terminal_dock;

pub use controller::SshController;
