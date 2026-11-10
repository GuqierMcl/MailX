//! Desktop update preparation, notification and graceful quit-time installation.
//! The host owns scheduling even when the main window is hidden. Only complete,
//! signature-verified packages are persisted; a fresh manifest gates cache reuse.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use mailx_core::{AppError, ErrorCode, OutboxState, SettingsSnapshot};
use serde::Deserialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{EmptyArgs, authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_chrome::MAIN_WINDOW;

mod cache;
mod policy;

use cache::Cache;
use policy::{
    AutomaticDownload, CHECK_TIMEOUT, DOWNLOAD_TIMEOUT, FailureStage, ReleaseDetails, Snapshot,
    UpdateState, cap_notes, is_newer, next_wait, progress_due,
};

pub(crate) const STATE_EVENT: &str = "mailx://updater-state";

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
    cache: Option<Cache>,
    download: Option<JoinHandle<()>>,
    generation: u64,
    checking: bool,
    pending_manual: bool,
    download_manual: bool,
    install_on_quit: bool,
    restart_after_install: bool,
    scheduler_started: bool,
    settings_revision: Option<u64>,
    install_failure: Option<String>,
    automatic: AutomaticDownload,
}

impl Inner {
    fn state(&self) -> UpdateState {
        self.state.clone().unwrap_or(UpdateState::Idle)
    }

    fn apply_settings(&mut self, revision: u64, enabled: bool) -> bool {
        if self
            .settings_revision
            .is_some_and(|current| current >= revision)
        {
            return false;
        }
        self.settings_revision = Some(revision);
        let enabling = enabled && !self.automatic.enabled;
        self.automatic.set_enabled(enabled);
        if enabling && !self.state().may_cancel() && !self.state().may_restart() {
            self.download_manual = false;
        }
        if !enabled && !self.download_manual {
            self.cancel_download(false);
            self.install_on_quit = false;
        } else if enabled
            && self.state().may_restart()
            && self
                .update
                .as_ref()
                .is_some_and(|update| !self.automatic.is_paused(&update.version))
        {
            self.install_on_quit = true;
        }
        true
    }

    fn begin_download(&mut self, manual: bool, now: Instant) -> Option<(Update, u64)> {
        if manual {
            self.pending_manual = true;
        }
        if self.checking {
            if manual {
                self.state = Some(UpdateState::Checking);
            }
            return None;
        }
        if manual && self.state().may_cancel() {
            self.download_manual = true;
            self.pending_manual = false;
            return None;
        }
        let update = self
            .update
            .clone()
            .filter(|_| self.state().may_download() || (manual && self.state().may_restart()))?;
        if !manual
            && (!self.automatic.may_start(&update.version, now)
                || self.download_manual
                || self.pending_manual
                || self.install_failure.as_deref() == Some(update.version.as_str())
                || matches!(
                    self.state(),
                    UpdateState::Failed {
                        stage: FailureStage::Install,
                        ..
                    }
                ))
        {
            return None;
        }
        if manual {
            self.automatic.resume(&update.version);
            self.pending_manual = false;
        }
        self.generation += 1;
        self.download_manual = manual;
        self.install_on_quit = false;
        self.state = Some(UpdateState::Downloading {
            version: update.version.clone(),
            details: release_details(&update),
            downloaded: 0,
            total: None,
        });
        Some((update, self.generation))
    }

    fn retry_deadline(&self) -> Option<Instant> {
        if !self.checking
            && self.automatic.enabled
            && !self.download_manual
            && matches!(
                self.state(),
                UpdateState::Failed {
                    stage: FailureStage::Download,
                    ..
                }
            )
        {
            self.automatic.retry_at
        } else {
            None
        }
    }

    fn arm_restart(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.state().may_restart() {
            return false;
        }
        self.restart_after_install = true;
        true
    }

