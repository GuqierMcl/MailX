//! Main-window close policy. Window visibility never owns the Rust mail workers.

use std::sync::Mutex;
use std::time::Duration;

use mailx_core::{
    ApiResult, AppError, CloseBehavior, ErrorCode, SettingsSnapshot, SupportedLocale,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};
use tauri_plugin_window_state::AppHandleExt;

use crate::commands::{authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_chrome::{self, MAIN_WINDOW, Platform};

const EVENT: &str = "mailx://lifecycle";
const DESKTOP_ACTION_EVENT: &str = "mailx://desktop-action";
/// Upper bound on waiting for the main window to flush inline drafts before
/// releasing the WebView, so a stalled page never leaves the window unclosable.
const RELEASE_WAIT_LIMIT: Duration = Duration::from_secs(15);

pub(crate) struct LifecycleState(Mutex<Inner>);

struct Inner {
    policy: CloseBehavior,
    release_ui_in_background: bool,
    settings_revision: u64,
    locale: SupportedLocale,
    show_menu_bar_icon: bool,
    background_available: bool,
    background_checked: bool,
    desktop_ready: bool,
    main_ready: bool,
    pending: Option<u64>,
    resolving: bool,
    /// Generation waiting for the main window to flush inline drafts before
    /// destroying the WebView to enter background.
    releasing: Option<u64>,
    /// True while the main window has been intentionally destroyed (or skipped
    /// on a silent login start) so only the Rust core stays resident.
    main_released: bool,
    /// True during a login launch while startup preferences are still loading
    /// before deciding whether to create the main window.
    login_deferred: bool,
    /// One-shot guard set right before destroying the last window on purpose so
    /// Tauri's `ExitRequested { code: None }` does not quit the background core.
    expect_empty_exit: bool,
    pending_desktop_action: Option<serde_json::Value>,
    pending_open_request: Option<crate::notifications::OpenRequest>,
    generation: u64,
    revision: u64,
    quit_timed_out: bool,
    recovery_pending: bool,
    /// Some((silent, release_ui)) waits for the first tray-host probe before
    /// deciding whether to leave a login-launched app without a visible window.
    startup_silent: Option<(bool, bool)>,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            policy: CloseBehavior::Ask,
            release_ui_in_background: false,
            settings_revision: 0,
            locale: crate::locale::system_locale(),
            // Keep the native tray hidden until persisted settings load. This
            // avoids briefly showing it for users who explicitly turned it off.
            show_menu_bar_icon: false,
            background_available: cfg!(target_os = "macos"),
            background_checked: false,
            desktop_ready: false,
            main_ready: false,
            pending: None,
            resolving: false,
            releasing: None,
            main_released: false,
            login_deferred: false,
            expect_empty_exit: false,
            pending_desktop_action: None,
            pending_open_request: None,
            generation: 0,
            revision: 0,
            quit_timed_out: false,
            recovery_pending: false,
            startup_silent: None,
        }))
    }
}

impl LifecycleState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    platform: Platform,
    background_available: bool,
    /// This platform can show the unread number on the app icon.
    badge_available: bool,
    close_request_id: Option<u64>,
    release_request_id: Option<u64>,
    quitting: bool,
    quit_request_id: Option<u64>,
    quit_timed_out: bool,
    revision: u64,
}

fn snapshot(app: &AppHandle, state: &LifecycleState) -> Snapshot {
    let inner = state.lock();
    Snapshot {
        platform: window_chrome::current_platform(),
        background_available: inner.background_available,
        badge_available: crate::badge::SUPPORTED,
        close_request_id: inner.pending,
        release_request_id: inner.releasing,
        quitting: crate::compose::is_quitting(app),
        quit_request_id: crate::compose::quit_generation(app),
        quit_timed_out: inner.quit_timed_out,
        revision: inner.revision,
    }
}

