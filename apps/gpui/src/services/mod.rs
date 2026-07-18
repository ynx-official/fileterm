//! Service-layer modules forked from `apps/tauri/src-tauri/src/services/` for
//! the GPUI runtime.
//!
//! G1 only forks the `logging` shim surface that `backend::sessions::local_files`
//! depends on; the rest of Tauri's `services/` tree (transfers, connections,
//! profile_ops, updates, etc.) lands phase-by-phase as the bridge methods that
//! need them are wired up.

pub mod logging;
