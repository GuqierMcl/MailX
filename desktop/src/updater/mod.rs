//! The in-app updater host (see `docs/architecture/updater.md`).
//!
//! The webview never talks to `tauri-plugin-updater`: it only sees the state
//! machine below through main-window commands and one event. The downloaded
//! package lives in memory until it is installed on exit, so cancelling a
//! download is just aborting the task that owns the request.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use mailx_core::{AppError, ErrorCode, OutboxState};
use serde::Deserialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{EmptyArgs, authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_chrome::MAIN_WINDOW;

mod policy;

use policy::{
    AutoCheck, CHECK_TIMEOUT, DOWNLOAD_TIMEOUT, FailureStage, ReleaseDetails, Snapshot,
    UpdateState, auto_check, cap_notes, is_newer, next_wait, progress_due,
};

pub(crate) const STATE_EVENT: &str = "mailx://updater-state";

/// Why a restart or quit-time install cannot go ahead right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NothingReady,
    Sending,
}

#[derive(Default)]
struct Inner {
    state: Option<UpdateState>,
    update: Option<Update>,
    package: Option<Arc<Vec<u8>>>,
    download: Option<JoinHandle<()>>,
    /// Bumped whenever a download starts or is cancelled, so a late task
    /// cannot write into a newer attempt.
    generation: u64,
    restart_after_install: bool,
    scheduler_started: bool,
}

#[derive(Clone, Default)]
pub(crate) struct UpdaterState(Arc<Mutex<Inner>>);

impl UpdaterState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn snapshot(&self) -> UpdateState {
        self.lock().state.clone().unwrap_or(UpdateState::Idle)
    }
}

fn set_state(app: &AppHandle, state: &UpdaterState, next: UpdateState) {
    state.lock().state = Some(next.clone());
    emit(app, &next);
}

/// The desktop edition installs what it finds.
fn desktop(state: UpdateState) -> Snapshot {
    Snapshot {
        can_install: true,
        state,
    }
}

fn emit(app: &AppHandle, state: &UpdateState) {
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        STATE_EVENT,
        desktop(state.clone()),
    );
}

fn release_details(update: &Update) -> ReleaseDetails {
    ReleaseDetails {
        current: update.current_version.clone(),
        notes: cap_notes(update.body.clone()),
        date: update.date.map(|date| date.unix_timestamp()),
    }
}

/// Checks the release manifest. A failed automatic check stays silent; a
/// failed manual one is reported to the user.
async fn run_check(app: &AppHandle, state: &UpdaterState, manual: bool) -> UpdateState {
    {
        let mut inner = state.lock();
        let current = inner.state.clone().unwrap_or(UpdateState::Idle);
        if !current.may_check() {
            return current;
        }
        inner.state = Some(UpdateState::Checking);
    }
    emit(app, &UpdateState::Checking);

    let checked = match app.updater_builder().timeout(CHECK_TIMEOUT).build() {
        Ok(updater) => updater.check().await.map_err(|_| ()),
        Err(_) => Err(()),
    };
    let next = match checked {
        Ok(Some(update)) if is_newer(&update.current_version, &update.version) => {
            // The check's 30 s limit is carried into the download by the plugin;
            // the installer needs far longer for its whole body.
            let mut update = update;
            update.timeout = Some(DOWNLOAD_TIMEOUT);
            let next = UpdateState::Available {
                version: update.version.clone(),
                details: release_details(&update),
            };
            state.lock().update = Some(update);
            next
        }
        Ok(_) => UpdateState::Idle,
        Err(()) if manual => UpdateState::Failed {
            stage: FailureStage::Check,
            version: None,
            details: None,
        },
        Err(()) => UpdateState::Idle,
    };
    set_state(app, state, next.clone());
    next
}