pub(crate) fn publish(app: &AppHandle) {
    let Some(state) = app.try_state::<LifecycleState>() else {
        return;
    };
    {
        let mut inner = state.lock();
        inner.revision += 1;
        if crate::compose::is_quitting(app) {
            inner.quit_timed_out = false;
            inner.pending = None;
            inner.resolving = false;
            inner.releasing = None;
        }
    }
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        EVENT,
        snapshot(app, &state),
    );
    crate::tray::refresh(app);
}

pub(crate) fn quit_timed_out(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.lock().quit_timed_out = true;
        window_chrome::focus_main_window(app);
        publish(app);
    }
}

/// None means the user must choose, including when a remembered tray choice
/// cannot be fulfilled in the current desktop session.
fn close_action(policy: CloseBehavior, background_available: bool) -> Option<CloseBehavior> {
    match policy {
        CloseBehavior::Ask => None,
        CloseBehavior::Background if !background_available => None,
        other => Some(other),
    }
}

pub(crate) fn close_main_requested(app: &AppHandle) {
    if crate::compose::is_quitting(app) {
        return;
    }
    let Some(state) = app.try_state::<LifecycleState>() else {
        return;
    };
    let action = {
        let mut inner = state.lock();
        if inner.pending.is_some() || inner.resolving || inner.releasing.is_some() {
            return;
        }
        let action = close_action(inner.policy, inner.background_available);
        if action.is_none() {
            inner.generation += 1;
            inner.pending = Some(inner.generation);
        }
        action
    };
    match action {
        Some(CloseBehavior::Background) => {
            if enter_background(app).is_err() {
                ask_again(app);
            }
        }
        Some(CloseBehavior::Quit) => crate::compose::request_quit(app),
        _ => {
            window_chrome::focus_main_window(app);
            publish(app);
        }
    }
}

fn ask_again(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        let mut inner = state.lock();
        inner.generation += 1;
        inner.pending = Some(inner.generation);
        inner.resolving = false;
        inner.releasing = None;
        drop(inner);
        window_chrome::focus_main_window(app);
        publish(app);
    }
}

fn enter_background(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<LifecycleState>();
    let (background_available, release_ui, main_ready) = {
        let inner = state.lock();
        (
            inner.background_available,
            inner.release_ui_in_background,
            inner.main_ready,
        )
    };
    if !background_available {
        return Err(AppError::validation("backgroundUnavailable"));
    }
    if !release_ui {
        return hide_main(app);
    }
    if !main_ready {
        return destroy_main_for_background(app);
    }
    let generation = {
        let mut inner = state.lock();
        inner.generation += 1;
        inner.releasing = Some(inner.generation);
        inner.generation
    };
    publish(app);
    let timeout_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(RELEASE_WAIT_LIMIT).await;
        let timed_out = timeout_app
            .try_state::<LifecycleState>()
            .is_some_and(|state| {
                let mut inner = state.lock();
                if inner.releasing == Some(generation) {
                    inner.releasing = None;
                    true
                } else {
                    false
                }
            });
        if timed_out {
            publish(&timeout_app);
        }
    });
    Ok(())
}

fn hide_main(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<LifecycleState>();
    if !state.lock().background_available {
        return Err(AppError::validation("backgroundUnavailable"));
    }
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
    window
        .hide()
        .map_err(|_| AppError::new(ErrorCode::Internal))?;
    window_chrome::refresh_activity(app);
    Ok(())
}