    fn cancel_download(&mut self, pause: bool) {
        if !self.state().may_cancel() {
            return;
        }
        self.generation += 1;
        if let Some(task) = self.download.take() {
            task.abort();
        }
        self.state = Some(match &self.update {
            Some(update) => {
                if pause {
                    self.automatic.pause(&update.version);
                }
                UpdateState::Available {
                    version: update.version.clone(),
                    details: release_details(update),
                }
            }
            None => UpdateState::Idle,
        });
        self.download_manual = false;
        self.pending_manual = false;
        self.install_on_quit = false;
    }
}

#[derive(Clone, Default)]
pub(crate) struct UpdaterState(
    Arc<Mutex<Inner>>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::OnceCell<()>>,
);

impl UpdaterState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn snapshot(&self) -> UpdateState {
        self.lock().state()
    }

    fn report(&self, app: &AppHandle) -> Snapshot {
        snapshot(app, &self.lock())
    }
}

fn can_install(app: &AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        app.env().appimage.is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        cfg!(any(target_os = "macos", target_os = "windows"))
    }
}

fn snapshot(app: &AppHandle, inner: &Inner) -> Snapshot {
    Snapshot {
        can_install: can_install(app),
        install_on_quit: inner.install_on_quit && inner.state().may_restart(),
        state: inner.state(),
    }
}

/// Publish while holding the state lock so progress and cancellation cannot
/// deliver obsolete state after a newer transition.
fn emit(app: &AppHandle, state: &UpdaterState) {
    let inner = state.lock();
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        STATE_EVENT,
        snapshot(app, &inner),
    );
}

fn release_details(update: &Update) -> ReleaseDetails {
    ReleaseDetails {
        current: update.current_version.clone(),
        notes: cap_notes(update.body.clone()),
        date: update.date.map(|date| date.unix_timestamp()),
    }
}

/// Called after the atomic settings write. Revisions prevent an older async
/// settings read from undoing a switch the user has just changed.
pub(crate) fn settings_changed(app: &AppHandle, settings: &SettingsSnapshot) {
    let Some(state) = app.try_state::<UpdaterState>() else {
        return;
    };
    if !state
        .lock()
        .apply_settings(settings.revision, settings.settings.general.auto_update)
    {
        return;
    }
    emit(app, &state);
    state.1.notify_one();
}

async fn initialize(app: &AppHandle, state: &UpdaterState) {
    state
        .2
        .get_or_init(|| async {
            let cache = state.lock().cache.clone();
            let current = app.package_info().version.to_string();
            let failure = tauri::async_runtime::spawn_blocking(move || {
                cache.and_then(|cache| cache.failure(&current).ok().flatten())
            })
            .await
            .ok()
            .flatten();
            if let Some(failure) = failure {
                let mut inner = state.lock();
                inner.install_failure = Some(failure.version.clone());
                inner.state = Some(UpdateState::Failed {
                    stage: FailureStage::Install,
                    version: Some(failure.version),
                    details: Some(failure.details),
                });
            }
        })
        .await;
}

async fn refresh_settings(app: &AppHandle) {
    if let Ok(settings) = app
        .state::<DesktopState>()
        .settings(|store| store.get())
        .await
    {
        settings_changed(app, &settings);
    }
}

fn same_update(a: &Update, b: &Update) -> bool {
    a.version == b.version && a.download_url == b.download_url && a.signature == b.signature
}

