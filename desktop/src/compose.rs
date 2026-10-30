//! Composer windows and the sending commands.
//!
//! One window per draft, labelled `composer-{draftId}`, loading the same
//! frontend bundle as the main window with `?composer={draftId}`. Which
//! commands each window may call is decided in `window_access`; the principal
//! given to the core comes from the window kind, never from the request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mailx_core::{
    AccountRemoval, AppError, CreateDraftFromRequest, CreateDraftRequest, Draft, DraftChanged,
    DraftSummary, ErrorCode, OutboxChanged, OutboxItem, QueueSendRequest, ResolveUnknownRequest,
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
/// Every account picker must forget a removed account, including composers
/// whose current draft belongs to a different account.
pub(crate) const ACCOUNT_REMOVED_EVENT: &str = "mailx://account-removed";
/// Pushed to every composer for an explicit application quit: save, then acknowledge
/// the generation with `composer_close`, or cancel with `composer_cancel_close`.
const APP_CLOSING_EVENT: &str = "mailx://app-closing";
/// Pushed to a composer when the system asked to close it (red button, Alt+F4,
/// the drawn close button): flush the draft, then call `composer_close`.
const CLOSE_REQUESTED_EVENT: &str = "mailx://composer-close-requested";

const CLOSE_CANCELLED_EVENT: &str = "mailx://app-close-cancelled";
/// A slow draft or SMTP transaction cancels quitting; it never forces data loss.
const QUIT_WAIT_LIMIT: Duration = Duration::from_secs(30);
const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 680.0;
const MIN_WIDTH: f64 = 520.0;
const MIN_HEIGHT: f64 = 420.0;
/// Offset of each further composer from the main window, in logical pixels.
const CASCADE_STEP: f64 = 32.0;
/// Composers cascade over this many positions, then start again.
const CASCADE_SLOTS: usize = 8;

/// Window acknowledgements and explicit quit attempts share one generation space.
#[derive(Default)]
pub(crate) struct Lifecycle {
    exiting: AtomicBool,
    control: Mutex<CloseControl>,
    /// Only worker threads take this lock. A cancelled attempt resumes sending
    /// before a later attempt pauses it, even when the first pause was delayed.
    coordinator: Mutex<()>,
}

#[derive(Default)]
struct CloseControl {
    pending: HashMap<String, u64>,
    quitting: Option<u64>,
    generation: u64,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CloseRequest {
    generation: u64,
}

impl Lifecycle {
    fn begin_close(&self, label: &str) -> Option<u64> {
        let mut control = self.control.lock().ok()?;
        if control.quitting.is_some() {
            return None;
        }
        if let Some(generation) = control.pending.get(label) {
            return Some(*generation);
        }
        control.generation += 1;
        let generation = control.generation;
        control.pending.insert(label.to_owned(), generation);
        Some(generation)
    }

    fn begin_quit(&self, labels: impl IntoIterator<Item = String>) -> Option<u64> {
        let mut control = self.control.lock().ok()?;
        if self.exiting.load(Ordering::SeqCst) || control.quitting.is_some() {
            return None;
        }
        control.generation += 1;
        let generation = control.generation;
        control.quitting = Some(generation);
        control.pending = labels
            .into_iter()
            .map(|label| (label, generation))
            .collect();
        Some(generation)
    }

    #[cfg(test)]
    fn is_pending(&self, label: &str, generation: u64) -> bool {
        self.control
            .lock()
            .is_ok_and(|control| control.pending.get(label) == Some(&generation))
    }

    fn accepts_close(&self, label: &str, generation: Option<u64>, force: bool) -> bool {
        if force {
            return true;
        }
        self.control.lock().is_ok_and(|control| match generation {
            Some(generation) => control.pending.get(label) == Some(&generation),
            None => !control.pending.contains_key(label),
        })
    }

    fn current_quit(&self) -> Option<u64> {
        self.control
            .lock()
            .ok()
            .and_then(|control| control.quitting)
    }

    fn cancel(&self, request: Option<(&str, u64)>) -> Option<CloseRequest> {
        let mut control = self.control.lock().ok()?;
        if self.exiting.load(Ordering::SeqCst) {
            return None;
        }
        if let Some((label, generation)) = request {
            if control.pending.get(label) != Some(&generation) {
                return None;
            }
            if control.quitting.is_none() {
                control.pending.remove(label);
                return Some(CloseRequest { generation });
            }
        }
        let generation = control.quitting.take()?;
        control.pending.clear();
        Some(CloseRequest { generation })
    }

    fn cancel_generation(&self, generation: u64) -> Option<CloseRequest> {
        let mut control = self.control.lock().ok()?;
        if self.exiting.load(Ordering::SeqCst) || control.quitting != Some(generation) {
            return None;
        }
        control.quitting = None;
        control.pending.clear();
        Some(CloseRequest { generation })
    }