fn destroy_main_for_background(app: &AppHandle) -> Result<(), AppError> {
    if crate::compose::is_quitting(app) {
        return Ok(());
    }
    let state = app.state::<LifecycleState>();
    if !state.lock().background_available {
        return Err(AppError::validation("backgroundUnavailable"));
    }
    let _ = app.save_window_state(window_chrome::WINDOW_STATE_FLAGS);
    if let Some(protocol) = app.try_state::<std::sync::Arc<crate::body_protocol::BodyProtocol>>() {
        protocol.revoke_all();
    }
    if let Some(inline) = app.try_state::<crate::compose::InlineComposerDraft>() {
        inline.clear();
    }
    let window = app.get_webview_window(MAIN_WINDOW);
    let (prev_main_ready, prev_desktop_ready) = {
        let mut inner = state.lock();
        let previous = (inner.main_ready, inner.desktop_ready);
        inner.main_ready = false;
        inner.desktop_ready = false;
        inner.releasing = None;
        inner.pending = None;
        inner.resolving = false;
        inner.main_released = true;
        previous
    };
    if let Some(window) = window
        && window.destroy().is_err()
    {
        let mut inner = state.lock();
        inner.main_ready = prev_main_ready;
        inner.desktop_ready = prev_desktop_ready;
        inner.main_released = false;
        return Err(AppError::new(ErrorCode::Internal));
    }
    window_chrome::refresh_activity(app);
    crate::tray::refresh(app);
    Ok(())
}

pub(crate) fn settings_changed(app: &AppHandle, snapshot: &SettingsSnapshot) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        let mut inner = state.lock();
        if snapshot.revision < inner.settings_revision {
            return;
        }
        inner.settings_revision = snapshot.revision;
        inner.policy = snapshot.settings.general.close_behavior;
        inner.release_ui_in_background = snapshot.settings.general.release_ui_in_background;
        inner.locale = snapshot.effective_locale;
        inner.show_menu_bar_icon = snapshot.settings.general.show_menu_bar_icon;
        drop(inner);
        crate::tray::refresh(app);
    }
}

pub(crate) fn main_ready(app: &AppHandle) -> bool {
    app.try_state::<LifecycleState>()
        .is_some_and(|state| state.lock().main_ready)
}

pub(crate) fn desktop_ready(app: &AppHandle) -> bool {
    app.try_state::<LifecycleState>()
        .is_some_and(|state| state.lock().desktop_ready)
}

pub(crate) fn can_dispatch_desktop_action(app: &AppHandle) -> bool {
    let core_ready = app
        .try_state::<DesktopState>()
        .is_some_and(|state| state.core().is_ok());
    app.try_state::<LifecycleState>().is_some_and(|state| {
        let inner = state.lock();
        inner.desktop_ready || (inner.main_released && core_ready)
    })
}

pub(crate) fn is_main_released(app: &AppHandle) -> bool {
    app.try_state::<LifecycleState>()
        .is_some_and(|state| state.lock().main_released)
}

pub(crate) fn can_recreate_main_window(app: &AppHandle) -> bool {
    app.try_state::<LifecycleState>().is_some_and(|state| {
        let inner = state.lock();
        inner.main_released || inner.login_deferred
    })
}

pub(crate) fn set_login_startup_deferred(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.lock().login_deferred = true;
    }
}

pub(crate) fn mark_main_created(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        let mut inner = state.lock();
        inner.main_released = false;
        inner.login_deferred = false;
        inner.releasing = None;
        inner.expect_empty_exit = false;
    }
}

pub(crate) fn cancel_releasing(app: &AppHandle) {
    let cancelled = app.try_state::<LifecycleState>().is_some_and(|state| {
        let mut inner = state.lock();
        inner.releasing.take().is_some()
    });
    if cancelled {
        publish(app);
    }
}

pub(crate) fn expect_window_empty_exit(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.lock().expect_empty_exit = true;
    }
}

pub(crate) fn take_expected_window_empty_exit(app: &AppHandle) -> bool {
    app.try_state::<LifecycleState>().is_some_and(|state| {
        let mut inner = state.lock();
        let expected = inner.expect_empty_exit;
        inner.expect_empty_exit = false;
        expected
    })
}

pub(crate) fn dispatch_desktop_action(app: &AppHandle, payload: serde_json::Value) {
    let immediate = app.get_webview_window(MAIN_WINDOW).is_some() && desktop_ready(app);
    window_chrome::focus_main_window(app);
    if immediate {
        let _ = app.emit_to(
            EventTarget::webview_window(MAIN_WINDOW),
            DESKTOP_ACTION_EVENT,
            payload,
        );
    } else if let Some(state) = app.try_state::<LifecycleState>() {
        state.lock().pending_desktop_action = Some(payload);
    }
}