async fn run_check(app: &AppHandle, state: &UpdaterState, manual: bool) {
    initialize(app, state).await;
    refresh_settings(app).await;
    let (generation, previous) = {
        let mut inner = state.lock();
        if inner.checking || !inner.state().may_check() || crate::compose::is_quitting(app) {
            return;
        }
        // A manual check of a prepared update opens its details immediately.
        if manual && inner.state().may_restart() {
            return;
        }
        inner.checking = true;
        let previous = inner.state();
        if manual
            || matches!(
                previous,
                UpdateState::Idle
                    | UpdateState::Failed {
                        stage: FailureStage::Check,
                        ..
                    }
            )
        {
            inner.state = Some(UpdateState::Checking);
        }
        (inner.generation, previous)
    };
    emit(app, state);
    let checked = match app.updater_builder().timeout(CHECK_TIMEOUT).build() {
        Ok(updater) => updater.check().await.map_err(|_| ()),
        Err(_) => Err(()),
    };
    let cached = if can_install(app) {
        match &checked {
            Ok(Some(update)) => {
                let cache = state.lock().cache.clone();
                let update = update.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    cache.and_then(|cache| cache.load(&update).ok().flatten())
                })
                .await
                .ok()
                .flatten()
            }
            _ => None,
        }
    } else {
        None
    };
    {
        let mut inner = state.lock();
        inner.checking = false;
        if inner.generation != generation {
            return;
        }
        if inner.restart_after_install || crate::compose::is_quitting(app) {
            if matches!(inner.state(), UpdateState::Checking) {
                inner.state = Some(previous);
            }
            return;
        }
        match checked {
            Ok(Some(mut update)) if is_newer(&update.current_version, &update.version) => {
                update.timeout = Some(DOWNLOAD_TIMEOUT);
                let same = inner
                    .update
                    .as_ref()
                    .is_some_and(|old| same_update(old, &update));
                let install_failed =
                    inner.install_failure.as_deref() == Some(update.version.as_str());
                if !same {
                    inner.generation += 1;
                    inner.automatic.reset_attempts();
                    inner.package = None;
                    inner.download_manual = false;
                    inner.install_on_quit = false;
                }
                let details = release_details(&update);
                inner.state = Some(if install_failed {
                    UpdateState::Failed {
                        stage: FailureStage::Install,
                        version: Some(update.version.clone()),
                        details: Some(details),
                    }
                } else if same && inner.package.is_some() && previous.may_restart() {
                    previous
                } else if inner.automatic.is_paused(&update.version) {
                    UpdateState::Available {
                        version: update.version.clone(),
                        details,
                    }
                } else if let Some(cached) = cached {
                    inner.package = Some(Arc::new(cached.bytes));
                    inner.download_manual = cached.manual;
                    inner.install_on_quit = cached.manual || inner.automatic.enabled;
                    UpdateState::Ready {
                        version: update.version.clone(),
                        details,
                    }
                } else if same
                    && matches!(
                        previous,
                        UpdateState::Failed {
                            stage: FailureStage::Download,
                            ..
                        }
                    )
                {
                    previous
                } else {
                    UpdateState::Available {
                        version: update.version.clone(),
                        details,
                    }
                });
                inner.update = Some(update);
            }
            Ok(_) => {
                // A withdrawn release must not remain armed for quit-time install.
                inner.generation += 1;
                inner.update = None;
                inner.pending_manual = false;
                inner.install_failure = None;
                inner.package = None;
                inner.install_on_quit = false;
                inner.automatic.reset_attempts();
                inner.state = Some(UpdateState::Idle);
                if let Some(cache) = &inner.cache {
                    let _ = cache.clear();
                }
            }
            Err(()) => {
                let explicit = manual || inner.pending_manual;
                inner.pending_manual = false;
                inner.state = Some(if explicit {
                    inner.install_on_quit = false;
                    UpdateState::Failed {
                        stage: FailureStage::Check,
                        version: None,
                        details: None,
                    }
                } else if matches!(
                    previous,
                    UpdateState::Idle
                        | UpdateState::Failed {
                            stage: FailureStage::Check,
                            ..
                        }
                ) {
                    UpdateState::Idle
                } else {
                    previous
                });
            }
        }
    }
    // Do not emit Available just before an automatic download: that would
    // produce an unnecessary "new version" notification.
    maybe_download(app, state);
    emit(app, state);
    state.1.notify_one();
}

fn retryable(error: &tauri_plugin_updater::Error) -> bool {
    matches!(
        error,
        tauri_plugin_updater::Error::Reqwest(_) | tauri_plugin_updater::Error::Network(_)
    )
}