async fn run_download(
    app: AppHandle,
    state: UpdaterState,
    update: Update,
    details: ReleaseDetails,
    generation: u64,
) {
    let version = update.version.clone();
    let progress = {
        let app = app.clone();
        let state = state.clone();
        let version = version.clone();
        let details = details.clone();
        let mut downloaded = 0_u64;
        let mut last_emit: Option<Instant> = None;
        let mut last_percent: Option<u64> = None;
        move |chunk: usize, total: Option<u64>| {
            downloaded += chunk as u64;
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| downloaded * 100 / total);
            let now = Instant::now();
            if !progress_due(last_emit, now, last_percent, percent) {
                return;
            }
            last_emit = Some(now);
            last_percent = percent;
            let next = UpdateState::Downloading {
                version: version.clone(),
                details: details.clone(),
                downloaded,
                total,
            };
            {
                let mut inner = state.lock();
                if inner.generation != generation {
                    return;
                }
                inner.state = Some(next.clone());
            }
            emit(&app, &next);
        }
    };
    let result = update.download(progress, || {}).await;

    let next = {
        let mut inner = state.lock();
        if inner.generation != generation {
            return;
        }
        inner.download = None;
        match result {
            Ok(bytes) => {
                inner.package = Some(Arc::new(bytes));
                UpdateState::Ready { version, details }
            }
            Err(_) => UpdateState::Failed {
                stage: FailureStage::Download,
                version: Some(version),
                details: Some(details),
            },
        }
    };
    set_state(&app, &state, next);
}

/// Starts the download unless one is running or nothing was found.
fn start_download(app: &AppHandle, state: &UpdaterState) -> UpdateState {
    let mut inner = state.lock();
    let current = inner.state.clone().unwrap_or(UpdateState::Idle);
    let Some(update) = inner.update.clone().filter(|_| current.may_download()) else {
        return current;
    };
    inner.generation += 1;
    let generation = inner.generation;
    let details = release_details(&update);
    let next = UpdateState::Downloading {
        version: update.version.clone(),
        details: details.clone(),
        downloaded: 0,
        total: None,
    };
    inner.state = Some(next.clone());
    inner.download = Some(tauri::async_runtime::spawn(run_download(
        app.clone(),
        state.clone(),
        update,
        details,
        generation,
    )));
    drop(inner);
    emit(app, &next);
    next
}

/// Aborting the task drops the request and the partial buffer: nothing is
/// written to disk before `install`, so there is nothing to clean up.
fn cancel_download(app: &AppHandle, state: &UpdaterState) -> UpdateState {
    let mut inner = state.lock();
    let current = inner.state.clone().unwrap_or(UpdateState::Idle);
    if !current.may_cancel() {
        return current;
    }
    inner.generation += 1;
    if let Some(task) = inner.download.take() {
        task.abort();
    }
    let next = inner
        .update
        .as_ref()
        .map(|update| UpdateState::Available {
            version: update.version.clone(),
            details: release_details(update),
        })
        .unwrap_or(UpdateState::Idle);
    inner.state = Some(next.clone());
    drop(inner);
    emit(app, &next);
    next
}

/// Installs the downloaded package, if any. Called once, when the process is
/// going away anyway. On Windows the installer ends the process itself.
fn install_downloaded(state: &UpdaterState, relaunch: bool) -> bool {
    let (update, package) = {
        let inner = state.lock();
        match (&inner.update, &inner.package) {
            (Some(update), Some(package))
                if inner.state.as_ref().is_some_and(|s| s.may_restart()) =>
            {
                (update.clone(), package.clone())
            }
            _ => return false,
        }
    };
    update
        .restart_after_install(relaunch)
        .install(package.as_slice())
        .is_ok()
}

/// Final hook of the exit path: every window is closed and every composer
/// has saved. Installs a downloaded update; an explicit restart relaunches.
pub(crate) fn on_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<UpdaterState>() else {
        return;
    };
    let restart = state.lock().restart_after_install;
    let installed = install_downloaded(&state, restart);
    if installed && restart {
        tauri::process::restart(&app.env());
    }
}

