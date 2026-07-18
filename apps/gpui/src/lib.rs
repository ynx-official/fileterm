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
//! ## Current phase: product integration
//!
//! The native shell, terminal core, shared storage bridge, connection-library
//! projection, and window/menu foundations are wired. Protocol/session and
//! product views are being connected as runnable vertical slices; methods
//! that still return `AppError::Unsupported` identify the remaining migration
//! boundary explicitly.

pub mod backend;
pub mod error;
pub mod services;
pub mod sftp;
pub mod ssh;
pub mod state;
pub mod term;
pub mod theme;
pub mod view;
pub mod window;