pub(crate) fn dispatch_open_request(app: &AppHandle, request: crate::notifications::OpenRequest) {
    let immediate = app.get_webview_window(MAIN_WINDOW).is_some() && desktop_ready(app);
    if immediate {
        let _ = app.emit_to(
            EventTarget::webview_window(MAIN_WINDOW),
            crate::notifications::OPEN_EVENT,
            request,
        );
    } else if let Some(state) = app.try_state::<LifecycleState>() {
        state.lock().pending_open_request = Some(request);
    }
}

pub(crate) fn tray_options(app: &AppHandle) -> (SupportedLocale, bool, bool) {
    let core_ready = app
        .try_state::<DesktopState>()
        .is_some_and(|state| state.core().is_ok());
    let state = app.state::<LifecycleState>();
    let inner = state.lock();
    let actions_ready = inner.desktop_ready || (inner.main_released && core_ready);
    (inner.locale, inner.show_menu_bar_icon, actions_ready)
}

/// Called on the UI thread after the native tray and its host are checked.
pub(crate) fn set_background_available(app: &AppHandle, available: bool, checked: bool) {
    let state = app.state::<LifecycleState>();
    let changed = {
        let mut inner = state.lock();
        let changed = inner.background_available != available;
        if inner.background_available && !available {
            inner.recovery_pending = true;
        }
        if available {
            inner.recovery_pending = false;
        }
        inner.background_available = available;
        inner.background_checked = checked;
        changed
    };
    let recover = state.lock().recovery_pending && !crate::tray::session_locked(app);
    resolve_startup_visibility(app);
    if recover && !crate::compose::is_quitting(app) {
        if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
            if !window.is_visible().unwrap_or(true) {
                // Restore discoverability without stealing focus from another app.
                if window.show().is_ok() {
                    state.lock().recovery_pending = false;
                }
                window_chrome::refresh_activity(app);
            }
        } else if is_main_released(app) {
            state.lock().recovery_pending = false;
            window_chrome::focus_main_window(app);
        }
    }
    if !changed {
        return;
    }
    // Do not recurse through tray::refresh.
    state.lock().revision += 1;
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        EVENT,
        snapshot(app, &state),
    );
}

pub(crate) fn startup_prepared(app: &AppHandle, silent: bool, release_ui: bool) {
    app.state::<LifecycleState>().lock().startup_silent = Some((silent, release_ui));
    resolve_startup_visibility(app);
}

fn startup_visibility(silent: bool, checked: bool, available: bool) -> Option<bool> {
    if !silent {
        Some(true)
    } else if checked {
        Some(!available)
    } else {
        None
    }
}

fn resolve_startup_visibility(app: &AppHandle) {
    let state = app.state::<LifecycleState>();
    let activated = crate::autostart::user_activated(app);
    let main_exists = app.get_webview_window(MAIN_WINDOW).is_some();
    let decision = {
        let mut inner = state.lock();
        let Some((silent, release_ui)) = inner.startup_silent else {
            return;
        };
        let Some(show) =
            startup_visibility(silent, inner.background_checked, inner.background_available)
        else {
            return;
        };
        inner.startup_silent = None;
        if !show && release_ui && !activated && !main_exists {
            inner.login_deferred = false;
            inner.main_released = true;
        }
        (show, release_ui)
    };
    let (show, release_ui) = decision;
    if show || activated {
        window_chrome::focus_main_window(app);
    } else if !release_ui && app.get_webview_window(MAIN_WINDOW).is_none() {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if handle.get_webview_window(MAIN_WINDOW).is_none() {
                let _ = window_chrome::create_main_window(&handle, false);
            }
        });
    }
    window_chrome::refresh_activity(app);
    crate::tray::refresh(app);
}

