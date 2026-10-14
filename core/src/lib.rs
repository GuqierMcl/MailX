//! Shared, host-independent foundation for MailX.
//!
//! Mail services, protocol connections, storage, and account routing will live
//! in this crate. Desktop and server hosts depend on it, never the reverse.
//! Mail functionality is not implemented yet.

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
