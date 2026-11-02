//! Server lifecycle: start, accept loop, sessions, stop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailx_core::tools::Tools;
use rmcp::ServiceExt as _;
use rmcp::model::Tool;
use rmcp::transport::async_rw::AsyncRwTransport;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::discovery::{self, DISCOVERY_VERSION, Discovery};
use crate::handler::{MailxMcp, build_catalog};
use crate::handshake::{HandshakeError, server_handshake};
use crate::ipc::{Listener, Stream};
use crate::limited::{LimitedRead, MAX_MESSAGE_BYTES};
use crate::status::{McpFailureCode, McpStartError, McpStatus};
use crate::token::random_token;

/// Authenticated sessions allowed at once. Further connections are refused
/// after the handshake with `{"ok":false}`. MCP clients start one bridge each,
/// so eight covers several editors and agents at the same time.
pub const MAX_CONNECTIONS: usize = 8;

/// Connections (authenticated or not) the accept loop keeps in flight. Bounds
/// what a local process that has no token can hold open during the 5 s
/// handshake window.
const MAX_INFLIGHT: usize = 32;

/// Slack above the size limit so rmcp's own line limit never fires first.
const TRANSPORT_LINE_SLACK: usize = 64 * 1024;

const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(200);
const MAX_CONSECUTIVE_ACCEPT_ERRORS: u32 = 25;

/// Everything the server needs from its host.
pub struct McpConfig {
    /// Application data directory: holds the discovery file and, when the
    /// path is short enough, the socket.
    pub data_dir: PathBuf,
    /// The shared tool layer; the server serves `Audience::Mcp`.
    pub tools: Arc<Tools>,
}

pub struct McpServer;

impl McpServer {
    /// Binds the endpoint, publishes a fresh token in the discovery file and
    /// starts accepting. Safe to call again after [`McpHandle::stop`].
    pub async fn start(config: McpConfig) -> Result<McpHandle, McpStartError> {
        let fail = |code| McpStartError { code };
        let token = random_token().map_err(|_| fail(McpFailureCode::TokenUnavailable))?;
        let instance_id = uuid::Uuid::new_v4().simple().to_string();
        let listener = Listener::bind(&config.data_dir, &instance_id)
            .await
            .map_err(fail)?;

        let discovery_path = discovery::discovery_path(&config.data_dir);
        let record = Discovery {
            version: DISCOVERY_VERSION,
            endpoint: listener.endpoint(),
            token: token.clone(),
            pid: std::process::id(),
            instance_id: instance_id.clone(),
        };
        if let Err(error) = discovery::write_atomic(&discovery_path, &record) {
            // The listener drops here, which removes the socket again.
            return Err(fail(match error.kind() {
                std::io::ErrorKind::PermissionDenied => McpFailureCode::PermissionDenied,
                _ => McpFailureCode::DiscoveryWriteFailed,
            }));
        }

        let (status_tx, status_rx) = watch::channel(McpStatus::Running {
            connections: 0,
            last_client_name: None,
        });
        let shared = Arc::new(Shared::new(status_tx));
        let (stop_tx, stop_rx) = watch::channel(false);
        let catalog = Arc::new(build_catalog(&config.tools));
        let context = AcceptContext {
            listener,
            token,
            tools: config.tools,
            catalog,
            shared: Arc::clone(&shared),
            stop: stop_rx,
        };
        let cleanup = Cleanup {
            discovery_path,
            instance_id,
        };
        let task = tokio::spawn(accept_loop(context, cleanup));
        Ok(McpHandle {
            shared,
            status: status_rx,
            stop: stop_tx,
            task: Mutex::new(Some(task)),
        })
    }
}

