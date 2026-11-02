//! One listener abstraction over a Unix domain socket and a Windows named
//! pipe. Everything above this module sees only `AsyncRead + AsyncWrite`.

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::status::McpFailureCode;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[cfg(not(any(unix, windows)))]
compile_error!("the local MCP endpoint is implemented for Unix and Windows only");

pub(crate) trait IpcStream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> IpcStream for T {}

pub(crate) type Stream = Box<dyn IpcStream>;

pub(crate) struct Listener {
    inner: platform::Listener,
}

impl Listener {
    /// Creates the endpoint. `instance_id` is unique per start; the Windows
    /// pipe name embeds it so the name cannot be predicted by another process.
    pub(crate) async fn bind(data_dir: &Path, instance_id: &str) -> Result<Self, McpFailureCode> {
        platform::Listener::bind(data_dir, instance_id)
            .await
            .map(|inner| Self { inner })
    }

    /// Absolute socket path or pipe name, as written to the discovery file.
    pub(crate) fn endpoint(&self) -> String {
        self.inner.endpoint()
    }

    pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
        self.inner.accept().await
    }
}

pub(crate) fn failure_code(error: &io::Error) -> McpFailureCode {
    match error.kind() {
        io::ErrorKind::PermissionDenied => McpFailureCode::PermissionDenied,
        io::ErrorKind::AddrInUse => McpFailureCode::EndpointInUse,
        _ => McpFailureCode::EndpointCreateFailed,
    }
}