async fn run_download(app: AppHandle, state: UpdaterState, update: Update, generation: u64) {
    let version = update.version.clone();
    let details = release_details(&update);
    let cached = {
        let state = state.clone();
        let update = update.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let inner = state.lock();
            if inner.generation != generation {
                return None;
            }
            inner
                .cache
                .as_ref()
                .and_then(|cache| cache.load(&update).ok().flatten())
        })
        .await
        .ok()
        .flatten()
    };
    if state.lock().generation != generation {
        return;
    }
    let result = match cached {
        Some(cached) => Ok(cached.bytes),
        None => {
            let progress = {
                let app = app.clone();
                let state = state.clone();
                let version = version.clone();
                let details = details.clone();
                let mut downloaded = 0_u64;
                let mut last_emit = None;
                let mut last_percent = None;
                move |chunk: usize, total: Option<u64>| {
                    downloaded += chunk as u64;
                    let percent = total
                        .filter(|total| *total > 0)
                        .map(|total| downloaded.saturating_mul(100) / total);
                    let now = Instant::now();
                    if !progress_due(last_emit, now, last_percent, percent) {
                        return;
                    }
                    last_emit = Some(now);
                    last_percent = percent;
                    {
                        let mut inner = state.lock();
                        if inner.generation != generation {
                            return;
                        }
                        inner.state = Some(UpdateState::Downloading {
                            version: version.clone(),
                            details: details.clone(),
                            downloaded,
                            total,
                        });
                    }
                    emit(&app, &state);
                }
            };
            update
                .download(progress, || {})
                .await
                .map_err(|error| retryable(&error))
        }
    };
    // Persist and publish as a single guarded completion. Cancellation cannot
    // leave a late disk commit that gets restored as an accepted update.
    let task_state = state.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let mut inner = task_state.lock();
        if inner.generation != generation {
            return;
        }
        let manual = inner.download_manual;
        let result = result.and_then(|bytes| {
            let cache = inner.cache.as_ref().ok_or(false)?;
            cache.store(&update, &bytes, manual).map_err(|_| false)?;
            cache.clear_failure().map_err(|_| false)?;
            Ok(bytes)
        });
        inner.download = None;
        inner.state = Some(match result {
            Ok(bytes) => {
                inner.install_failure = None;
                inner.package = Some(Arc::new(bytes));
                inner.install_on_quit = manual || inner.automatic.enabled;
                inner.automatic.reset_attempts();
                UpdateState::Ready { version, details }
            }
            Err(retryable) => {
                inner.install_on_quit = false;
                if !manual {
                    inner.automatic.failed(Instant::now(), retryable);
                }
                UpdateState::Failed {
                    stage: FailureStage::Download,
                    version: Some(version),
                    details: Some(details),
                }
            }
        });
    })
    .await;
    emit(&app, &state);
    state.1.notify_one();
}

/// Returns true when a download was started (or a prepared package was armed).
fn start_download(app: &AppHandle, state: &UpdaterState, manual: bool) -> bool {
    if !can_install(app) || crate::compose::is_quitting(app) {
        return false;
    }
    let mut inner = state.lock();
    let Some((update, generation)) = inner.begin_download(manual, Instant::now()) else {
        return false;
    };
    inner.download = Some(tauri::async_runtime::spawn(run_download(
        app.clone(),
        state.clone(),
        update,
        generation,
    )));
    true
}

fn maybe_download(app: &AppHandle, state: &UpdaterState) {
    if state.lock().pending_manual {
        start_download(app, state, true);
        return;
    }
    let allowed = {
        let inner = state.lock();
        inner.update.as_ref().is_some_and(|update| {
            inner.automatic.may_start(&update.version, Instant::now())
                && matches!(
                    inner.state(),
                    UpdateState::Available { .. }
                        | UpdateState::Failed {
                            stage: FailureStage::Download,
                            ..
                        }
                )
                && !inner.download_manual
        })
    };
    if allowed {
        start_download(app, state, false);
    }
}

fn cancel_download(app: &AppHandle, state: &UpdaterState) {
    state.lock().cancel_download(true);
    emit(app, state);
    state.1.notify_one();
}

