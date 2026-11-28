//! Main-window IPC commands for calendar/task action items: L0 `.ics` reading,
//! L1 document action queries, L2 cached results, status persistence, safe
//! `.ics` opening/saving, and task clipboard copying.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Arc,
};

use mailx_core::{
    ActionItemExportInput, ActionItemStatus, ApiResult, AppError, CachedEmailActionItems,
    DetectedActionItem, ErrorCode, MailxCore,
};
use serde::Deserialize;
use tauri::{Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::{
    attachment_save::{self, SaveOutcome},
    body_protocol::{BodyProtocol, DocumentActions},
    commands::{authorize as authorize_window, respond},
    eventkit::{self, NativeAddOutcome, NativeCalendarCapabilities},
    state::DesktopState,
};

const TEMP_ICS_DIR: &str = "mailx-ics-temp";

fn authorize(window: &WebviewWindow, command: &str) -> Result<(), AppError> {
    authorize_window(window, command).map(|_| ())
}

fn parse<T: serde::de::DeserializeOwned>(body: &tauri::ipc::InvokeBody) -> Result<T, AppError> {
    let tauri::ipc::InvokeBody::Json(value) = body else {
        return Err(AppError::validation("request"));
    };
    serde_json::from_value(value.clone()).map_err(|_| AppError::validation("request"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailIdArgs {
    email_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailActionItemsArgs {
    email_id: String,
    #[serde(default)]
    time_zone: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DocumentActionsArgs {
    email_id: String,
    document_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetActionStatusArgs {
    email_id: String,
    item_id: String,
    status: ActionItemStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportInputArgs {
    input: ActionItemExportInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrivacySettingsArgs {
    kind: mailx_core::ActionItemKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmptyArgs {}

fn temp_ics_dir(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    let base = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().temp_dir())
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    Ok(base.join(TEMP_ICS_DIR))
}

/// Removes stale temporary `.ics` files left over from previous sessions.
/// Files created during the current session are kept until the next launch so
/// asynchronous system calendar apps never race a short timeout deletion.
pub(crate) fn cleanup_stale_ics_temp_files(app: &tauri::AppHandle) {
    let Ok(dir) = temp_ics_dir(app) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("ics") {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_temp_ics(app: &tauri::AppHandle, ics: &str) -> Result<PathBuf, AppError> {
    let dir = temp_ics_dir(app)?;
    fs::create_dir_all(&dir).map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    let path = dir.join(format!("{}.ics", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    file.write_all(ics.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    Ok(path)
}

#[tauri::command]
pub(crate) async fn mail_get_calendar_items(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<DetectedActionItem>>, ()> {
    let args = authorize(&window, "mail_get_calendar_items")
        .and_then(|_| parse::<EmailIdArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state.core()?.email_calendar_items(args.email_id).await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_list_document_actions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, Arc<BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<DocumentActions>, ()> {
    let args = authorize(&window, "mail_list_document_actions")
        .and_then(|_| parse::<DocumentActionsArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let core = state.core()?;
            let mut listed =
                protocol.list_document_actions(args.email_id.clone(), args.document_token)?;
            if !listed.items.is_empty()
                && let Ok(states) = core.email_action_states(args.email_id).await
            {
                for item in &mut listed.items {
                    if let Some(status) = states.get(&item.id) {
                        item.status = *status;
                    }
                }
            }
            Ok(listed)
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_set_action_item_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "mail_set_action_item_status")
        .and_then(|_| parse::<SetActionStatusArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .core()?
                .set_action_item_status(args.email_id, args.item_id, args.status)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_email_action_items(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Option<CachedEmailActionItems>>, ()> {
    let args = authorize(&window, "assistant_email_action_items")
        .and_then(|_| parse::<EmailActionItemsArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .assistant(&app)?
                .email_action_items(&args.email_id, args.time_zone.as_deref())
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_open_action_item_calendar(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SaveOutcome>, ()> {
    let args = authorize(&window, "mail_open_action_item_calendar")
        .and_then(|_| parse::<ExportInputArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let ics = MailxCore::generate_action_item_ics(&args.input)?;
            let path = write_temp_ics(&app, &ics)?;
            let opened = app
                .opener()
                .open_path(path.to_string_lossy().as_ref(), None::<&str>);
            if opened.is_ok() {
                let _ = state
                    .core()?
                    .set_action_item_status(
                        args.input.email_id,
                        args.input.item_id,
                        ActionItemStatus::Added,
                    )
                    .await;
                return Ok(SaveOutcome::Saved);
            }
            // Fallback to system Save dialog when no default `.ics` handler exists.
            save_ics_with_dialog(&app, &window, &state, &args.input, &ics).await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_save_action_item_ics(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SaveOutcome>, ()> {
    let args = authorize(&window, "mail_save_action_item_ics")
        .and_then(|_| parse::<ExportInputArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let ics = MailxCore::generate_action_item_ics(&args.input)?;
            save_ics_with_dialog(&app, &window, &state, &args.input, &ics).await
        }
        .await,
    ))
}

async fn save_ics_with_dialog(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    state: &DesktopState,
    input: &ActionItemExportInput,
    ics: &str,
) -> Result<SaveOutcome, AppError> {
    let default_name = match input.kind {
        mailx_core::ActionItemKind::Event => "event.ics",
        mailx_core::ActionItemKind::Task => "task.ics",
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(window)
        .set_file_name(default_name)
        .add_filter("iCalendar", &["ics"])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx.await.map_err(|_| AppError::new(ErrorCode::Internal))? else {
        return Ok(SaveOutcome::Cancelled);
    };
    let path = picked
        .into_path()
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    let bytes = ics.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || attachment_save::write_atomically(&path, &bytes))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
    let _ = state
        .core()?
        .set_action_item_status(
            input.email_id.clone(),
            input.item_id.clone(),
            ActionItemStatus::Added,
        )
        .await;
    Ok(SaveOutcome::Saved)
}

#[tauri::command]
pub(crate) async fn mail_copy_action_item_text(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "mail_copy_action_item_text")
        .and_then(|_| parse::<ExportInputArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let text = MailxCore::format_action_item_task_text(&args.input)?;
            tokio::task::spawn_blocking(move || -> Result<(), AppError> {
                let mut clipboard =
                    arboard::Clipboard::new().map_err(|_| AppError::new(ErrorCode::Internal))?;
                clipboard
                    .set_text(text)
                    .map_err(|_| AppError::new(ErrorCode::Internal))
            })
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
            let _ = state
                .core()?
                .set_action_item_status(
                    args.input.email_id,
                    args.input.item_id,
                    ActionItemStatus::Added,
                )
                .await;
            Ok(())
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_action_item_capabilities(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<NativeCalendarCapabilities>, ()> {
    let args = authorize(&window, "mail_action_item_capabilities")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            Ok(eventkit::capabilities())
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_add_action_item_native(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<NativeAddOutcome>, ()> {
    let args = authorize(&window, "mail_add_action_item_native")
        .and_then(|_| parse::<ExportInputArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let input = args.input.clone();
            let outcome = tokio::task::spawn_blocking(move || eventkit::add_native(&input))
                .await
                .map_err(|_| AppError::new(ErrorCode::Internal))??;
            if outcome == NativeAddOutcome::Added {
                let _ = state
                    .core()?
                    .set_action_item_status(
                        args.input.email_id,
                        args.input.item_id,
                        ActionItemStatus::Added,
                    )
                    .await;
            }
            Ok(outcome)
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn mail_open_calendar_privacy_settings(
    app: tauri::AppHandle,
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "mail_open_calendar_privacy_settings")
        .and_then(|_| parse::<PrivacySettingsArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            if let Some(url) = eventkit::privacy_settings_url(args.kind) {
                let _ = app.opener().open_url(url, None::<&str>);
            }
            Ok(())
        }
        .await,
    ))
}