    fn finish(&self, label: &str) {
        if let Ok(mut control) = self.control.lock() {
            control.pending.remove(label);
        }
    }

    fn confirm_main(&self, generation: u64) -> Result<(), AppError> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if control.quitting != Some(generation)
            || control.pending.get(MAIN_WINDOW) != Some(&generation)
        {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        control.pending.remove(MAIN_WINDOW);
        Ok(())
    }

    fn complete_quit(&self, generation: u64) -> bool {
        let Ok(control) = self.control.lock() else {
            return false;
        };
        if control.quitting != Some(generation) || !control.pending.is_empty() {
            return false;
        }
        self.exiting.store(true, Ordering::SeqCst);
        true
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

fn account_event_target(label: &str, url: &tauri::Url) -> bool {
    WindowKind::parse(label).is_some() && window_access::url_allowed(url)
}

pub(crate) fn emit_account_removed(app: &AppHandle, removed: &AccountRemoval) {
    for (label, window) in app.webview_windows() {
        if window
            .url()
            .is_ok_and(|url| account_event_target(&label, &url))
        {
            let _ = app.emit_to(
                EventTarget::webview_window(label),
                ACCOUNT_REMOVED_EVENT,
                removed.clone(),
            );
        }
    }
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
struct OpenArgs {
    draft_id: String,
    /// The draft was just created for this window (写邮件 / 回复 / 转发), so
    /// closing without saving deletes it. Absent when reopened from a list.
    #[serde(default)]
    fresh: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseArgs {
    /// Account removal has already deleted the draft and may bypass an outstanding close.
    force: bool,
    #[serde(default)]
    generation: Option<u64>,
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
        Ok(args) => match state.settings(|store| store.get()).await {
            Ok(snapshot) => {
                state
                    .run(move |core| {
                        core.create_draft_from_localized(args.request, snapshot.effective_locale)
                    })
                    .await
            }
            Err(error) => Err(error),
        },
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveLocalCopyArgs {
    email_id: String,
}

/// Deletes a local Sent copy (`local-sent:<outbox uuid>`) from this device.
/// Main window only; the server is not touched. Idempotent. The core announces
/// the change on the sent-copies event so Sent lists refetch.
#[tauri::command]
pub(crate) async fn outbox_remove_local_copy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "outbox_remove_local_copy").and_then(|_| {
        let args = parse::<RemoveLocalCopyArgs>(request.body())?;
        window_access::local_copy_outbox_id(&args.email_id)
            .ok_or_else(|| AppError::validation("emailId"))?;
        Ok(args)
    });
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.remove_local_copy(&args.email_id))
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
        let args = parse::<OpenArgs>(request.body())?;
        parse_draft_id(&args.draft_id)
            .map(|id| (id, args.fresh))
            .ok_or_else(|| AppError::validation("draftId"))
    });
    let result = async {
        let (id, fresh) = args?;
        if is_quitting(&app) {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let text = id.hyphenated().to_string();
        // The draft must exist; a window for nothing is never created.
        state
            .run(move |core| core.get_draft(&text).map(|_| ()))
            .await?;
        if is_quitting(&app) {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        open_composer(&app, &id, fresh).map_err(|_| AppError::new(ErrorCode::Internal))
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
    Ok(respond(args.and_then(|args| {
        if !window.app_handle().state::<Lifecycle>().accepts_close(
            window.label(),
            args.generation,
            args.force,
        ) {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        window
            .destroy()
            .map_err(|_| AppError::new(ErrorCode::Internal))
    })))
}

/// Cancels an ordinary window close or the matching whole-application quit.
#[tauri::command]
pub(crate) async fn composer_cancel_close(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "composer_cancel_close")
        .and_then(|_| parse::<CloseRequest>(request.body()));
    Ok(respond(args.map(|request| {
        let app = window.app_handle();
        if let Some(cancelled) = app
            .state::<Lifecycle>()
            .cancel(Some((window.label(), request.generation)))
        {
            close_cancelled(app, cancelled);
        }
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

/// Page of a composer window; `fresh` marks a draft created for it.
fn composer_url(draft_id: &uuid::Uuid, fresh: bool) -> String {
    let url = format!("index.html?composer={}", draft_id.hyphenated());
    if fresh { format!("{url}&fresh=1") } else { url }
}

fn open_composer(app: &AppHandle, draft_id: &uuid::Uuid, fresh: bool) -> tauri::Result<()> {
    let label = composer_label(draft_id);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        return existing.set_focus();
    }
    let slot = composer_windows(app).len();
    let navigation_app = app.clone();
    let url = composer_url(draft_id, fresh);
    let locale = app
        .try_state::<DesktopState>()
        .and_then(|state| state.notification_settings().map(|(_, locale)| locale))
        .unwrap_or_else(crate::locale::system_locale);
    let builder = window_chrome::style_builder(
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
            .title(locale.new_mail_title())
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
/// page first; the main window follows its own background/quit preference.
pub(crate) fn watch_close(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let Some(kind) = WindowKind::parse(&label) else {
        return;
    };
    window.on_window_event(move |event| match (event, kind) {
        (WindowEvent::CloseRequested { api, .. }, WindowKind::Main) => {
            if !is_exiting(&app) {
                api.prevent_close();
                crate::lifecycle::close_main_requested(&app);
            }
        }
        (WindowEvent::CloseRequested { api, .. }, WindowKind::Composer(_)) => {
            api.prevent_close();
            composer_close_requested(&app, &label);
        }
        (WindowEvent::DragDrop(drop), WindowKind::Composer(_)) => {
            crate::compose_files::on_drag_drop(&app, &label, drop);
        }
        (WindowEvent::Destroyed, WindowKind::Composer(_)) => {
            if let Some(dropped) = app.try_state::<crate::compose_files::DroppedFiles>() {
                dropped.forget(&label);
            }
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
    let Some(generation) = lifecycle.begin_close(label) else {
        return;
    };
    let _ = app.emit_to(
        EventTarget::webview_window(label),
        CLOSE_REQUESTED_EVENT,
        CloseRequest { generation },
    );
}

pub(crate) fn is_exiting(app: &AppHandle) -> bool {
    app.try_state::<Lifecycle>()
        .is_some_and(|state| state.exiting.load(Ordering::SeqCst))
}

pub(crate) fn is_quitting(app: &AppHandle) -> bool {
    app.try_state::<Lifecycle>()
        .is_some_and(|state| state.current_quit().is_some())
}

pub(crate) fn quit_generation(app: &AppHandle) -> Option<u64> {
    app.try_state::<Lifecycle>()
        .and_then(|state| state.current_quit())
}

pub(crate) fn confirm_main_quit(app: &AppHandle, generation: u64) -> Result<(), AppError> {
    app.state::<Lifecycle>().confirm_main(generation)
}

pub(crate) fn cancel_quit_generation(app: &AppHandle, generation: u64) {
    if let Some(state) = app.try_state::<Lifecycle>()
        && let Some(cancelled) = state.cancel_generation(generation)
    {
        close_cancelled(app, cancelled);
    }
}

fn close_cancelled(app: &AppHandle, cancelled: CloseRequest) {
    let _ = app.emit(CLOSE_CANCELLED_EVENT, cancelled);
    crate::updater::cancel_restart(app);
    crate::lifecycle::publish(app);
}

pub(crate) fn cancel_quit(app: &AppHandle) {
    if let Some(state) = app.try_state::<Lifecycle>()
        && let Some(cancelled) = state.cancel(None)
    {
        close_cancelled(app, cancelled);
    }
}

/// An explicit quit saves composers and lets active SMTP transactions finish.
/// No ordinary window close enters this path. A cancelled/expired generation
/// cannot close a window or terminate a subsequent session.
pub(crate) fn request_quit(app: &AppHandle) {
    let Some(lifecycle) = app.try_state::<Lifecycle>() else {
        return;
    };
    let composers = composer_windows(app);
    let labels = composers
        .iter()
        .map(|window| window.label().to_owned())
        .chain(crate::lifecycle::main_ready(app).then(|| MAIN_WINDOW.to_owned()));
    let Some(generation) = lifecycle.begin_quit(labels) else {
        return;
    };
    crate::lifecycle::publish(app);
    for window in &composers {
        let _ = app.emit_to(
            EventTarget::webview_window(window.label()),
            APP_CLOSING_EVENT,
            CloseRequest { generation },
        );
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let lifecycle = app.state::<Lifecycle>();
        let _coordinator = lifecycle
            .coordinator
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lifecycle.current_quit() != Some(generation) {
            return;
        }
        let core = app
            .try_state::<DesktopState>()
            .and_then(|state| state.core().ok());
        if let Some(core) = &core {
            core.begin_shutdown();
        }
        let deadline = std::time::Instant::now() + QUIT_WAIT_LIMIT;
        loop {
            if lifecycle.current_quit() != Some(generation) {
                if let Some(core) = &core {
                    core.cancel_shutdown();
                }
                return;
            }
            if composer_windows(&app).is_empty()
                && core.as_ref().is_none_or(|core| core.shutdown_idle())
                && lifecycle.complete_quit(generation)
            {
                app.exit(0);
                return;
            }
            if std::time::Instant::now() >= deadline {
                if let Some(cancelled) = lifecycle.cancel_generation(generation) {
                    close_cancelled(&app, cancelled);
                    crate::lifecycle::quit_timed_out(&app);
                }
                if let Some(core) = &core {
                    core.cancel_shutdown();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::ipc::InvokeBody;

    #[test]
    fn account_removal_targets_only_trusted_main_and_composers() {
        let app = tauri::Url::parse("tauri://localhost/index.html").unwrap();
        let external = tauri::Url::parse("https://mail.example.test").unwrap();
        let composer = composer_label(&uuid::Uuid::new_v4());
        for label in [MAIN_WINDOW, composer.as_str()] {
            assert!(account_event_target(label, &app));
            assert!(!account_event_target(label, &external));
        }
        assert!(!account_event_target("composer-invalid", &app));
        assert!(!account_event_target("body", &app));
    }

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
    fn the_fresh_flag_is_carried_in_the_page_url_only_when_set() {
        let id = uuid::Uuid::new_v4();
        let text = id.hyphenated().to_string();
        assert_eq!(
            composer_url(&id, false),
            format!("index.html?composer={text}")
        );
        assert_eq!(
            composer_url(&id, true),
            format!("index.html?composer={text}&fresh=1")
        );
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
        assert_eq!(lifecycle.begin_close("composer-a"), Some(first));
        let other = lifecycle.begin_close("composer-b").unwrap();
        assert_ne!(first, other);
        assert!(lifecycle.is_pending("composer-a", first));
        // A late acknowledgement must not close a new window with this label.
        lifecycle.finish("composer-a");
        assert!(!lifecycle.is_pending("composer-a", first));
        let second = lifecycle.begin_close("composer-a").unwrap();
        assert_ne!(first, second);
        assert!(!lifecycle.is_pending("composer-a", first));
        assert!(lifecycle.is_pending("composer-a", second));
    }

    #[test]
    fn cancelling_a_close_allows_the_window_to_be_closed_again() {
        let lifecycle = Lifecycle::default();
        let first = lifecycle.begin_close("composer-a").unwrap();
        assert!(lifecycle.cancel(Some(("composer-a", first))).is_some());
        let second = lifecycle.begin_close("composer-a").unwrap();
        assert_ne!(first, second);
        assert!(lifecycle.cancel(Some(("composer-a", first))).is_none());
        assert!(lifecycle.is_pending("composer-a", second));
    }

    #[test]
    fn cancelled_quit_acknowledgements_cannot_complete_a_later_quit() {
        let lifecycle = Lifecycle::default();
        let first = lifecycle.begin_quit(["composer-a".to_owned()]).unwrap();
        assert!(lifecycle.begin_close("composer-b").is_none());
        assert!(lifecycle.cancel(Some(("composer-a", first))).is_some());
        let second = lifecycle.begin_quit(["composer-a".to_owned()]).unwrap();
        assert!(!lifecycle.is_pending("composer-a", first));
        assert!(!lifecycle.accepts_close("composer-a", Some(first), false));
        assert!(!lifecycle.accepts_close("composer-a", None, false));
        assert!(lifecycle.accepts_close("composer-a", Some(second), false));
        assert!(lifecycle.accepts_close("composer-a", None, true));
        assert!(lifecycle.cancel_generation(first).is_none());
        assert!(!lifecycle.complete_quit(first));
        assert!(!lifecycle.complete_quit(second));
        lifecycle.finish("composer-a");
        assert!(lifecycle.complete_quit(second));
        assert!(lifecycle.cancel(None).is_none());
    }

    #[test]
    fn main_settings_ack_is_required_and_bound_to_the_quit_attempt() {
        let lifecycle = Lifecycle::default();
        let first = lifecycle.begin_quit([MAIN_WINDOW.to_owned()]).unwrap();
        assert!(!lifecycle.complete_quit(first));
        lifecycle.cancel_generation(first).unwrap();
        let second = lifecycle.begin_quit([MAIN_WINDOW.to_owned()]).unwrap();
        assert!(lifecycle.confirm_main(first).is_err());
        assert!(!lifecycle.complete_quit(second));
        lifecycle.confirm_main(second).unwrap();
        assert!(lifecycle.confirm_main(second).is_err());
        assert!(lifecycle.complete_quit(second));
    }

    #[test]
    fn command_arguments_are_strict() {
        let id = uuid::Uuid::new_v4().to_string();
        let json = |value| InvokeBody::Json(value);
        assert!(parse::<OpenArgs>(&json(json!({"draftId": id}))).is_ok());
        assert!(parse::<OpenArgs>(&json(json!({"draftId": id, "fresh": true}))).is_ok());
        assert!(parse::<OpenArgs>(&json(json!({"draftId": id, "x": 1}))).is_err());
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