fn install_downloaded(state: &UpdaterState, relaunch: bool) -> bool {
    let (update, package, cache) = {
        let inner = state.lock();
        if !inner.state().may_restart() || !(relaunch || inner.install_on_quit) {
            return false;
        }
        let (Some(update), Some(package), Some(cache)) =
            (&inner.update, &inner.package, &inner.cache)
        else {
            return false;
        };
        (update.clone(), package.clone(), cache.clone())
    };
    // Windows may exit inside install and invoke cleanup callbacks. Do not hold
    // the updater lock across the plugin, and record the attempt beforehand.
    if cache
        .mark_install_attempt(&update.version, &release_details(&update))
        .is_err()
    {
        return false;
    }
    update
        .restart_after_install(relaunch)
        .install(package.as_slice())
        .is_ok()
}

pub(crate) fn on_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<UpdaterState>() else {
        return;
    };
    let restart = state.lock().restart_after_install;
    if can_install(app) && install_downloaded(&state, restart) && restart {
        tauri::process::restart(&app.env());
    }
}

pub(crate) fn cancel_restart(app: &AppHandle) {
    if let Some(state) = app.try_state::<UpdaterState>() {
        state.lock().restart_after_install = false;
        state.1.notify_one();
    }
}

pub(crate) fn start_scheduler(app: &AppHandle) {
    let state = app.state::<UpdaterState>().inner().clone();
    {
        let mut inner = state.lock();
        if inner.scheduler_started {
            return;
        }
        inner.scheduler_started = true;
        inner.cache = app.state::<DesktopState>().data_dir().ok().map(Cache::new);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        refresh_settings(&app).await;
        initialize(&app, &state).await;
        emit(&app, &state);
        let mut next_check = Instant::now() + next_wait(true);
        loop {
            let retry_at = if crate::compose::is_quitting(&app) {
                None
            } else {
                state.lock().retry_deadline()
            };
            let wake_at = retry_at.map_or(next_check, |retry| retry.min(next_check));
            tokio::select! {
                _ = tokio::time::sleep_until(wake_at.into()) => {},
                _ = state.1.notified() => {},
            }
            if Instant::now() >= next_check {
                run_check(&app, &state, false).await;
                next_check = Instant::now() + next_wait(false);
            } else {
                maybe_download(&app, &state);
                emit(&app, &state);
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
    Ok(respond(args.map(|_| updater.report(window.app_handle()))))
}

#[tauri::command]
pub(crate) async fn updater_check(
    window: WebviewWindow,
    updater: State<'_, UpdaterState>,
    request: tauri::ipc::Request<'_>,
) -> Result<mailx_core::ApiResult<Snapshot>, ()> {
    let args = authorize(&window, "updater_check").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => {
            run_check(window.app_handle(), &updater, true).await;
            Ok(updater.report(window.app_handle()))
        }
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
    Ok(respond(match args {
        Err(error) => Err(error),
        Ok(_) if !can_install(window.app_handle()) => Err(AppError::new(ErrorCode::Unsupported)),
        Ok(_) => {
            start_download(window.app_handle(), &updater, true);
            let needs_check = {
                let inner = updater.lock();
                inner.pending_manual && !inner.checking
            };
            if needs_check {
                run_check(window.app_handle(), &updater, true).await;
            }
            emit(window.app_handle(), &updater);
            Ok(updater.report(window.app_handle()))
        }
    }))
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
        cancel_download(window.app_handle(), &updater);
        updater.report(window.app_handle())
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
    let generation = updater.lock().generation;
    let result = match args {
        Err(error) => Err(error),
        Ok(_) if !can_install(window.app_handle()) => Err(AppError::new(ErrorCode::Unsupported)),
        Ok(_) => match refuse_reason(&state, &updater).await {
            Some(Refusal::NothingReady) => Err(AppError::validation("nothing to install")),
            Some(Refusal::Sending) => Err(AppError::new(ErrorCode::AlreadySending)),
            None => {
                if !updater.lock().arm_restart(generation) {
                    return Ok(respond(Err(AppError::validation("update changed"))));
                }
                crate::compose::request_quit(window.app_handle());
                Ok(updater.report(window.app_handle()))
            }
        },
    };
    Ok(respond(result))
}

/// App and release-note links open in the system browser, https only.
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
mod tests;
