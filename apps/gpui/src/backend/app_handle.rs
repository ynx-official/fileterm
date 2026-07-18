//! GPUI runtime's replacement for `tauri::AppHandle`.
//!
//! G1 phase of `docs/plans/active/gpui-refactor.md` section 6.2.
//!
//! ## Why this exists
//!
//! Tauri's `AppHandle` is a framework handle that gives commands access to
//! the app data dir, window manager, event emitter, state registry, etc.
//! In the GPUI runtime there's no framework handle — the app is a single
//! process with direct references. But the forked `storage/` and
//! `commands/` code expects *something* to resolve `app_data_dir()` from,
//! so we introduce a minimal `AppHandle` that holds just the data dir.
//!
//! As G2–G5 land, this struct grows: G2 adds `WindowRegistry`, G3 adds
//! session state, G4 adds `TransferService`, etc. For G1 it's just a
//! typed `PathBuf` wrapper.
//!
//! ## Why not just pass `PathBuf` everywhere?
//!
//! Because the forked storage/commands code has ~50 call sites that take
//! `&AppHandle`. Changing all of them to `&Path` is a diff bomb that
//! risks introducing bugs during the fork. Keeping the `AppHandle` shape
//! means the fork is line-for-line with Tauri, and we can cherry-pick
//! upstream storage fixes without re-translating the parameter types.

use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};

/// Framework-agnostic replacement for `tauri::AppHandle`.
///
/// Held as `Arc<AppHandle>` inside `GpuiDesktopApi` (once G1 wires it in)
/// and passed by `&AppHandle` to storage/commands fns, matching Tauri's
/// `&AppHandle` convention.
#[derive(Clone)]
pub struct AppHandle {
    /// Resolved app data directory. In Tauri this came from
    /// `tauri::Manager::app_data_dir()` (platform-specific: `~/Library`
    /// on macOS, `%APPDATA%` on Windows, `~/.local/share` on Linux).
    /// In GPUI we resolve it ourselves in [`AppHandle::new`] using the
    /// same `dirs` crate logic.
    data_dir: PathBuf,
}

impl AppHandle {
    /// Construct from an explicit data dir. Used in tests (point at
    /// `tempfile::tempdir()`) and in `main.rs` (point at the real
    /// platform dir).
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Resolve the platform-appropriate data dir at runtime.
    ///
    /// Mirrors Tauri's `app_data_dir()` logic: macOS →
    /// `~/Library/Application Support/<bundle>`, Windows →
    /// `%APPDATA%/<bundle>`, Linux → `~/.local/share/<bundle>`.
    ///
    /// The bundle identifier is `dev.fileterm` (same as Tauri's
    /// `tauri.conf.json` identifier) so the GPUI runtime shares the data
    /// directory with the Tauri runtime — per refactor.md decision 8
    /// ("数据目录：与 Tauri 共享").
    pub fn platform_default() -> Result<Self> {
        let dir = dirs::data_dir().ok_or_else(|| {
            AppError::Storage("could not resolve platform data dir".into())
        })?;
        Ok(Self::new(dir.join("dev.fileterm")))
    }

    /// Equivalent of `tauri::Manager::app_data_dir()`.
    ///
    /// Tauri returns `Result<PathBuf, tauri::Error>`; we return `&Path`
    /// because the dir is resolved at construction time and can't fail
    /// here. Callers that expected `Result<PathBuf>` (the storage fork)
    /// use [`Self::app_data_dir_result`] to keep the same `?` shape.
    pub fn app_data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `Result`-returning variant for call sites that mirror Tauri's
    /// `app.app_data_dir()?` pattern. Never actually errors — kept so
    /// the forked code's `?` operators compile without rewrite.
    pub fn app_data_dir_result(&self) -> std::result::Result<PathBuf, AppError> {
        Ok(self.data_dir.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_roundtrips() {
        let handle = AppHandle::new(PathBuf::from("/tmp/fileterm-test"));
        assert_eq!(handle.app_data_dir(), Path::new("/tmp/fileterm-test"));
        assert_eq!(
            handle.app_data_dir_result().unwrap(),
            PathBuf::from("/tmp/fileterm-test")
        );
    }

    #[test]
    fn platform_default_does_not_panic() {
        // In CI / headless environments `dirs::data_dir()` may return
        // None (no HOME set). We just assert it doesn't panic; the
        // `Err` path is exercised when HOME is unset.
        let _ = AppHandle::platform_default();
    }
}
