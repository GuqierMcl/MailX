//! Composer windows and the sending commands.
//!
//! One window per draft, labelled `composer-{draftId}`, loading the same
//! frontend bundle as the main window with `?composer={draftId}`. Which
//! commands each window may call is decided in `window_access`; the principal
//! given to the core comes from the window kind, never from the request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mailx_core::{
    AppError, CreateDraftFromRequest, CreateDraftRequest, Draft, DraftChanged, DraftSummary,
    ErrorCode, OutboxChanged, OutboxItem, QueueSendRequest, ResolveUnknownRequest,
    UnknownResolution, UpdateDraftRequest,
};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, EventTarget, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use crate::commands::{authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_access::{self, WindowKind, composer_label, parse_draft_id};
use crate::window_chrome::{self, MAIN_WINDOW};
use mailx_core::ApiResult;

/// Pushed to the main window and the affected composer on every queue change.
pub(crate) const OUTBOX_EVENT: &str = "mailx://outbox";
/// Pushed to the main window and the affected composer when the queue locks,
/// releases or deletes a draft.
pub(crate) const DRAFTS_EVENT: &str = "mailx://drafts";
/// Pushed to every composer when the main window is closing: flush the draft,
/// then call `composer_close`.
const APP_CLOSING_EVENT: &str = "mailx://app-closing";
/// Pushed to a composer when the system asked to close it (red button, Alt+F4,
/// the drawn close button): flush the draft, then call `composer_close`.
const CLOSE_REQUESTED_EVENT: &str = "mailx://composer-close-requested";

/// How long the frontend gets to flush before the host closes anyway.
pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(3);
const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 680.0;
const MIN_WIDTH: f64 = 520.0;
const MIN_HEIGHT: f64 = 420.0;
/// Offset of each further composer from the main window, in logical pixels.
const CASCADE_STEP: f64 = 32.0;
/// Composers cascade over this many positions, then start again.
const CASCADE_SLOTS: usize = 8;
const NEW_MAIL_TITLE: &str = "新邮件";

/// Close bookkeeping shared by all windows.
#[derive(Default)]
pub(crate) struct Lifecycle {
    /// The main window is closing and the app will exit.
    quitting: AtomicBool,
    /// Every composer is gone and `exit` has been called; nothing holds back
    /// the close that follows.
    exiting: AtomicBool,
    /// Composers with an outstanding close request, by generation so a timer
    /// never closes a window that reused the label.
    pending: Mutex<HashMap<String, u64>>,
    generation: AtomicU64,
}

impl Lifecycle {
    /// Records a close request; `None` when one is already outstanding.
    fn begin_close(&self, label: &str) -> Option<u64> {
        let mut pending = self.pending.lock().ok()?;
        if pending.contains_key(label) {
            return None;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        pending.insert(label.to_owned(), generation);
        Some(generation)
    }

    fn is_pending(&self, label: &str, generation: u64) -> bool {
        self.pending
            .lock()
            .is_ok_and(|pending| pending.get(label) == Some(&generation))
    }

    fn finish(&self, label: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(label);
        }
    }
}

// ---------------------------------------------------------------- events

/// Windows that should hear a change: always main, plus the composer of the
/// affected draft.
pub(crate) fn event_labels(draft_id: Option<&str>) -> Vec<String> {
    let mut labels = vec![MAIN_WINDOW.to_owned()];
    if let Some(id) = draft_id.and_then(parse_draft_id) {
        labels.push(composer_label(&id));
    }
    labels
}

fn emit_to_labels<S: Serialize + Clone>(
    app: &AppHandle,
    event: &str,
    draft_id: Option<&str>,
    payload: &S,
) {
    for label in event_labels(draft_id) {
        let _ = app.emit_to(EventTarget::webview_window(label), event, payload.clone());
    }
}

pub(crate) fn emit_outbox(app: &AppHandle, change: &OutboxChanged) {
    emit_to_labels(app, OUTBOX_EVENT, change.draft_id.as_deref(), change);
}

pub(crate) fn emit_draft(app: &AppHandle, change: &DraftChanged) {
    emit_to_labels(app, DRAFTS_EVENT, Some(&change.draft_id), change);
}

// --------------------------------------------------------------- commands

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestArgs<T> {
    request: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftListArgs {
    account_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutboxListArgs {
    account_id: Option<String>,
    #[serde(default)]
    include_all: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveArgs {
    id: String,
    choice: UnknownResolution,
    client_mutation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftArgs {
    draft_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseArgs {
    /// Accepted for the contract; the host closes either way, because the
    /// frontend has already flushed (or given up) before calling.
    #[serde(rename = "force")]
    _force: bool,
}

#[tauri::command]
pub(crate) async fn drafts_create(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let args = authorize(&window, "drafts_create")
        .and_then(|_| parse::<RequestArgs<CreateDraftRequest>>(request.body()));
    let result = async {
        let args = args?;
        let default_account = state
            .settings(|store| store.get())
            .await?
            .settings
            .compose
            .default_account_id;
        state
            .run(move |core| core.create_draft(args.request, &default_account))
            .await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn drafts_create_from(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let args = authorize(&window, "drafts_create_from")
        .and_then(|_| parse::<RequestArgs<CreateDraftFromRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.create_draft_from(args.request))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn drafts_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<DraftSummary>>, ()> {
    let args =
        authorize(&window, "drafts_list").and_then(|_| parse::<DraftListArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.list_drafts(args.account_id.as_deref()))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn drafts_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let args = authorize(&window, "drafts_get").and_then(|kind| {
        let args = parse::<IdArgs>(request.body())?;
        window_access::check_draft(kind, "drafts_get", &args.id)?;
        Ok(args)
    });
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.get_draft(&args.id)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn drafts_update(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let args = authorize(&window, "drafts_update").and_then(|kind| {
        let args = parse::<RequestArgs<UpdateDraftRequest>>(request.body())?;
        window_access::check_draft(kind, "drafts_update", &args.request.id)?;
        Ok(args)
    });
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.update_draft(args.request)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn drafts_delete(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "drafts_delete").and_then(|kind| {
        let args = parse::<IdArgs>(request.body())?;
        window_access::check_draft(kind, "drafts_delete", &args.id)?;
        Ok(args)
    });
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.delete_draft(&args.id)).await,
        Err(error) => Err(error),
    }))
}

/// Queues the composer's own draft. The two-step button is only UX: the host
/// checks the window kind and draft here, and the core checks the principal.
#[tauri::command]
pub(crate) async fn outbox_queue_send(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<OutboxItem>, ()> {
    let args = authorize(&window, "outbox_queue_send").and_then(|kind| {
        let args = parse::<QueueSendRequest>(request.body())?;
        window_access::check_draft(kind, "outbox_queue_send", &args.draft_id)?;
        Ok((kind.caller(), args))
    });
    let result = async {
        let (caller, args) = args?;
        let undo = state
            .settings(|store| store.get())
            .await?
            .settings
            .compose
            .undo_send_seconds;
        state
            .run(move |core| core.queue_send(caller, args, undo))
            .await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn outbox_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<OutboxItem>>, ()> {
    let args =
        authorize(&window, "outbox_list").and_then(|_| parse::<OutboxListArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.list_outbox(args.account_id.as_deref(), args.include_all))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn outbox_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<OutboxItem>, ()> {
    let args = authorize(&window, "outbox_get").and_then(|_| parse::<IdArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.get_outbox(&args.id)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn outbox_cancel(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<OutboxItem>, ()> {
    let args = authorize(&window, "outbox_cancel").and_then(|_| parse::<IdArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.cancel_outbox(&args.id)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn outbox_retry_now(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<OutboxItem>, ()> {
    let args = authorize(&window, "outbox_retry_now").and_then(|_| parse::<IdArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.retry_outbox_now(&args.id)).await,
        Err(error) => Err(error),
    }))
}

/// Settles an `unknown` item. Allowed from the main window (the outbox view)
/// as interactive UI; `resend` is the user's explicit decision there.
#[tauri::command]
pub(crate) async fn outbox_resolve_unknown(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<OutboxItem>, ()> {
    let args = authorize(&window, "outbox_resolve_unknown")
        .and_then(|kind| parse::<ResolveArgs>(request.body()).map(|args| (kind.caller(), args)));
    Ok(respond(match args {
        Ok((caller, args)) => {
            state
                .run(move |core| {
                    core.resolve_unknown(
                        caller,
                        ResolveUnknownRequest {
                            id: args.id,
                            resolution: args.choice,
                            client_mutation_id: args.client_mutation_id,
                        },
                    )
                })
                .await
        }
        Err(error) => Err(error),
    }))
}

/// Opens the composer window of an existing draft, or focuses it. Async on
/// purpose: building a window from a synchronous command can deadlock on
/// Windows.
#[tauri::command]
pub(crate) async fn composer_open(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "composer_open").and_then(|_| {
        let args = parse::<DraftArgs>(request.body())?;
        parse_draft_id(&args.draft_id).ok_or_else(|| AppError::validation("draftId"))
    });
    let result = async {
        let id = args?;
        let text = id.hyphenated().to_string();
        // The draft must exist; a window for nothing is never created.
        state
            .run(move |core| core.get_draft(&text).map(|_| ()))
            .await?;
        open_composer(&app, &id).map_err(|_| AppError::new(ErrorCode::Internal))
    }
    .await;
    Ok(respond(result))
}

/// Closes the calling composer window. The frontend has already flushed the
/// draft; `destroy` skips the close-request interception, which would
/// otherwise ask the page again.
#[tauri::command]
pub(crate) async fn composer_close(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args =
        authorize(&window, "composer_close").and_then(|_| parse::<CloseArgs>(request.body()));
    Ok(respond(args.and_then(|_| {
        window
            .destroy()
            .map_err(|_| AppError::new(ErrorCode::Internal))
    })))
}

// ---------------------------------------------------------------- windows

/// Labels of the composer windows that are currently open.
fn composer_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| matches!(WindowKind::parse(label), Some(WindowKind::Composer(_))))
        .map(|(_, window)| window)
        .collect()
}

/// Top-left of the next composer in physical pixels: offset from the main
/// window by one cascade step per slot, kept inside the monitor when it fits.
/// `monitor` is `(position, size)`; `size` is the composer's physical size.
pub(crate) fn cascade_position(
    main: (i32, i32),
    step: i32,
    slot: usize,
    size: (i32, i32),
    monitor: Option<((i32, i32), (i32, i32))>,
) -> (i32, i32) {
    let offset = step.saturating_mul(i32::try_from(slot % CASCADE_SLOTS).unwrap_or(0) + 1);
    let mut x = main.0.saturating_add(offset);
    let mut y = main.1.saturating_add(offset);
    if let Some((origin, extent)) = monitor {
        // `max` last: a window larger than the monitor starts at its corner.
        x = x
            .min(origin.0.saturating_add(extent.0).saturating_sub(size.0))
            .max(origin.0);
        y = y
            .min(origin.1.saturating_add(extent.1).saturating_sub(size.1))
            .max(origin.1);
    }
    (x, y)
}

fn open_composer(app: &AppHandle, draft_id: &uuid::Uuid) -> tauri::Result<()> {
    let label = composer_label(draft_id);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        return existing.set_focus();
    }
    let slot = composer_windows(app).len();
    let navigation_app = app.clone();
    let url = format!("index.html?composer={}", draft_id.hyphenated());
    let builder = window_chrome::style_builder(
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
            .title(NEW_MAIL_TITLE)
            .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
            .visible(false)
            // Nothing may navigate a composer away from the app page; its
            // links are plain text and never opened from here.
            .on_navigation(move |url| {
                window_access::url_allowed(url)
                    && crate::body_protocol::navigation(&navigation_app, url)
            }),
    );
    let builder = match placement(app, slot) {
        Some((x, y)) => builder.position(x, y),
        None => builder,
    };
    let window = builder.build()?;
    window_chrome::watch_state(&window);
    watch_close(&window);
    window.show()?;
    window.set_focus()
}

/// Logical position for the next composer, relative to the main window.
fn placement(app: &AppHandle, slot: usize) -> Option<(f64, f64)> {
    let main = app.get_webview_window(MAIN_WINDOW)?;
    let scale = main.scale_factor().ok()?;
    let origin = main.outer_position().ok()?;
    let monitor = main.current_monitor().ok().flatten().map(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        (
            (position.x, position.y),
            (
                i32::try_from(size.width).unwrap_or(i32::MAX),
                i32::try_from(size.height).unwrap_or(i32::MAX),
            ),
        )
    });
    let step = (CASCADE_STEP * scale).round() as i32;
    let size = (
        (WINDOW_WIDTH * scale).round() as i32,
        (WINDOW_HEIGHT * scale).round() as i32,
    );
    let (x, y) = cascade_position((origin.x, origin.y), step, slot, size, monitor);
    Some((f64::from(x) / scale, f64::from(y) / scale))
}

/// Installs close handling on a window: composers hand the close to their
/// page first; the main window hands the whole app's close to every composer.
pub(crate) fn watch_close(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let Some(kind) = WindowKind::parse(&label) else {
        return;
    };
    window.on_window_event(move |event| match (event, kind) {
        (WindowEvent::CloseRequested { api, .. }, WindowKind::Main) => {
            if main_close_requested(&app) {
                api.prevent_close();
            }
        }
        (WindowEvent::CloseRequested { api, .. }, WindowKind::Composer(_)) => {
            // Always intercepted: only `composer_close` (destroy) or the
            // timer below ends the window.
            api.prevent_close();
            composer_close_requested(&app, &label);
        }
        (WindowEvent::Destroyed, WindowKind::Composer(_)) => {
            if let Some(lifecycle) = app.try_state::<Lifecycle>() {
                lifecycle.finish(&label);
            }
        }
        _ => {}
    });
}

fn composer_close_requested(app: &AppHandle, label: &str) {
    let Some(lifecycle) = app.try_state::<Lifecycle>() else {
        return;
    };
    // While the app is quitting every composer already has its request.
    if lifecycle.quitting.load(Ordering::SeqCst) {
        return;
    }
    let Some(generation) = lifecycle.begin_close(label) else {
        return;
    };
    let _ = app.emit_to(
        EventTarget::webview_window(label),
        CLOSE_REQUESTED_EVENT,
        (),
    );
    let app = app.clone();
    let label = label.to_owned();
    std::thread::spawn(move || {
        std::thread::sleep(CLOSE_GRACE);
        let Some(lifecycle) = app.try_state::<Lifecycle>() else {
            return;
        };
        // Still waiting for the page, and the window is still the same one.
        if lifecycle.is_pending(&label, generation)
            && let Some(window) = app.get_webview_window(&label)
        {
            let _ = window.destroy();
        }
        lifecycle.finish(&label);
    });
}

/// Returns whether the app's exit is held back (main window closed, or a
/// quit from the menu / Cmd+Q). Without composers
/// there is nothing to wait for and the app exits as it always did.
pub(crate) fn main_close_requested(app: &AppHandle) -> bool {
    if let Some(lifecycle) = app.try_state::<Lifecycle>() {
        if lifecycle.exiting.load(Ordering::SeqCst) {
            return false;
        }
        if lifecycle.quitting.load(Ordering::SeqCst) {
            // Already waiting; a second click does not start another round.
            return true;
        }
    }
    let composers = composer_windows(app);
    if composers.is_empty() {
        return false;
    }
    let Some(lifecycle) = app.try_state::<Lifecycle>() else {
        return false;
    };
    lifecycle.quitting.store(true, Ordering::SeqCst);
    for window in &composers {
        let _ = app.emit_to(
            EventTarget::webview_window(window.label()),
            APP_CLOSING_EVENT,
            (),
        );
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + CLOSE_GRACE;
        while std::time::Instant::now() < deadline && !composer_windows(&app).is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
        for window in composer_windows(&app) {
            let _ = window.destroy();
        }
        if let Some(lifecycle) = app.try_state::<Lifecycle>() {
            lifecycle.exiting.store(true, Ordering::SeqCst);
        }
        app.exit(0);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::ipc::InvokeBody;

    #[test]
    fn events_go_to_main_and_the_affected_composer_only() {
        let id = uuid::Uuid::new_v4();
        let text = id.hyphenated().to_string();
        assert_eq!(
            event_labels(Some(&text)),
            vec!["main".to_owned(), format!("composer-{text}")]
        );
        assert_eq!(event_labels(None), vec!["main".to_owned()]);
        for bad in ["", "x", "composer-x", &text.to_uppercase()] {
            assert_eq!(event_labels(Some(bad)), vec!["main".to_owned()]);
        }
    }

    #[test]
    fn cascade_offsets_each_slot_and_wraps() {
        let size = (1520, 1360);
        let screen = Some(((0, 0), (5120, 2880)));
        assert_eq!(
            cascade_position((100, 100), 64, 0, size, screen),
            (164, 164)
        );
        assert_eq!(
            cascade_position((100, 100), 64, 1, size, screen),
            (228, 228)
        );
        assert_eq!(
            cascade_position((100, 100), 64, CASCADE_SLOTS, size, screen),
            (164, 164)
        );
        assert_eq!(cascade_position((100, 100), 64, 0, size, None), (164, 164));
    }

    #[test]
    fn cascade_stays_inside_the_monitor() {
        let size = (1520, 1360);
        // Near the bottom-right corner of a 2560x1600 monitor.
        let screen = Some(((0, 0), (2560, 1600)));
        assert_eq!(
            cascade_position((2400, 1500), 64, 0, size, screen),
            (2560 - 1520, 1600 - 1360)
        );
        // A second monitor to the left (negative origin).
        let left = Some(((-1920, 0), (1920, 1080)));
        let (x, y) = cascade_position((-100, 50), 64, 0, (1520, 1360), left);
        // Pulled back to the right edge (0 - 1520), and to the top because
        // the window is taller than this monitor.
        assert_eq!((x, y), (-1520, 0));
        // Never left or above the monitor's own corner.
        let (x, y) = cascade_position((-5000, -5000), 64, 0, size, screen);
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn a_close_request_is_recorded_once_per_window_and_generation() {
        let lifecycle = Lifecycle::default();
        let first = lifecycle.begin_close("composer-a").unwrap();
        assert!(lifecycle.begin_close("composer-a").is_none());
        let other = lifecycle.begin_close("composer-b").unwrap();
        assert_ne!(first, other);
        assert!(lifecycle.is_pending("composer-a", first));
        // The window went away and the label was reused: the old timer must
        // not close the new window.
        lifecycle.finish("composer-a");
        assert!(!lifecycle.is_pending("composer-a", first));
        let second = lifecycle.begin_close("composer-a").unwrap();
        assert_ne!(first, second);
        assert!(!lifecycle.is_pending("composer-a", first));
        assert!(lifecycle.is_pending("composer-a", second));
    }

    #[test]
    fn command_arguments_are_strict() {
        let id = uuid::Uuid::new_v4().to_string();
        let json = |value| InvokeBody::Json(value);
        assert!(parse::<DraftArgs>(&json(json!({"draftId": id}))).is_ok());
        assert!(parse::<DraftArgs>(&json(json!({"draftId": id, "x": 1}))).is_err());
        assert!(parse::<IdArgs>(&json(json!({"id": id}))).is_ok());
        assert!(parse::<IdArgs>(&json(json!({}))).is_err());
        assert!(parse::<CloseArgs>(&json(json!({"force": true}))).is_ok());
        assert!(parse::<CloseArgs>(&json(json!({}))).is_err());
        let list = parse::<OutboxListArgs>(&json(json!({}))).unwrap();
        assert_eq!((list.account_id, list.include_all), (None, false));
        assert!(parse::<DraftListArgs>(&json(json!({"accountId": null}))).is_ok());
        let resolve = parse::<ResolveArgs>(&json(
            json!({"id": id, "choice": "markSent", "clientMutationId": id}),
        ))
        .unwrap();
        assert_eq!(resolve.choice, UnknownResolution::MarkSent);
        // The principal is never part of a request.
        let queue = parse::<QueueSendRequest>(&json(
            json!({"draftId": id, "expectedRevision": 1, "clientMutationId": id}),
        ));
        assert!(queue.is_ok());
        assert!(
            parse::<QueueSendRequest>(&json(json!({
                "draftId": id, "expectedRevision": 1, "clientMutationId": id, "caller": "composer"
            })))
            .is_err()
        );
    }
}
