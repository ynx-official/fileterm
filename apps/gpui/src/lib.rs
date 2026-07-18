//! FileTerm GPUI runtime.
//!
//! Layer layout (target state per `docs/plans/active/gpui-refactor.md`
//! section 3.1):
//!
//! ```text
//! GPUI View (Render + Element)
//!   ↓ 调用
//! Bridge (in-process async fn, FileTermDesktopApi trait)
//!   ↓ 委托
//! Backend Services (fork from Tauri)
//!   ↓ 驱动
//! Session Controllers (russh / suppaftp / tokio-serial)
//!   ↓
//! Protocol Adapters → Remote Servers
//! ```
//!
//! ## Current phase: G0 (scaffold)
//!
//! Only `term` (from the G-1 spike) and the `backend` trait shell +
//! `error` types are wired up. `backend::GpuiDesktopApi` returns
//! `AppError::Unsupported` for every method — real implementations land
//! incrementally in G1 (storage), G2 (window/tray/menu), G3 (SSH
//! terminal), G4 (SFTP + transfer), G5 (detach + release).

pub mod backend;
pub mod error;
pub mod services;
pub mod term;
