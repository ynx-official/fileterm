//! Minimal logging shim forked from `apps/tauri/src-tauri/src/services/logging.rs`
//! for the GPUI runtime's G1 phase.
//!
//! Only the `_global` entry points called by the forked
//! `backend::sessions::local_files` module are surfaced here. Full file-backed
//! logging (redaction, rotation, `LOG_DIRECTORY` `OnceLock`, `init(app)`, and
//! the `warn_global` / `write` / `session` / `ssh_debug` helpers) lands in a
//! later G-phase when the first session controller that needs richer logging
//! is forked.
//!
//! For G1 these are no-ops, which is semantically identical to Tauri's
//! behavior before `logging::init()` is called: `write_global` returns early
//! when `LOG_DIRECTORY` is unset, so none of the `*_global` helpers write
//! anything until the runtime is initialized.

pub fn debug_global(_scope: &str, _message: impl AsRef<str>) {}

pub fn info_global(_scope: &str, _message: impl AsRef<str>) {}

pub fn error_global(_scope: &str, _message: impl AsRef<str>) {}
