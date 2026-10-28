//! Main-window close policy. Window visibility never owns the Rust mail workers.

use std::sync::Mutex;

use mailx_core::{
    ApiResult, AppError, CloseBehavior, ErrorCode, SettingsSnapshot, SupportedLocale,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};

use crate::commands::{authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_chrome::{self, MAIN_WINDOW, Platform};

const EVENT: &str = "mailx://lifecycle";

pub(crate) struct LifecycleState(Mutex<Inner>);

struct Inner {
    policy: CloseBehavior,
    settings_revision: u64,
    locale: SupportedLocale,
    show_menu_bar_icon: bool,
    background_available: bool,
    desktop_ready: bool,
    main_ready: bool,
    pending: Option<u64>,
    resolving: bool,
    generation: u64,
    revision: u64,
    quit_timed_out: bool,
    recovery_pending: bool,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            policy: CloseBehavior::Ask,
            settings_revision: 0,
            locale: crate::locale::system_locale(),
            show_menu_bar_icon: false,
            background_available: cfg!(target_os = "macos"),
            desktop_ready: false,
            main_ready: false,
            pending: None,
            resolving: false,
            generation: 0,
            revision: 0,
            quit_timed_out: false,
            recovery_pending: false,
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
    close_request_id: Option<u64>,
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
        close_request_id: inner.pending,
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
        if inner.pending.is_some() || inner.resolving {
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
            if hide_main(app).is_err() {
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
        drop(inner);
        window_chrome::focus_main_window(app);
        publish(app);
    }
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

pub(crate) fn settings_changed(app: &AppHandle, snapshot: &SettingsSnapshot) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        let mut inner = state.lock();
        if snapshot.revision < inner.settings_revision {
            return;
        }
        inner.settings_revision = snapshot.revision;
        inner.policy = snapshot.settings.general.close_behavior;
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

pub(crate) fn tray_options(app: &AppHandle) -> (SupportedLocale, bool, bool) {
    let state = app.state::<LifecycleState>();
    let inner = state.lock();
    (inner.locale, inner.show_menu_bar_icon, inner.desktop_ready)
}

/// Called on the UI thread after the native tray and its host are checked.
pub(crate) fn set_background_available(app: &AppHandle, available: bool) {
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
        changed
    };
    let recover = state.lock().recovery_pending && !crate::tray::session_locked(app);
    if recover
        && !crate::compose::is_quitting(app)
        && let Some(window) = app.get_webview_window(MAIN_WINDOW)
        && !window.is_visible().unwrap_or(true)
    {
        // Restore discoverability without stealing focus from another app.
        if window.show().is_ok() {
            state.lock().recovery_pending = false;
        }
        window_chrome::refresh_activity(app);
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
                hide_main(app)?;
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
                window.state::<LifecycleState>().lock().desktop_ready = args.ready;
                crate::tray::refresh(window.app_handle());
            }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::InvokeBody;

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
}
