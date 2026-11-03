//! Local MCP server lifecycle inside the desktop host.
//!
//! The persisted switch is `settings.mcp.enabled` (default off). The server
//! runs only while the switch is on *and* the core is ready: `reconcile` is
//! called at startup (via the core's ready hook, after the single-instance
//! plugin has let this process live), when the user flips the switch, and
//! `shutdown` runs on real app exit. Hiding the main window in background mode
//! does not touch it.

use std::sync::{Arc, Mutex};

use mailx_core::{ApiResult, AppError};
use mailx_mcp_server::{McpConfig, McpFailureCode, McpHandle, McpServer, McpStatus, StderrAudit};
use serde::Deserialize;
use tauri::async_runtime::JoinHandle;
use tauri::{Emitter, EventTarget, Manager, State, WebviewWindow};

use crate::commands::{EmptyArgs, authorize, parse, respond};
use crate::mcp_sidecar::McpClientConfig;
use crate::state::DesktopState;

pub(crate) const STATUS_EVENT: &str = "mailx://mcp-status-changed";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Start,
    Stop,
    Keep,
}

/// What to do given the switch, core readiness and whether a healthy server
/// is running. A failed server counts as not running so that turning the
/// switch on again retries.
pub(crate) fn plan(wanted: bool, core_ready: bool, running: bool) -> Action {
    match (wanted, running) {
        (true, false) if core_ready => Action::Start,
        (false, true) => Action::Stop,
        _ => Action::Keep,
    }
}

/// Status shown after a start attempt.
pub(crate) fn status_after_start(result: &Result<(), McpFailureCode>) -> Option<McpStatus> {
    result
        .as_ref()
        .err()
        .map(|code| McpStatus::Failed { code: *code })
}

#[derive(Default)]
struct Slot {
    handle: Option<McpHandle>,
    watcher: Option<JoinHandle<()>>,
}

struct Inner {
    slot: tokio::sync::Mutex<Slot>,
    status: Mutex<McpStatus>,
}

#[derive(Clone)]
pub(crate) struct McpManager(Arc<Inner>);

impl McpManager {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            slot: tokio::sync::Mutex::new(Slot::default()),
            status: Mutex::new(McpStatus::Off),
        }))
    }

    pub(crate) fn status(&self) -> McpStatus {
        self.0.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn publish(&self, app: &tauri::AppHandle, status: McpStatus) {
        if let Ok(mut current) = self.0.status.lock() {
            if *current == status {
                return;
            }
            current.clone_from(&status);
        }
        let _ = app.emit_to(
            EventTarget::webview_window(crate::window_chrome::MAIN_WINDOW),
            STATUS_EVENT,
            status,
        );
    }

    /// Brings the server in line with the persisted switch. Serialized, so
    /// concurrent flips and the ready hook cannot start two servers.
    pub(crate) async fn reconcile(&self, app: &tauri::AppHandle) {
        let state = app.state::<DesktopState>().inner().clone();
        let mut slot = self.0.slot.lock().await;

        // A server that failed on its own is cleaned up so it can restart.
        if slot
            .handle
            .as_ref()
            .is_some_and(|handle| !matches!(handle.status(), McpStatus::Running { .. }))
        {
            self.stop_slot(app, &mut slot, false).await;
        }

        let wanted = state.mcp_enabled();
        let core = state.core().ok();
        match plan(wanted, core.is_some(), slot.handle.is_some()) {
            Action::Keep => {}
            Action::Stop => self.stop_slot(app, &mut slot, true).await,
            Action::Start => {
                let (Some(core), Ok(data_dir)) = (core, state.data_dir()) else {
                    return;
                };
                let default_state = state.clone();
                let tools =
                    mailx_core::tools::Tools::new(core, move || default_state.default_account_id())
                        .with_locale({
                            let locale_state = state.clone();
                            // The same locale the UI's reply command uses.
                            move || {
                                locale_state
                                    .notification_settings()
                                    .map(|(_, locale)| locale)
                                    .unwrap_or_else(crate::locale::system_locale)
                            }
                        })
                        .with_audit(Arc::new(StderrAudit));
                let config = McpConfig {
                    data_dir,
                    tools: Arc::new(tools),
                };
                match McpServer::start(config).await {
                    Ok(handle) => {
                        let mut changes = handle.subscribe();
                        self.publish(app, handle.status());
                        let manager = self.clone();
                        let watcher_app = app.clone();
                        slot.watcher = Some(tauri::async_runtime::spawn(async move {
                            while changes.changed().await.is_ok() {
                                let status = changes.borrow_and_update().clone();
                                manager.publish(&watcher_app, status);
                            }
                        }));
                        slot.handle = Some(handle);
                    }
                    Err(error) => {
                        let failed = status_after_start(&Err(error.code)).unwrap_or_default();
                        eprintln!("[mailx-mcp] start failed: {}", error.code);
                        self.publish(app, failed);
                    }
                }
            }
        }
    }

    async fn stop_slot(&self, app: &tauri::AppHandle, slot: &mut Slot, publish_off: bool) {
        if let Some(watcher) = slot.watcher.take() {
            watcher.abort();
        }
        if let Some(handle) = slot.handle.take() {
            let before = handle.status();
            handle.stop().await;
            if publish_off || matches!(before, McpStatus::Running { .. }) {
                self.publish(app, McpStatus::Off);
            }
        }
    }

    /// Stops the server and waits; called once on real exit.
    pub(crate) fn shutdown(&self, app: &tauri::AppHandle) {
        let manager = self.clone();
        let app = app.clone();
        tauri::async_runtime::block_on(async move {
            let mut slot = manager.0.slot.lock().await;
            manager.stop_slot(&app, &mut slot, true).await;
        });
    }
}

