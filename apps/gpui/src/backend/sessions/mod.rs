//! Sessions module forked from `apps/tauri/src-tauri/src/sessions/` for the
//! GPUI runtime's G1 phase (see `docs/plans/active/gpui-refactor.md`).
//!
//! Each submodule is migrated line-for-line from the Tauri source, with only
//! the mechanical transformations documented in the G1 migration rules:
//! `tauri::AppHandle` → `crate::backend::app_handle::AppHandle`,
//! `crate::AppError` → `crate::error::AppError`, and `#[tauri::command]`
//! annotations dropped (the GPUI bridge calls these fns directly, no IPC
//! macro registration).

pub mod local_files;
