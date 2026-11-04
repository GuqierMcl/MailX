//! Current-user login startup. No registration is written during app setup.
//! Only the trusted main window can change it; the OS entry is the source of
//! truth, independent of the mail database and of generic preference saves.

#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod file;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use mailx_core::{ApiResult, AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::commands::{authorize, parse, respond};

pub(super) const MARKER: &str = "--mailx-autostart";
pub(super) const ENTRY_ID: &str = "com.mailxapp.desktop.autostart";

/// Available before plugins start, so an early manual second launch is not
/// lost while the first instance is still creating its hidden main window.
#[derive(Default)]
pub(crate) struct StartupIntent(AtomicBool);

/// The login marker is a one-shot launch source. In particular the Windows
/// updater captures this managed Env before its installer exits the process.
pub(crate) fn foreground_environment(mut environment: tauri::Env) -> tauri::Env {
    environment.args_os = environment
        .args_os
        .into_iter()
        .enumerate()
        .filter_map(|(index, arg)| (index == 0 || arg != MARKER).then_some(arg))
        .collect();
    environment
}

pub(crate) fn user_activated<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    app.try_state::<StartupIntent>()
        .is_some_and(|state| state.0.load(Ordering::SeqCst))
}

pub(crate) fn record_activation<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(state) = app.try_state::<StartupIntent>() {
        state.0.store(true, Ordering::SeqCst);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
type NativeEntry = file::Entry;
#[cfg(target_os = "windows")]
type NativeEntry = windows::Entry;

pub(crate) struct AutostartState {
    /// Serializes reads and writes, including a read back after a change.
    entry: Mutex<Result<Option<NativeEntry>, ()>>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    enabled: bool,
    available: bool,
}

impl AutostartState {
    pub(crate) fn new(app: &AppHandle) -> Self {
        // Development and isolated test profiles must never register a debug
        // executable or touch the installed application's login entry.
        let entry = if cfg!(debug_assertions) || std::env::var_os("MAILX_DATA_DIR").is_some() {
            Ok(None)
        } else {
            match NativeEntry::current(app) {
                Ok(entry) => Ok(Some(entry)),
                Err(error) if error.kind() == std::io::ErrorKind::Unsupported => Ok(None),
                Err(_) => Err(()),
            }
        };
        Self {
            entry: Mutex::new(entry),
        }
    }

    fn access(&self, wanted: Option<bool>) -> Result<Snapshot, AppError> {
        let slot = self.entry.lock().map_err(|_| failure())?;
        let entry = slot.as_ref().map_err(|_| failure())?;
        let Some(entry) = entry else {
            return if wanted.is_some() {
                Err(AppError::new(ErrorCode::Unsupported))
            } else {
                Ok(Snapshot {
                    enabled: false,
                    available: false,
                })
            };
        };
        if let Some(enabled) = wanted {
            entry.set(enabled).map_err(|_| failure())?;
        }
        let enabled = entry.enabled().map_err(|_| failure())?;
        if wanted.is_some_and(|wanted| wanted != enabled) {
            return Err(failure());
        }
        Ok(Snapshot {
            enabled,
            available: true,
        })
    }
}

fn failure() -> AppError {
    AppError {
        field: Some("autostart".into()),
        ..AppError::new(ErrorCode::StorageUnavailable)
    }
}

pub(crate) fn is_login_launch(args: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    args.into_iter().skip(1).any(|arg| arg.as_ref() == MARKER)
}

/// The window starts hidden for a login launch. Resolve preferences and OS
/// state off the UI thread, then let lifecycle wait for the tray's first check.
pub(crate) fn prepare_startup(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let worker_app = app.clone();
        let silent = tauri::async_runtime::spawn_blocking(move || {
            let enabled = worker_app
                .state::<AutostartState>()
                .access(None)
                .is_ok_and(|snapshot| snapshot.enabled);
            enabled
                && worker_app
                    .state::<crate::state::DesktopState>()
                    .silent_start()
        })
        .await
        .unwrap_or(false);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || crate::lifecycle::startup_prepared(&handle, silent));
    });
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    enabled: bool,
}

#[tauri::command]
pub(crate) async fn autostart_get(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Snapshot>, ()> {
    let args = authorize(&window, "autostart_get").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Err(error) => Err(error),
        Ok(_) => access(window.app_handle().clone(), None).await,
    }))
}

#[tauri::command]
pub(crate) async fn autostart_set(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Snapshot>, ()> {
    let args = authorize(&window, "autostart_set").and_then(|_| parse::<SetArgs>(request.body()));
    Ok(respond(match args {
        Err(error) => Err(error),
        Ok(args) => access(window.app_handle().clone(), Some(args.enabled)).await,
    }))
}

async fn access(app: AppHandle, wanted: Option<bool>) -> Result<Snapshot, AppError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AutostartState>().access(wanted))
        .await
        .map_err(|_| failure())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_profiles_never_write_a_login_registration() {
        let state = AutostartState {
            entry: Mutex::new(Ok(None)),
        };
        let snapshot = state.access(None).unwrap();
        assert!(!snapshot.enabled && !snapshot.available);
        assert_eq!(
            state.access(Some(true)).err().unwrap().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            state.access(Some(false)).err().unwrap().code,
            ErrorCode::Unsupported
        );
    }

    #[test]
    fn only_the_explicit_login_marker_selects_login_startup() {
        assert!(is_login_launch(["MailX", MARKER]));
        assert!(!is_login_launch([MARKER]));
        assert!(!is_login_launch([
            "MailX",
            "--silent",
            "--mailx-autostart=false"
        ]));
    }

    #[test]
    fn relaunches_keep_other_arguments_but_do_not_inherit_the_login_source() {
        let mut environment = tauri::Env::default();
        environment.args_os = ["MailX", MARKER, "--other", "value"]
            .map(Into::into)
            .to_vec();
        let expected = ["MailX", "--other", "value"].map(std::ffi::OsString::from);
        let environment = foreground_environment(environment);
        assert_eq!(environment.args_os, expected);
        // Verify Tauri preserves an Env supplied on the builder; this is also
        // the environment read by the Windows updater's internal relaunch.
        let app = tauri::test::mock_builder()
            .manage(environment)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        assert_eq!(app.env().args_os, expected);
    }

    #[test]
    fn startup_commands_accept_no_paths_or_extra_fields() {
        assert!(serde_json::from_value::<SetArgs>(serde_json::json!({"enabled":true})).is_ok());
        for value in [
            serde_json::json!({}),
            serde_json::json!({"enabled":"true"}),
            serde_json::json!({"enabled":true,"path":"/tmp/other"}),
        ] {
            assert!(serde_json::from_value::<SetArgs>(value).is_err());
        }
    }
}