/// Registers the manager and starts the server once the core is ready.
pub(crate) fn setup(app: &tauri::AppHandle, state: &DesktopState) {
    let manager = McpManager::new();
    app.manage(manager.clone());
    let hook_app = app.clone();
    state.set_ready_hook(move || {
        let manager = hook_app.state::<McpManager>().inner().clone();
        let app = hook_app.clone();
        tauri::async_runtime::spawn(async move { manager.reconcile(&app).await });
    });
}

pub(crate) fn shutdown(app: &tauri::AppHandle) {
    if let Some(manager) = app.try_state::<McpManager>() {
        manager.shutdown(app);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetEnabledArgs {
    enabled: bool,
}

#[tauri::command]
pub(crate) async fn mcp_get_status(
    window: WebviewWindow,
    manager: State<'_, McpManager>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<McpStatus>, ()> {
    let args =
        authorize(&window, "mcp_get_status").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(args.map(|_| manager.status())))
}

#[tauri::command]
pub(crate) async fn mcp_set_enabled(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    manager: State<'_, McpManager>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<McpStatus>, ()> {
    let args =
        authorize(&window, "mcp_set_enabled").and_then(|_| parse::<SetEnabledArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => set_enabled(&app, &state, &manager, args.enabled).await,
        Err(error) => Err(error),
    }))
}

/// Command and arguments a client needs to start the bridge. Reads no secret:
/// the discovery file's token is only ever read by the bridge itself.
#[tauri::command]
pub(crate) async fn mcp_client_config(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<McpClientConfig>, ()> {
    let args =
        authorize(&window, "mcp_client_config").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(args.and_then(|_| {
        crate::mcp_sidecar::current_client_config(&state.data_dir()?)
    })))
}

async fn set_enabled(
    app: &tauri::AppHandle,
    state: &DesktopState,
    manager: &McpManager,
    enabled: bool,
) -> Result<McpStatus, AppError> {
    let snapshot = state
        .settings(move |store| store.set_mcp_enabled(enabled))
        .await?;
    crate::commands::emit_settings_changed(app, &snapshot);
    manager.reconcile(app).await;
    Ok(manager.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_starts_only_when_wanted_ready_and_not_running() {
        assert_eq!(plan(true, true, false), Action::Start);
        assert_eq!(plan(true, false, false), Action::Keep);
        assert_eq!(plan(true, true, true), Action::Keep);
        assert_eq!(plan(false, true, false), Action::Keep);
        assert_eq!(plan(false, false, false), Action::Keep);
    }

    #[test]
    fn plan_stops_whenever_running_but_not_wanted() {
        assert_eq!(plan(false, true, true), Action::Stop);
        assert_eq!(plan(false, false, true), Action::Stop);
    }

    #[test]
    fn failed_start_maps_to_failed_status() {
        assert_eq!(status_after_start(&Ok(())), None);
        assert_eq!(
            status_after_start(&Err(McpFailureCode::EndpointInUse)),
            Some(McpStatus::Failed {
                code: McpFailureCode::EndpointInUse
            })
        );
    }

    #[test]
    fn switch_is_off_by_default() {
        assert!(!mailx_core::McpSettings::default().enabled);
        assert!(!mailx_core::AppSettings::default().mcp.enabled);
    }
}
