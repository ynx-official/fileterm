//! Service-layer modules for the GPUI runtime.
//!
//! Services remain framework-independent and are connected to views through
//! `FileTermDesktopApi`. Storage/profile operations are live; protocol,
//! transfer, update, and other services are added with their vertical slices.

pub mod logging;
pub mod profile_ops;
pub mod ssh_keys;
pub mod ssh_profile;