/// Handle to a running server. Dropping it without calling `stop` leaves the
/// server running until the process exits; hosts should always `stop`.
pub struct McpHandle {
    shared: Arc<Shared>,
    status: watch::Receiver<McpStatus>,
    stop: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl McpHandle {
    pub fn status(&self) -> McpStatus {
        self.status.borrow().clone()
    }

    /// Receiver that wakes whenever the status changes.
    pub fn subscribe(&self) -> watch::Receiver<McpStatus> {
        self.status.clone()
    }

    /// Test hook: turns the per-call enabled check off without aborting any
    /// session, to prove the check itself rejects calls.
    #[doc(hidden)]
    pub fn disable_calls_only_for_tests(&self) {
        if let Ok(mut inner) = self.shared.inner.lock() {
            inner.enabled = false;
        }
    }

    /// Stops accepting, rejects new calls at once, disconnects every session,
    /// removes the socket and (if still ours) the discovery file, and waits for
    /// all of that. The token stops being valid immediately. Idempotent.
    pub async fn stop(&self) {
        self.shared.disable();
        let _ = self.stop.send(true);
        let task = self.task.lock().ok().and_then(|mut task| task.take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

/// State shared between the accept loop, sessions and the handle.
pub(crate) struct Shared {
    inner: Mutex<SharedInner>,
    status: watch::Sender<McpStatus>,
}

struct SharedInner {
    enabled: bool,
    connections: u32,
    last_client_name: Option<String>,
}

impl Shared {
    fn new(status: watch::Sender<McpStatus>) -> Self {
        Self {
            inner: Mutex::new(SharedInner {
                enabled: true,
                connections: 0,
                last_client_name: None,
            }),
            status,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.inner.lock().is_ok_and(|inner| inner.enabled)
    }

    pub(crate) fn note_client(&self, name: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_client_name = (!name.is_empty()).then(|| name.to_owned());
            self.publish(&inner);
        }
    }

    fn disable(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.enabled = false;
            inner.connections = 0;
            self.status.send_replace(McpStatus::Off);
        }
    }

    fn fail(&self, code: McpFailureCode) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.enabled = false;
            inner.connections = 0;
            self.status.send_replace(McpStatus::Failed { code });
        }
    }

    fn try_admit(self: &Arc<Self>) -> Option<SessionGuard> {
        let mut inner = self.inner.lock().ok()?;
        if !inner.enabled || inner.connections as usize >= MAX_CONNECTIONS {
            return None;
        }
        inner.connections += 1;
        self.publish(&inner);
        Some(SessionGuard(Arc::clone(self)))
    }

    fn publish(&self, inner: &SharedInner) {
        if inner.enabled {
            self.status.send_replace(McpStatus::Running {
                connections: inner.connections,
                last_client_name: inner.last_client_name.clone(),
            });
        }
    }
}

/// Counts one authenticated session for as long as it lives.
struct SessionGuard(Arc<Shared>);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.0.inner.lock() {
            inner.connections = inner.connections.saturating_sub(1);
            self.0.publish(&inner);
        }
    }
}

struct Cleanup {
    discovery_path: PathBuf,
    instance_id: String,
}

struct AcceptContext {
    listener: Listener,
    token: String,
    tools: Arc<Tools>,
    catalog: Arc<Vec<Tool>>,
    shared: Arc<Shared>,
    stop: watch::Receiver<bool>,
}

async fn accept_loop(mut context: AcceptContext, cleanup: Cleanup) {
    let mut sessions = JoinSet::new();
    let mut consecutive_errors = 0_u32;
    loop {
        tokio::select! {
            _ = context.stop.changed() => break,
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
            accepted = context.listener.accept() => match accepted {
                Ok(stream) => {
                    consecutive_errors = 0;
                    if sessions.len() >= MAX_INFLIGHT {
                        drop(stream);
                        continue;
                    }
                    sessions.spawn(run_connection(
                        stream,
                        context.token.clone(),
                        Arc::clone(&context.tools),
                        Arc::clone(&context.catalog),
                        Arc::clone(&context.shared),
                    ));
                }
                Err(_) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ACCEPT_ERRORS {
                        context.shared.fail(McpFailureCode::AcceptFailed);
                        break;
                    }
                    tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                }
            },
        }
    }
    // Invalidate the token first, then drop every session, then the endpoint.
    discovery::remove_if_mine(&cleanup.discovery_path, &cleanup.instance_id);
    sessions.shutdown().await;
    drop(context.listener);
}

async fn run_connection(
    mut stream: Stream,
    token: String,
    tools: Arc<Tools>,
    catalog: Arc<Vec<Tool>>,
    shared: Arc<Shared>,
) {
    let admit_shared = Arc::clone(&shared);
    let guard = match server_handshake(&mut stream, &token, move || admit_shared.try_admit()).await
    {
        Ok(guard) => guard,
        Err(HandshakeError::Busy | HandshakeError::BadToken) => {
            eprintln!("[mailx-mcp] connection refused");
            return;
        }
        Err(_) => return,
    };
    let connection_id = uuid::Uuid::new_v4().simple().to_string();
    let handler = MailxMcp::new(tools, catalog, shared, connection_id);
    serve_stream(handler, stream).await;
    drop(guard);
}

async fn serve_stream<S>(handler: MailxMcp, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (read, write) = tokio::io::split(stream);
    let read = LimitedRead::new(read, MAX_MESSAGE_BYTES);
    let transport = AsyncRwTransport::new_server(read, write)
        .with_max_line_length(MAX_MESSAGE_BYTES + TRANSPORT_LINE_SLACK);
    if let Ok(running) = handler.serve(transport).await {
        let _ = running.waiting().await;
    }
}
