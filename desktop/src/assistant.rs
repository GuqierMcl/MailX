//! Built-in assistant commands. Main window only: the composer windows and
//! mail content never reach them (`window_access`, capabilities). The run
//! itself lives in `mailx_core::Assistant`; this module only authorizes,
//! parses and forwards. The consent is granted here and nowhere else (the
//! settings patch ignores it).
use mailx_core::{
    AiConsent, ApiResult, AppError, ConversationDetail, ConversationList, ErrorCode, RunSnapshot,
    StartRun,
};
use serde::Deserialize;
use tauri::{State, WebviewWindow};

use crate::commands::{EmptyArgs, authorize, parse, respond};
use crate::state::DesktopState;

/// Pushed to the main window for every assistant event.
pub(crate) const EVENT: &str = "mailx://assistant";
/// Pushed to the main window when saved conversations changed.
pub(crate) const CONVERSATIONS_EVENT: &str = "mailx://assistant-conversations";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    request: StartRun,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunArgs {
    run_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationArgs {
    conversation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RespondArgs {
    run_id: String,
    request_id: String,
    approved: bool,
}

#[tauri::command]
pub(crate) async fn assistant_start(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<RunSnapshot>, ()> {
    let args =
        authorize(&window, "assistant_start").and_then(|_| parse::<StartArgs>(request.body()));
    Ok(respond(
        async { state.assistant(&app)?.start(args?.request).await }.await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_cancel(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args =
        authorize(&window, "assistant_cancel").and_then(|_| parse::<RunArgs>(request.body()));
    Ok(respond(
        async { state.assistant(&app)?.cancel(&args?.run_id) }.await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_snapshot(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<RunSnapshot>, ()> {
    let args =
        authorize(&window, "assistant_snapshot").and_then(|_| parse::<RunArgs>(request.body()));
    Ok(respond(
        async { state.assistant(&app)?.snapshot(&args?.run_id) }.await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_respond(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args =
        authorize(&window, "assistant_respond").and_then(|_| parse::<RespondArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .assistant(&app)?
                .respond(&args.run_id, &args.request_id, args.approved)
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_grant_consent(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<AiConsent>, ()> {
    let args = authorize(&window, "assistant_grant_consent")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            let consent = state.assistant(&app)?.grant_consent().await?;
            settings_changed(&app, &state);
            Ok(consent)
        }
        .await,
    ))
}

/// Consent lives in the settings file: tell every window, like a settings
/// update does, so their settings caches see the new revision.
fn settings_changed(app: &tauri::AppHandle, state: &DesktopState) {
    if let Some(snapshot) = state.notification_snapshot() {
        crate::commands::emit_settings_changed(app, &snapshot);
    }
}

/// Works without a ready core: settings do not need one, and without a core
/// there are no runs to stop.
#[tauri::command]
pub(crate) async fn assistant_revoke_consent(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "assistant_revoke_consent")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            let result = match state.assistant(&app) {
                Ok(assistant) => assistant.revoke_consent(),
                Err(error) if error.code == ErrorCode::CoreNotReady => {
                    state
                        .settings(|store| {
                            store.update_assistant(|assistant| {
                                assistant.consented = None;
                                Ok(())
                            })
                        })
                        .await
                }
                Err(error) => Err::<(), AppError>(error),
            };
            if result.is_ok() {
                settings_changed(&app, &state);
            }
            result
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_list_conversations(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<ConversationList>, ()> {
    let args = authorize(&window, "assistant_list_conversations")
        .and_then(|_| parse::<ListArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .assistant(&app)?
                .list_conversations(args.limit, args.cursor)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_get_conversation(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<ConversationDetail>, ()> {
    let args = authorize(&window, "assistant_get_conversation")
        .and_then(|_| parse::<ConversationArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .assistant(&app)?
                .get_conversation(&args.conversation_id)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_delete_conversation(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "assistant_delete_conversation")
        .and_then(|_| parse::<ConversationArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            state
                .assistant(&app)?
                .delete_conversation(&args.conversation_id)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn assistant_clear_conversations(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "assistant_clear_conversations")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            state.assistant(&app)?.clear_conversations().await
        }
        .await,
    ))
}
