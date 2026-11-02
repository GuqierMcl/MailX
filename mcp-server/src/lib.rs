//! Local MCP server for MailX.
//!
//! Exposes the shared tool layer (`mailx_core::tools`) to MCP clients over a
//! per-user local endpoint (Unix domain socket or Windows named pipe). The
//! endpoint is protected by OS permissions plus a per-start token that the
//! `mailx-mcp` stdio bridge reads from the discovery file. See
//! `docs/contracts/mcp.md`.
//!
//! This crate has no dependency on Tauri or any UI; the desktop host owns the
//! on/off setting and calls [`McpServer::start`] and [`McpHandle::stop`].

mod audit;
mod discovery;
#[cfg(unix)]
mod endpoint;
mod handler;
mod handshake;
mod ipc;
mod limited;
mod server;
mod status;
mod token;

pub use audit::StderrAudit;
pub use discovery::{DISCOVERY_FILE_NAME, DISCOVERY_VERSION, Discovery, discovery_path};
#[cfg(unix)]
pub use endpoint::{MAX_SOCKET_PATH_BYTES, SOCKET_DIR_NAME, SOCKET_FILE_NAME};
pub use handler::TOOL_PREFIX;
pub use handshake::{BRIDGE_PROTOCOL, HANDSHAKE_TIMEOUT, MAX_HANDSHAKE_BYTES};
pub use limited::MAX_MESSAGE_BYTES;
pub use server::{MAX_CONNECTIONS, McpConfig, McpHandle, McpServer};
pub use status::{McpClient, McpFailureCode, McpStartError, McpStatus};