/// Spawns the periodic check: shortly after start, then every few hours,
/// until a version has been found.
pub(crate) fn start_scheduler(app: &AppHandle) {
    let state = app.state::<UpdaterState>().inner().clone();
    {
        let mut inner = state.lock();
        if inner.scheduler_started {
            return;
        }
        inner.scheduler_started = true;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            tokio::time::sleep(next_wait(first)).await;
            first = false;
            let enabled = app
                .state::<DesktopState>()
                .auto_check_updates()
                .await
                .unwrap_or_default();
            match auto_check(enabled, &state.snapshot()) {
                AutoCheck::Stop => break,
                AutoCheck::Skip => {}
                AutoCheck::Run => {
                    run_check(&app, &state, false).await;
                }
            }
        }
    });
}

/// A restart is refused while a message is being handed to the SMTP server.
async fn refuse_reason(state: &DesktopState, updater: &UpdaterState) -> Option<Refusal> {
    if !updater.snapshot().may_restart() {
        return Some(Refusal::NothingReady);
    }
    let sending = state
        .run(|core| core.list_outbox(None, true))
        .await
        .map(|items| items.iter().any(|item| item.state == OutboxState::Sending))
        // An unreadable outbox is not a reason to trap the user.
        .unwrap_or(false);
    sending.then_some(Refusal::Sending)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenLinkArgs {
    url: String,
}

#[tauri::command]
pub(crate) async fn updater_get_state(
    window: WebviewWindow,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args =
        authorize(&window, "updater_get_state").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(args.map(|_| desktop(updater.snapshot()))))
}

#[tauri::command]
pub(crate) async fn updater_check(
    window: WebviewWindow,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args = authorize(&window, "updater_check").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => Ok(desktop(
            run_check(window.app_handle(), &updater, true).await,
        )),
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn updater_download(
    window: WebviewWindow,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args =
        authorize(&window, "updater_download").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(args.map(|_| {
        desktop(start_download(window.app_handle(), &updater))
    })))
}

#[tauri::command]
pub(crate) async fn updater_cancel(
    window: WebviewWindow,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args =
        authorize(&window, "updater_cancel").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(args.map(|_| {
        desktop(cancel_download(window.app_handle(), &updater))
    })))
}

/// Quits through the graceful path (composers save first); `on_exit` then
/// installs and relaunches.
#[tauri::command]
pub(crate) async fn updater_restart(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args =
        authorize(&window, "updater_restart").and_then(|_| parse::<EmptyArgs>(request.body()));
    let result = match args {
        Err(error) => Err(error),
        Ok(_) => match refuse_reason(&state, &updater).await {
            Some(Refusal::NothingReady) => Err(AppError::validation("nothing to install")),
            Some(Refusal::Sending) => Err(AppError::new(ErrorCode::AlreadySending)),
            None => {
                updater.lock().restart_after_install = true;
                crate::compose::request_quit(window.app_handle());
                Ok(desktop(updater.snapshot()))
            }
        },
    };
    Ok(respond(result))
}

/// Release-note links open in the system browser, https only.
#[tauri::command]
pub(crate) async fn updater_open_link(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<()>, ()> {
    let args =
        authorize(&window, "updater_open_link").and_then(|_| parse::<OpenLinkArgs>(request.body()));
    let result = args.and_then(|args| {
        let url = https_link(&args.url).ok_or_else(|| AppError::validation("link"))?;
        window
            .opener()
            .open_url(url.as_str(), None::<&str>)
            .map_err(|_| AppError::new(ErrorCode::Internal))
    });
    Ok(respond(result))
}

pub(crate) fn https_link(text: &str) -> Option<tauri::Url> {
    let url = tauri::Url::parse(text).ok()?;
    (url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty())
        .then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_links_open() {
        assert!(https_link("https://github.com/GuqierMcl/MailX/releases").is_some());
        for bad in [
            "http://example.com",
            "javascript:alert(1)",
            "data:text/html,x",
            "file:///etc/passwd",
            "https://user:pw@example.com",
            "mailto:a@b.c",
            "not a url",
        ] {
            assert!(https_link(bad).is_none(), "{bad}");
        }
    }
}