pub(crate) fn setup(app: &AppHandle) {
    crate::tray::setup(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>().inner().clone();
        if let Ok(snapshot) = state.settings(|store| store.get()).await {
            settings_changed(&app, &snapshot);
        }
    });
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum CloseChoice {
    Background,
    Quit,
    Cancel,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveArgs {
    request_id: u64,
    action: CloseChoice,
    remember: bool,
}

#[tauri::command]
pub(crate) fn app_get_lifecycle(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Snapshot>, ()> {
    let result = authorize(&window, "app_get_lifecycle")
        .and_then(|_| parse::<EmptyArgs>(request.body()))
        .map(|_| {
            let state = window.state::<LifecycleState>();
            state.lock().main_ready = true;
            snapshot(window.app_handle(), &state)
        });
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn app_resolve_close(
    window: WebviewWindow,
    desktop: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args =
        authorize(&window, "app_resolve_close").and_then(|_| parse::<ResolveArgs>(request.body()));
    let result = async {
        let args = args?;
        let app = window.app_handle();
        let state = app.state::<LifecycleState>();
        {
            let mut inner = state.lock();
            if inner.pending != Some(args.request_id)
                || inner.resolving
                || crate::compose::is_quitting(app)
            {
                return Err(AppError::new(ErrorCode::Conflict));
            }
            if args.action == CloseChoice::Background && !inner.background_available {
                return Err(AppError::validation("backgroundUnavailable"));
            }
            inner.resolving = true;
        }
        let resolved = async {
            if args.remember && args.action != CloseChoice::Cancel {
                let choice = if args.action == CloseChoice::Background {
                    CloseBehavior::Background
                } else {
                    CloseBehavior::Quit
                };
                let snapshot = desktop
                    .settings(move |store| store.set_close_behavior(choice))
                    .await?;
                settings_changed(app, &snapshot);
                crate::commands::emit_settings_changed(app, &snapshot);
            }
            // A tray host can disappear or a menu quit can arrive while saving.
            if crate::compose::is_quitting(app) || state.lock().pending != Some(args.request_id) {
                return Ok(());
            }
            if args.action == CloseChoice::Background {
                enter_background(app)?;
            }
            {
                let mut inner = state.lock();
                inner.pending = None;
                inner.resolving = false;
            }
            if args.action == CloseChoice::Quit {
                crate::compose::request_quit(app);
            }
            Ok(())
        }
        .await;
        {
            let mut inner = state.lock();
            if inner.pending == Some(args.request_id) {
                inner.resolving = false;
            }
        }
        publish(app);
        resolved
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) fn app_quit(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    Ok(respond(
        authorize(&window, "app_quit")
            .and_then(|_| parse::<EmptyArgs>(request.body()))
            .map(|_| crate::compose::request_quit(window.app_handle())),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelQuitArgs {
    #[serde(default)]
    generation: Option<u64>,
}

#[tauri::command]
pub(crate) fn app_cancel_quit(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    Ok(respond(
        authorize(&window, "app_cancel_quit")
            .and_then(|_| parse::<CancelQuitArgs>(request.body()))
            .map(|args| match args.generation {
                Some(generation) => {
                    crate::compose::cancel_quit_generation(window.app_handle(), generation)
                }
                None => crate::compose::cancel_quit(window.app_handle()),
            }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmQuitArgs {
    generation: u64,
}

#[tauri::command]
pub(crate) fn app_confirm_quit(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    Ok(respond(
        authorize(&window, "app_confirm_quit")
            .and_then(|_| parse::<ConfirmQuitArgs>(request.body()))
            .and_then(|args| {
                crate::compose::confirm_main_quit(window.app_handle(), args.generation)
            }),
    ))
}

#[tauri::command]
pub(crate) fn app_confirm_background_release(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let result = authorize(&window, "app_confirm_background_release")
        .and_then(|_| parse::<ConfirmQuitArgs>(request.body()))
        .and_then(|args| {
            let app = window.app_handle();
            let state = app.state::<LifecycleState>();
            {
                let inner = state.lock();
                if inner.releasing != Some(args.generation) || crate::compose::is_quitting(app) {
                    return Err(AppError::new(ErrorCode::Conflict));
                }
            }
            destroy_main_for_background(app)
        });
    Ok(respond(result))
}

#[tauri::command]
pub(crate) fn app_cancel_background_release(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let result = authorize(&window, "app_cancel_background_release")
        .and_then(|_| parse::<CancelQuitArgs>(request.body()))
        .map(|args| {
            let app = window.app_handle();
            let cancelled = {
                let state = app.state::<LifecycleState>();
                let mut inner = state.lock();
                let matches = match args.generation {
                    Some(generation) => inner.releasing == Some(generation),
                    None => inner.releasing.is_some(),
                };
                if matches {
                    inner.releasing = None;
                    true
                } else {
                    false
                }
            };
            if cancelled {
                publish(app);
            }
        });
    Ok(respond(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyArgs {
    ready: bool,
}

#[tauri::command]
pub(crate) fn app_desktop_ready(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    Ok(respond(
        authorize(&window, "app_desktop_ready")
            .and_then(|_| parse::<ReadyArgs>(request.body()))
            .map(|args| {
                let app = window.app_handle();
                let (pending_action, pending_open) = {
                    let state = app.state::<LifecycleState>();
                    let mut inner = state.lock();
                    inner.desktop_ready = args.ready;
                    if args.ready {
                        (
                            inner.pending_desktop_action.take(),
                            inner.pending_open_request.take(),
                        )
                    } else {
                        (None, None)
                    }
                };
                crate::tray::refresh(app);
                if let Some(action) = pending_action {
                    let _ = app.emit_to(
                        EventTarget::webview_window(MAIN_WINDOW),
                        DESKTOP_ACTION_EVENT,
                        action,
                    );
                }
                if let Some(open) = pending_open {
                    let _ = app.emit_to(
                        EventTarget::webview_window(MAIN_WINDOW),
                        crate::notifications::OPEN_EVENT,
                        open,
                    );
                }
            }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::InvokeBody;

    #[test]
    fn silent_login_startup_waits_for_a_probe_and_falls_back_to_a_visible_window() {
        assert_eq!(startup_visibility(false, false, false), Some(true));
        assert_eq!(startup_visibility(true, false, false), None);
        assert_eq!(startup_visibility(true, true, false), Some(true));
        assert_eq!(startup_visibility(true, true, true), Some(false));
    }

    #[test]
    fn remembered_background_requires_a_recovery_entry_but_quit_does_not() {
        for available in [false, true] {
            assert_eq!(close_action(CloseBehavior::Ask, available), None);
            assert_eq!(
                close_action(CloseBehavior::Quit, available),
                Some(CloseBehavior::Quit)
            );
        }
        assert_eq!(close_action(CloseBehavior::Background, false), None);
        assert_eq!(
            close_action(CloseBehavior::Background, true),
            Some(CloseBehavior::Background)
        );
    }

    #[test]
    fn close_choice_is_explicit_and_rejects_extra_fields() {
        let body = |value| InvokeBody::Json(value);
        assert!(
            parse::<ResolveArgs>(&body(
                serde_json::json!({"requestId":1,"action":"cancel","remember":true})
            ))
            .is_ok()
        );
        for value in [
            serde_json::json!({"requestId":1,"action":"quit"}),
            serde_json::json!({"requestId":1,"action":"hide","remember":true}),
            serde_json::json!({"requestId":1,"action":"quit","remember":false,"caller":"main"}),
        ] {
            assert!(parse::<ResolveArgs>(&body(value)).is_err());
        }
    }

    #[test]
    fn expected_empty_exit_is_consumed_once() {
        let state = LifecycleState::default();
        assert!(!state.lock().expect_empty_exit);
        state.lock().expect_empty_exit = true;
        let first = {
            let mut inner = state.lock();
            let expected = inner.expect_empty_exit;
            inner.expect_empty_exit = false;
            expected
        };
        let second = state.lock().expect_empty_exit;
        assert!(first);
        assert!(!second);
    }
}
