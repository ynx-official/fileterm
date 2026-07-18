//! Bridge layer: the in-process async API surface that GPUI views call to
//! reach backend services.
//!
//! G0 phase of `docs/plans/active/gpui-refactor.md` section 6.1.
//!
//! ## What G0 delivers
//!
//! Just the **trait shell** + a stub implementation. The trait declares the
//! domain-grouped method categories (matching the inventory's 14 sections),
//! but every method body in `GpuiDesktopApi` returns `AppError::Unsupported`
//! — the real implementations land incrementally in G1 (storage fork),
//! G2 (window/tray/menu), G3 (SSH terminal), G4 (SFTP + transfer),
//! G5 (detach + release).
//!
//! ## Why a trait at all?
//!
//! Tauri's `invoke_handler!` macro registers 108 commands against a concrete
//! `AppHandle`. In GPUI there's no IPC boundary, so views could in principle
//! call backend fns directly. We still introduce a trait for three reasons:
//!
//! 1. **Testability**: views can be constructed against a mock
//!    `FileTermDesktopApi` in unit tests, without spinning up real SSH/SFTP.
//! 2. **Migration ordering**: the trait is the contract the view layer
//!    compiles against today, so as G1–G5 fill in methods one by one, the
//!    view layer never needs to change its call sites — only the impl
//!    behind the trait grows.
//! 3. **Future flexibility**: if we later split the backend into a separate
//!    process (e.g. for sandboxing), the trait is the natural seam.
//!
//! ## Method shape
//!
//! Every method is `async fn` and returns `Result<T>`. This matches the
//! Tauri-side `#[tauri::command] async fn` shape line-for-line, minus the
//! `app: AppHandle` / `state: State<...>` / `window: WebviewWindow`
//! framework params (which don't exist in-process). The inventory's 108
//! commands map 1:1 to trait methods in the same domain groupings.

use async_trait::async_trait;

use crate::error::Result;

/// In-process bridge between GPUI views and FileTerm backend services.
///
/// Method categories mirror `docs/plans/active/gpui-migration-inventory.md`
/// section 1's 14 groups. G0 only declares the trait + stub impl; each
/// method's real signature lands together with its backend in the phase
/// that needs it (see the `// G1` / `// G3` / etc. comments below).
///
/// The trait is `Send + Sync` because it's held inside gpui entities
/// (`Arc<dyn FileTermDesktopApi>`) which must be `Send` for the foreground
/// executor's `Task`s to capture them.
#[async_trait]
pub trait FileTermDesktopApi: Send + Sync {
    // ===== G1 (storage fork) — inventory 1.1 / 1.14 =====
    // Profile-folder CRUD + local-files storage core land first because
    // every other feature reads from / writes to storage.

    /// Inventory 1.1 stub. Real signature lands in G1 when storage is forked.
    ///
    /// G0 returns `Unsupported` so view-layer scaffolding can compile
    /// against the trait without waiting for storage migration.
    async fn app_get_platform(&self) -> Result<String>;

    // ===== G2 (window/tray/menu) — inventory 1.2 / 1.6 =====
    // WindowRegistry + 7 window kinds + tray + native menu.

    /// Inventory 1.2 stub. Real signature lands in G2 with WindowRegistry.
    async fn workspace_list_windows(&self) -> Result<Vec<()>>;

    // ===== G3 (SSH terminal) — inventory 1.3 =====
    // russh shell channel + TermView + SystemSidebar + CWD 跟随.

    /// Inventory 1.3 stub. Real signature lands in G3 with russh integration.
    async fn ssh_connect(&self) -> Result<()>;

    // ===== G4 (SFTP + transfer) — inventory 1.4 / 1.5 =====
    // russh-sftp + TransferService + TransferCenter.

    /// Inventory 1.4 stub. Real signature lands in G4 with russh-sftp.
    async fn sftp_list(&self) -> Result<Vec<()>>;

    /// Inventory 1.5 stub. Real signature lands in G4 with TransferService.
    async fn transfer_create(&self) -> Result<()>;

    // ===== G5 (detach + release) — inventory 1.2 (detach) =====
    // detached-session windows + tab drag + 3-platform packaging.

    /// Inventory 1.2 (detach) stub. Real signature lands in G5.
    async fn workspace_detach_tab(&self) -> Result<()>;

    // ===== Lower-priority groups (P1/P2) — inventory 1.7–1.13 =====
    // tunnel / webdav / ssh-key / command-template / ui-state / update /
    // clipboard / external. These get their method declarations as each
    // phase needs them; G0 leaves them out entirely to keep the trait
    // surface minimal.
}

/// G0 stub implementation: every method returns `AppError::Unsupported`.
///
/// Held by the main window's root view as `Arc<dyn FileTermDesktopApi>`
/// so view code compiles today and silently no-ops until G1+ wires real
/// backends. Once a method gets a real impl, replace the
/// `Err(AppError::Unsupported(...))` line with the actual logic and
/// remove the `// G0 stub` comment.
#[derive(Default)]
pub struct GpuiDesktopApi;

#[async_trait]
impl FileTermDesktopApi for GpuiDesktopApi {
    async fn app_get_platform(&self) -> Result<String> {
        Err(crate::error::AppError::Unsupported(
            "app_get_platform (G1: storage fork)",
        ))
    }

    async fn workspace_list_windows(&self) -> Result<Vec<()>> {
        Err(crate::error::AppError::Unsupported(
            "workspace_list_windows (G2: WindowRegistry)",
        ))
    }

    async fn ssh_connect(&self) -> Result<()> {
        Err(crate::error::AppError::Unsupported(
            "ssh_connect (G3: russh integration)",
        ))
    }

    async fn sftp_list(&self) -> Result<Vec<()>> {
        Err(crate::error::AppError::Unsupported(
            "sftp_list (G4: russh-sftp)",
        ))
    }

    async fn transfer_create(&self) -> Result<()> {
        Err(crate::error::AppError::Unsupported(
            "transfer_create (G4: TransferService)",
        ))
    }

    async fn workspace_detach_tab(&self) -> Result<()> {
        Err(crate::error::AppError::Unsupported(
            "workspace_detach_tab (G5: detach + release)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// G0 acceptance: the stub compiles, is `Send + Sync`, and every
    /// method returns `Unsupported` with a phase tag. As G1+ lands, each
    /// method's test gets a real assertion; for now we just verify the
    /// trait is usable as `Arc<dyn FileTermDesktopApi>` (the shape view
    /// code will hold).
    #[tokio::test]
    async fn stub_is_usable_as_dyn_arc() {
        let api: Arc<dyn FileTermDesktopApi> = Arc::new(GpuiDesktopApi);
        let err = api.app_get_platform().await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::AppError::Unsupported(name) if name.contains("G1")
        ));
    }

    /// Every stub method returns `Unsupported`. Once a method gets a real
    /// impl, remove its line from this test rather than weakening the
    /// assertion.
    #[tokio::test]
    async fn all_stubs_return_unsupported() {
        let api = GpuiDesktopApi;
        assert!(api.app_get_platform().await.is_err());
        assert!(api.workspace_list_windows().await.is_err());
        assert!(api.ssh_connect().await.is_err());
        assert!(api.sftp_list().await.is_err());
        assert!(api.transfer_create().await.is_err());
        assert!(api.workspace_detach_tab().await.is_err());
    }
}
