//! Writing help in the composer (`docs/architecture/ai-chat-ui.md` section
//! 5.3). Open to composer windows and the main window's inline composer, and
//! narrower than the assistant's commands: a composer window may only start
//! a compose run for its own draft, and only follow or stop compose runs of
//! that draft. It gets no conversations, confirmations or undo. The run lives
//! in `mailx_core::Assistant`; its events go to the window that has the draft
//! (`main.rs`).
use mailx_core::{
    AiConsent, ApiResult, AppError, ComposeAssistState, ComposeTask, RunKind, RunSnapshot, StartRun,
};
use serde::Deserialize;
use tauri::{State, WebviewWindow};

use crate::commands::{EmptyArgs, authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_access::{WindowKind, check_draft, parse_draft_id};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComposeRequest {
    draft_id: String,
    task: ComposeTask,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    request: ComposeRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunArgs {
    run_id: String,
}

/// A compose run this window may follow: any for the main window, its own
/// draft's for a composer. Anything else reads as unknown.
fn owned_run(kind: WindowKind, run_draft: Option<String>) -> Result<(), AppError> {
    let own = match (kind, run_draft) {
        (_, None) => false,
        (WindowKind::Main, Some(_)) => true,
        (WindowKind::Composer(id), Some(draft)) => parse_draft_id(&draft) == Some(id),
    };
    if own {
        Ok(())
    } else {
        Err(AppError::new(mailx_core::ErrorCode::NotFound))
    }
}

#[tauri::command]
pub(crate) async fn compose_assist_state(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<ComposeAssistState>, ()> {
    let args =
        authorize(&window, "compose_assist_state").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            state.assistant(&app)?.compose_state().await
        }
        .await,
    ))
}

/// The same consent as the assistant's own command: it binds the connection,
/// not the window that asked.
#[tauri::command]
pub(crate) async fn compose_assist_grant_consent(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<AiConsent>, ()> {
    let args = authorize(&window, "compose_assist_grant_consent")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(
        async {
            args?;
            state.assistant(&app)?.grant_consent().await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn compose_assist_start(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<RunSnapshot>, ()> {
    let args = authorize(&window, "compose_assist_start").and_then(|kind| {
        let args = parse::<StartArgs>(request.body())?;
        check_draft(kind, "compose_assist_start", &args.request.draft_id)?;
        Ok(args)
    });
    Ok(respond(
        async {
            let ComposeRequest { draft_id, task } = args?.request;
            state
                .assistant(&app)?
                .start(StartRun {
                    kind: RunKind::ComposeAssist { draft_id, task },
                })
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn compose_assist_cancel(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "compose_assist_cancel")
        .and_then(|kind| Ok((kind, parse::<RunArgs>(request.body())?)));
    Ok(respond(
        async {
            let (kind, args) = args?;
            let assistant = state.assistant(&app)?;
            owned_run(kind, assistant.compose_run_draft(&args.run_id))?;
            assistant.cancel(&args.run_id)
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn compose_assist_snapshot(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<RunSnapshot>, ()> {
    let args = authorize(&window, "compose_assist_snapshot")
        .and_then(|kind| Ok((kind, parse::<RunArgs>(request.body())?)));
    Ok(respond(
        async {
            let (kind, args) = args?;
            let assistant = state.assistant(&app)?;
            owned_run(kind, assistant.compose_run_draft(&args.run_id))?;
            assistant.snapshot(&args.run_id)
        }
        .await,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_composer_follows_only_its_own_drafts_compose_runs() {
        let own = Uuid::new_v4();
        let other = Uuid::new_v4().hyphenated().to_string();
        let composer = WindowKind::Composer(own);
        assert!(owned_run(composer, Some(own.hyphenated().to_string())).is_ok());
        assert!(owned_run(composer, Some(other.clone())).is_err());
        // A chat or summary run has no draft: not a compose run at all.
        assert!(owned_run(composer, None).is_err());
        assert!(owned_run(WindowKind::Main, Some(other)).is_ok());
        assert!(owned_run(WindowKind::Main, None).is_err());
    }
}
