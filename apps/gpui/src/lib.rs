//! FileTerm GPUI runtime.
//!
//! ```text
//! GPUI Views
//!   ↓
//! In-process bridge contracts
//!   ↓
//! Runtime services and session controllers
//!   ↓
//! Protocol adapters and operating-system capabilities
//! ```
//!
//! Views own presentation and window coordination. Persistent data and system
//! operations stay behind services; live protocol operations stay scoped to
//! their owning session controller.

pub mod backend;
pub mod error;
pub mod ftp;
pub mod services;
pub mod sftp;
pub mod ssh;
pub mod state;
pub mod term;
pub mod theme;
pub mod view;
pub mod window;
