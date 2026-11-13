//! The shared trusted-host copy action. Only a recognized core capability can
//! reach the clipboard; optional cleanup follows successful writing, never it.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

use mailx_core::{
    ApiResult, AppError, ErrorCode, MailxCore, SupportedLocale, VerificationCodeCleanup,
    VerificationCodeCopyOutcome, VerificationCodeStatus,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};

use crate::{
    commands::{authorize, parse, respond},
    state::DesktopState,
    window_chrome::MAIN_WINDOW,
};

#[derive(Default)]
pub(crate) struct VerificationCodes {
    // On Linux clipboard ownership must outlive set_text; keep the native
    // clipboard for the process instead of clearing ownership when it drops.
    clipboard: Mutex<Option<arboard::Clipboard>>,
    undo: Mutex<VecDeque<UndoEntry>>,
}

#[derive(Clone)]
struct UndoEntry {
    token: String,
    batch_id: String,
    email_id: String,
    account_id: String,
    expires: Instant,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Changed {
    email_ids: Vec<String>,
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

#[derive(Debug, PartialEq, Eq)]
enum UndoOffer {
    Offered,
    Restored,
    Unavailable,
}

fn offer_undo(
    show: impl FnOnce() -> Result<(), ()>,
    restore: impl FnOnce() -> Result<(), AppError>,
) -> UndoOffer {
    if show().is_ok() {
        UndoOffer::Offered
    } else if restore().is_ok() {
        UndoOffer::Restored
    } else {
        UndoOffer::Unavailable
    }
}

pub(crate) fn copy(
    app: &AppHandle,
    core: &MailxCore,
    email_id: &str,
    recognition_id: &str,
    native_token: Option<&str>,
) -> Result<VerificationCodeCopyOutcome, AppError> {
    core.copy_verification_code_with_policy(email_id, recognition_id, |code| {
        let snapshot = app
            .state::<DesktopState>()
            .notification_snapshot()
            .ok_or_else(internal)?;
        crate::notifications::write_verification_code(
            app,
            &snapshot,
            email_id,
            recognition_id,
            native_token,
            || {
                let state = app.state::<VerificationCodes>();
                let mut clipboard = state.clipboard.lock().map_err(|_| internal())?;
                if clipboard.is_none() {
                    *clipboard = Some(arboard::Clipboard::new().map_err(|_| internal())?);
                }
                clipboard
                    .as_mut()
                    .ok_or_else(internal)?
                    .set_text(code.to_owned())
                    .map_err(|_| internal())
            },
        )
    })
}

/// Register undo for both UI and native callers. The UI gets its own response;
/// only background copies emit a second event for a visible workbench toast.
pub(crate) fn finish_copy(
    app: &AppHandle,
    core: &MailxCore,
    mut outcome: VerificationCodeCopyOutcome,
    background: bool,
    locale: SupportedLocale,
) -> VerificationCodeCopyOutcome {
    if let Some(deletion) = &outcome.deletion {
        let ttl = undo_remaining(&deletion.undo_until);
        let state = app.state::<VerificationCodes>();
        let token = uuid::Uuid::new_v4().to_string();
        {
            let mut entries = state.undo.lock().unwrap_or_else(|error| error.into_inner());
            entries.retain(|entry| entry.expires > Instant::now());
            entries.push_back(UndoEntry {
                token: token.clone(),
                batch_id: deletion.batch_id.clone(),
                email_id: outcome.email_id.clone(),
                account_id: outcome.account_id.clone(),
                expires: Instant::now() + ttl,
            });
        }
        if background {
            // If offering undo fails, attempt to preserve the mail. The server
            // may already have claimed it: distinguish that from a restoration.
            match offer_undo(
                || crate::notifications::show_code_undo(app, token, ttl, locale),
                || core.undo_delete(&deletion.batch_id),
            ) {
                UndoOffer::Restored => {
                    state
                        .undo
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .retain(|entry| entry.batch_id != deletion.batch_id);
                    outcome.deletion = None;
                    outcome.cleanup = VerificationCodeCleanup::Failed;
                }
                UndoOffer::Unavailable => {
                    crate::notifications::show_code_undo_unavailable(app, locale)
                }
                UndoOffer::Offered => {}
            }
        }
        if background {
            changed(app, vec![outcome.email_id.clone()]);
        }
    }
    if background {
        let _ = app.emit_to(
            EventTarget::webview_window(MAIN_WINDOW),
            "mailx://verification-code-copied",
            &outcome,
        );
        if matches!(
            outcome.cleanup,
            VerificationCodeCleanup::Unavailable | VerificationCodeCleanup::Failed
        ) {
            crate::notifications::show_code_cleanup_failed(app, locale);
        }
    }
    outcome
}

fn undo_remaining(until: &str) -> Duration {
    chrono::DateTime::parse_from_rfc3339(until)
        .ok()
        .and_then(|until| {
            until
                .signed_duration_since(chrono::Utc::now())
                .to_std()
                .ok()
        })
        .unwrap_or_default()
        .min(Duration::from_secs(5))
}

fn changed(app: &AppHandle, email_ids: Vec<String>) {
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        "mailx://verification-code-changed",
        Changed { email_ids },
    );
}

pub(crate) fn recognized(app: &AppHandle, email_id: String) {
    changed(app, vec![email_id]);
}

pub(crate) fn retire(app: &AppHandle, account_id: &str) {
    if let Some(state) = app.try_state::<VerificationCodes>() {
        state
            .undo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|entry| entry.account_id != account_id);
    }
}

pub(crate) fn undo_native(app: &AppHandle, token: &str) -> Result<(), AppError> {
    let state = app.state::<VerificationCodes>();
    let entry = state
        .undo
        .lock()
        .map_err(|_| internal())?
        .iter()
        .find(|entry| entry.token == token && entry.expires > Instant::now())
        .cloned()
        .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
    let core = app.state::<DesktopState>().core()?;
    undo_entry(app, &core, &entry)
}

fn undo_entry(app: &AppHandle, core: &MailxCore, entry: &UndoEntry) -> Result<(), AppError> {
    core.undo_delete(&entry.batch_id)?;
    app.state::<VerificationCodes>()
        .undo
        .lock()
        .map_err(|_| internal())?
        .retain(|item| item.batch_id != entry.batch_id);
    changed(app, vec![entry.email_id.clone()]);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusArgs {
    email_id: String,
    allow_fetch: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CopyArgs {
    email_id: String,
    recognition_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UndoArgs {
    batch_id: String,
}

#[tauri::command]
pub(crate) async fn emails_verification_code_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<VerificationCodeStatus>, ()> {
    let args = authorize(&window, "emails_verification_code_status")
        .and_then(|_| parse::<StatusArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => match state.core() {
            Ok(core) => {
                core.verification_code_status(args.email_id, args.allow_fetch)
                    .await
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_copy_verification_code(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<VerificationCodeCopyOutcome>, ()> {
    let args = authorize(&window, "emails_copy_verification_code")
        .and_then(|_| parse::<CopyArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            let state = state.inner().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let snapshot = state.notification_snapshot().ok_or_else(internal)?;
                let core = state.core()?;
                let outcome = copy(&app, &core, &args.email_id, &args.recognition_id, None)?;
                Ok(finish_copy(
                    &app,
                    &core,
                    outcome,
                    false,
                    snapshot.effective_locale,
                ))
            })
            .await
            .map_err(|_| internal())
            .and_then(|result| result)
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_undo_verification_code_delete(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "emails_undo_verification_code_delete")
        .and_then(|_| parse::<UndoArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| {
                    let entry = app
                        .state::<VerificationCodes>()
                        .undo
                        .lock()
                        .map_err(|_| internal())?
                        .iter()
                        .find(|entry| {
                            entry.batch_id == args.batch_id && entry.expires > Instant::now()
                        })
                        .cloned()
                        .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
                    undo_entry(&app, core, &entry)
                })
                .await
        }
        Err(error) => Err(error),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_cannot_supply_a_code_or_opt_into_permanent_deletion() {
        let valid = serde_json::json!({ "emailId": "mail", "recognitionId": "opaque" });
        assert!(serde_json::from_value::<CopyArgs>(valid.clone()).is_ok());
        for field in ["code", "confirmedPermanent", "deleteAfterCopy"] {
            let mut value = valid.clone();
            value[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<CopyArgs>(value).is_err());
        }
    }
    #[test]
    fn undo_duration_is_bounded_and_stale_or_invalid_times_are_expired() {
        assert_eq!(undo_remaining("invalid"), Duration::ZERO);
        assert_eq!(undo_remaining("2000-01-01T00:00:00Z"), Duration::ZERO);
        assert!(undo_remaining("2099-01-01T00:00:00Z") <= Duration::from_secs(5));
    }
    #[test]
    fn failed_undo_offer_distinguishes_restored_mail_from_unavailable_rollback() {
        assert_eq!(
            offer_undo(
                || Ok(()),
                || panic!("a visible undo must not restore immediately")
            ),
            UndoOffer::Offered
        );
        assert_eq!(offer_undo(|| Err(()), || Ok(())), UndoOffer::Restored);
        assert_eq!(
            offer_undo(
                || Err(()),
                || Err(AppError::new(ErrorCode::AlreadyDeleting))
            ),
            UndoOffer::Unavailable
        );
    }
}
