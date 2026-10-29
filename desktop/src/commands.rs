use mailx_core::{
    Account, AccountConnectionVerification, AccountRemoval, AccountSyncState, ApiResult, AppError,
    CancelConnectionVerification, CancelDraftVerification, CreateAccountRequest,
    DraftConnectionVerification, EmailDetail, EmailListQuery, EmailPage, ErrorCode, Folder,
    RemoveAccountRequest, SettingsPatch, SettingsSnapshot, UpdateConnectionRequest,
    UpdateProfileRequest, VerifyAccountConnection, VerifyDraftConnection,
};
use serde::{Deserialize, de::DeserializeOwned};
use tauri::{Emitter, EventTarget, Manager, State, WebviewWindow, ipc::InvokeBody};

use crate::state::{DesktopState, StartupStatus};
use crate::window_access::{self, WindowKind};
use crate::window_chrome::{self, WindowAction, WindowChrome, WindowTheme};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetArgs {
    account_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs<T> {
    request: T,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailListArgs {
    query: EmailListQuery,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailGetArgs {
    email_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkOpenArgs {
    email_id: String,
    index: usize,
    document_token: String,
    request_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BodyLinksArgs {
    email_id: String,
    document_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkRequestArgs {
    email_id: String,
    index: usize,
    document_token: String,
}

pub(crate) fn parse<T: DeserializeOwned>(body: &InvokeBody) -> Result<T, AppError> {
    match body {
        InvokeBody::Json(value) => {
            serde_json::from_value(value.clone()).map_err(|_| AppError::validation("request"))
        }
        InvokeBody::Raw(_) => Err(AppError::validation("request")),
    }
}

/// Checks the calling window against the per-command whitelist in
/// `window_access`. The caller cannot supply its own principal, data
/// directory, SQL or credential reference.
pub(crate) fn authorize(window: &WebviewWindow, command: &str) -> Result<WindowKind, AppError> {
    let url = window.url().map_err(|_| AppError::validation("caller"))?;
    window_access::check_kind(window.label(), &url, command)
}

pub(crate) fn respond<T>(result: Result<T, AppError>) -> ApiResult<T> {
    ApiResult::from_result(result, uuid::Uuid::new_v4().to_string())
}

#[tauri::command]
pub(crate) async fn app_get_startup_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<StartupStatus>, ()> {
    let args = authorize(&window, "app_get_startup_status")
        .and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => Ok(state.startup_status()),
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn app_retry_startup(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<StartupStatus>, ()> {
    let args =
        authorize(&window, "app_retry_startup").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => Ok(state.retry_startup()),
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<Account>>, ()> {
    let args = authorize(&window, "accounts_list").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => state.run(|core| core.list_accounts()).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Account>, ()> {
    let args = authorize(&window, "accounts_get").and_then(|_| parse::<GetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.get_account(&args.account_id))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_create(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Account>, ()> {
    let args = authorize(&window, "accounts_create")
        .and_then(|_| parse::<WriteArgs<CreateAccountRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.create_account(args.request))
                .await
        }
        Err(error) => Err(error),
    }))
}

/// Removes this account's local data after the revision and sending checks.
#[tauri::command]
pub(crate) async fn accounts_remove(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<AccountRemoval>, ()> {
    let args = authorize(&window, "accounts_remove")
        .and_then(|_| parse::<WriteArgs<RemoveAccountRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            let result = state
                .run(move |core| core.remove_account(args.request))
                .await;
            if let Ok(removed) = &result {
                protocol.revoke_account(&removed.account_id);
            }
            result
        }
        Err(error) => Err(error),
    }))
}

/// Replaces the connection; the account's sync restarts immediately.
#[tauri::command]
pub(crate) async fn accounts_update_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Account>, ()> {
    let args = authorize(&window, "accounts_update_connection")
        .and_then(|_| parse::<WriteArgs<UpdateConnectionRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            let result = state
                .run(move |core| core.update_account_connection(args.request))
                .await;
            if let Ok(account) = &result {
                protocol.revoke_account(&account.id);
            }
            result
        }
        Err(error) => Err(error),
    }))
}

/// Authentication-only checks for the selected saved connection.
#[tauri::command]
pub(crate) async fn accounts_verify_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<AccountConnectionVerification>, ()> {
    let args = authorize(&window, "accounts_verify_connection")
        .and_then(|_| parse::<WriteArgs<VerifyAccountConnection>>(request.body()));
    Ok(respond(match args {
        Ok(args) => match state.core() {
            Ok(core) => core.verify_account_connection(args.request).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_cancel_verification(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "accounts_cancel_verification")
        .and_then(|_| parse::<WriteArgs<CancelConnectionVerification>>(request.body()));
    Ok(respond(args.and_then(|args| {
        state
            .core()?
            .cancel_account_connection_verification(args.request)
    })))
}

/// Authentication-only checks for the unsaved add-account form.
#[tauri::command]
pub(crate) async fn accounts_verify_draft_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<DraftConnectionVerification>, ()> {
    let args = authorize(&window, "accounts_verify_draft_connection")
        .and_then(|_| parse::<WriteArgs<VerifyDraftConnection>>(request.body()));
    Ok(respond(match args {
        Ok(args) => match state.core() {
            Ok(core) => core.verify_draft_connection(args.request).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_cancel_draft_verification(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "accounts_cancel_draft_verification")
        .and_then(|_| parse::<WriteArgs<CancelDraftVerification>>(request.body()));
    Ok(respond(args.and_then(|args| {
        state.core()?.cancel_draft_verification(args.request)
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderHelpArgs {
    provider_id: String,
}

fn provider_help_url(provider_id: &str) -> Result<&'static str, AppError> {
    match provider_id {
        "qq" => Ok("https://help.mail.qq.com/detail/0/1087"),
        "netease163" | "netease126" | "neteaseYeah" => Ok(
            "https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2a5feb28b66796d3b",
        ),
        _ => Err(AppError::validation("providerId")),
    }
}

#[tauri::command]
pub(crate) async fn accounts_open_provider_help(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    use tauri_plugin_opener::OpenerExt;
    let args = authorize(&window, "accounts_open_provider_help")
        .and_then(|_| parse::<ProviderHelpArgs>(request.body()));
    Ok(respond(args.and_then(|args| {
        window
            .opener()
            .open_url(provider_help_url(&args.provider_id)?, None::<&str>)
            .map_err(|_| AppError::new(ErrorCode::Internal))
    })))
}

/// Renames or recolors an account without touching its connection or sync.
#[tauri::command]
pub(crate) async fn accounts_update_profile(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Account>, ()> {
    let args = authorize(&window, "accounts_update_profile")
        .and_then(|_| parse::<WriteArgs<UpdateProfileRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.update_account_profile(args.request))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn folders_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<Folder>>, ()> {
    let args = authorize(&window, "folders_list").and_then(|_| parse::<GetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.list_folders(&args.account_id))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<EmailPage>, ()> {
    let args =
        authorize(&window, "emails_list").and_then(|_| parse::<EmailListArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.list_emails(&args.query)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<EmailDetail>, ()> {
    let args = authorize(&window, "emails_get").and_then(|_| parse::<EmailGetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => state.run(move |core| core.get_email(&args.email_id)).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_fetch_body(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<EmailDetail>, ()> {
    let args =
        authorize(&window, "emails_fetch_body").and_then(|_| parse::<EmailGetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => match state.core() {
            Ok(core) => core.fetch_body(args.email_id).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareBodyArgs {
    email_id: String,
    #[serde(default)]
    load_remote_images: bool,
}

#[tauri::command]
pub(crate) async fn emails_prepare_body(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<crate::body_protocol::PreparedBody>, ()> {
    let args = authorize(&window, "emails_prepare_body")
        .and_then(|_| parse::<PrepareBodyArgs>(request.body()));
    let result = async {
        let args = args?;
        let policy = state
            .settings(|store| store.get())
            .await?
            .settings
            .security
            .remote_images;
        let allow = match policy {
            mailx_core::RemoteImagePolicy::Automatic => true,
            mailx_core::RemoteImagePolicy::Ask => args.load_remote_images,
            mailx_core::RemoteImagePolicy::Never => false,
        };
        protocol
            .prepare_with_images(
                &state.core()?,
                args.email_id,
                allow,
                policy == mailx_core::RemoteImagePolicy::Ask && args.load_remote_images,
            )
            .await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn emails_release_body(
    window: WebviewWindow,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    Ok(respond(
        authorize(&window, "emails_release_body")
            .and_then(|_| parse::<BodyLinksArgs>(request.body()))
            .and_then(|args| protocol.release(&args.email_id, &args.document_token)),
    ))
}

#[tauri::command]
pub(crate) async fn emails_image_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<mailx_core::RemoteImageStatus>>, ()> {
    let args = authorize(&window, "emails_image_status")
        .and_then(|_| parse::<BodyLinksArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let policy = state
                .settings(|store| store.get())
                .await?
                .settings
                .security
                .remote_images;
            protocol
                .image_status(&state.core()?, &args.email_id, &args.document_token, policy)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn emails_retry_images(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<crate::body_protocol::PreparedBody>, ()> {
    let args = authorize(&window, "emails_retry_images")
        .and_then(|_| parse::<BodyLinksArgs>(request.body()));
    Ok(respond(
        async {
            let args = args?;
            let policy = state
                .settings(|store| store.get())
                .await?
                .settings
                .security
                .remote_images;
            protocol
                .retry_images(&state.core()?, &args.email_id, &args.document_token, policy)
                .await
        }
        .await,
    ))
}

#[tauri::command]
pub(crate) async fn emails_body_links(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<crate::body_protocol::BodyLinks>, ()> {
    let args = authorize(&window, "emails_body_links")
        .and_then(|_| parse::<BodyLinksArgs>(request.body()));
    let result = async {
        let args = args?;
        protocol
            .list_links(&state.core()?, args.email_id, args.document_token)
            .await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn links_request(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    use tauri::Emitter;
    let args =
        authorize(&window, "links_request").and_then(|_| parse::<LinkRequestArgs>(request.body()));
    let result = async {
        let args = args?;
        let event = protocol
            .request_link(
                &state.core()?,
                args.email_id,
                args.index,
                args.document_token,
            )
            .await?;
        window
            .emit_to(
                tauri::EventTarget::webview_window("main"),
                crate::body_protocol::LINK_EVENT,
                event,
            )
            .map_err(|_| AppError::new(ErrorCode::Internal))
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn links_open(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    use tauri_plugin_opener::OpenerExt;
    let args = authorize(&window, "links_open").and_then(|_| parse::<LinkOpenArgs>(request.body()));
    let result = async {
        let args = args?;
        let core = state.core()?;
        let url = protocol
            .confirm_link(
                &core,
                args.email_id,
                args.index,
                args.document_token,
                args.request_id,
            )
            .await?;
        window
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|_| AppError::new(ErrorCode::Internal))
    }
    .await;
    Ok(respond(result))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachmentArgs {
    email_id: String,
    attachment_id: String,
}

/// Shows the system save dialog and writes the attachment where the user says.
/// The request carries no path, so a compromised page can only ask the user.
#[tauri::command]
pub(crate) async fn attachments_save(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<crate::attachment_save::SaveOutcome>, ()> {
    use crate::attachment_save::{SaveOutcome, write_atomically};
    use tauri_plugin_dialog::DialogExt;
    let args = authorize(&window, "attachments_save")
        .and_then(|_| parse::<AttachmentArgs>(request.body()));
    let result = async {
        let args = args?;
        let core = state.core()?;
        // Reads the local copy; the metadata is checked before any dialog opens.
        let file = core
            .attachment_content(args.email_id, args.attachment_id)
            .await?;
        // The callback form never blocks the event loop; the wait happens on the
        // blocking pool. A dropped sender (the dialog could not open) reads as
        // "cancelled".
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        app.dialog()
            .file()
            .set_parent(&window)
            .set_title("保存附件")
            .set_file_name(&file.info.file_name)
            .set_can_create_directories(true)
            .save_file(move |path| {
                let _ = reply.send(path);
            });
        let picked = tauri::async_runtime::spawn_blocking(move || answer.recv().ok().flatten())
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let Some(picked) = picked else {
            return Ok(SaveOutcome::Cancelled);
        };
        let destination = picked
            .into_path()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        tauri::async_runtime::spawn_blocking(move || write_atomically(&destination, &file.bytes))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
        Ok(SaveOutcome::Saved)
    }
    .await;
    Ok(respond(result))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetFlagArgs {
    email_id: String,
    flag: FlagName,
    wanted: bool,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum FlagName {
    Read,
    Starred,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FlagOutcome {
    Unchanged,
    /// Saved; the server is updated in the background.
    Queued,
    /// Saved on this device only: the server cannot keep a starred flag here.
    LocalOnly,
}

/// Changes read / starred locally at once; the write to the server is queued.
#[tauri::command]
pub(crate) async fn emails_set_flag(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<FlagOutcome>, ()> {
    use mailx_core::{FlagChange, MessageFlag};
    let args =
        authorize(&window, "emails_set_flag").and_then(|_| parse::<SetFlagArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| {
                    let flag = match args.flag {
                        FlagName::Read => MessageFlag::Read,
                        FlagName::Starred => MessageFlag::Starred,
                    };
                    Ok(
                        match core.set_message_flag(&args.email_id, flag, args.wanted)? {
                            FlagChange::Unchanged => FlagOutcome::Unchanged,
                            FlagChange::Queued => FlagOutcome::Queued,
                            FlagChange::LocalOnly => FlagOutcome::LocalOnly,
                        },
                    )
                })
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn sync_refresh(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args = authorize(&window, "sync_refresh").and_then(|_| parse::<GetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.refresh_sync(&args.account_id))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn image_cache_clear(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    let args =
        authorize(&window, "image_cache_clear").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => {
            state
                .run(|core| {
                    core.clear_image_cache();
                    Ok(())
                })
                .await
        }
        Err(error) => Err(error),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AvatarArgs {
    account_id: String,
    address: String,
}

/// No lookup, and so nothing sent to a third party, unless the user opted in.
#[tauri::command]
pub(crate) async fn avatars_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Option<mailx_core::AvatarImage>>, ()> {
    let args = authorize(&window, "avatars_get").and_then(|_| parse::<AvatarArgs>(request.body()));
    let result = async {
        let args = args?;
        if !state
            .settings(|store| store.get())
            .await?
            .settings
            .security
            .sender_avatars
        {
            return Ok(None);
        }
        state
            .core()?
            .sender_avatar(args.account_id, args.address)
            .await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn sync_states(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<AccountSyncState>>, ()> {
    let args = authorize(&window, "sync_states").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => state.run(|core| Ok(core.sync_states())).await,
        Err(error) => Err(error),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowPerformArgs {
    action: WindowAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowSetThemeArgs {
    theme: WindowTheme,
}

#[tauri::command]
pub(crate) async fn settings_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SettingsSnapshot>, ()> {
    let args = authorize(&window, "settings_get").and_then(|_| parse::<EmptyArgs>(request.body()));
    Ok(respond(match args {
        Ok(_) => state.settings(|store| store.get()).await,
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn settings_update(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SettingsSnapshot>, ()> {
    let args = authorize(&window, "settings_update")
        .and_then(|_| parse::<WriteArgs<SettingsPatch>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            let changes_security = args.request.security.is_some();
            let result = state.settings(move |store| store.patch(args.request)).await;
            if changes_security
                && result.as_ref().is_ok_and(|s| {
                    s.settings.security.remote_images != mailx_core::RemoteImagePolicy::Automatic
                })
            {
                protocol.revoke_remote_images();
            }
            if let Ok(snapshot) = &result {
                emit_settings_changed(&app, snapshot);
                crate::lifecycle::settings_changed(&app, snapshot);
            }
            result
        }
        Err(error) => Err(error),
    }))
}

pub(crate) fn emit_settings_changed(app: &tauri::AppHandle, snapshot: &SettingsSnapshot) {
    let change = mailx_core::SettingsChanged {
        revision: snapshot.revision,
    };
    for (label, target) in app.webview_windows() {
        if target
            .url()
            .ok()
            .is_some_and(|url| window_access::check_kind(&label, &url, "settings_get").is_ok())
        {
            let _ = app.emit_to(
                EventTarget::webview_window(label),
                "mailx://settings-changed",
                change,
            );
        }
    }
}

// Window commands stay synchronous so they run on the main thread, which
// GTK settings and native window calls require.
#[tauri::command]
pub(crate) fn window_get_chrome(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<WindowChrome> {
    let args =
        authorize(&window, "window_get_chrome").and_then(|_| parse::<EmptyArgs>(request.body()));
    respond(args.map(|_| window_chrome::chrome(&window)))
}

#[tauri::command]
pub(crate) fn window_perform(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    let args = authorize(&window, "window_perform")
        .and_then(|_| parse::<WindowPerformArgs>(request.body()));
    respond(args.and_then(|args| {
        window_chrome::perform(&window, args.action).map_err(|_| AppError::new(ErrorCode::Internal))
    }))
}

#[tauri::command]
pub(crate) fn window_set_theme(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    let args = authorize(&window, "window_set_theme")
        .and_then(|_| parse::<WindowSetThemeArgs>(request.body()));
    respond(args.and_then(|args| {
        window_chrome::set_theme(&window, args.theme)
            .map_err(|_| AppError::new(ErrorCode::Internal))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_and_forged_arguments_are_redacted() {
        for body in [
            json!({"accountId": ["synthetic-secret"]}),
            json!({"accountId": "id", "actor": "synthetic-secret"}),
            json!({"accountId": "id", "secretRef": "synthetic-secret"}),
        ] {
            let error = parse::<GetArgs>(&InvokeBody::Json(body)).err().unwrap();
            assert_eq!(error, AppError::validation("request"));
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("synthetic-secret")
            );
        }
        assert!(parse::<EmptyArgs>(&InvokeBody::Json(json!({"actor":"admin"}))).is_err());
        assert!(parse::<EmptyArgs>(&InvokeBody::Raw(vec![1, 2])).is_err());
    }

    #[test]
    fn verification_and_help_reject_arbitrary_targets_and_secret_arguments() {
        let args = json!({"request": {"accountId":"account", "expectedRevision":1, "verificationId":"attempt"}});
        assert!(parse::<WriteArgs<VerifyAccountConnection>>(&InvokeBody::Json(args)).is_ok());
        let forged = json!({"request": {"accountId":"account", "expectedRevision":1, "verificationId":"attempt", "password":"secret", "host":"attacker"}});
        assert!(parse::<WriteArgs<VerifyAccountConnection>>(&InvokeBody::Json(forged)).is_err());
        assert_eq!(
            provider_help_url("qq").unwrap(),
            "https://help.mail.qq.com/detail/0/1087"
        );
        for provider in ["netease163", "netease126", "neteaseYeah"] {
            assert_eq!(
                provider_help_url(provider).unwrap(),
                "https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2a5feb28b66796d3b"
            );
        }
        for provider in [
            "https://attacker.invalid",
            "foxmail",
            "qq/../",
            "",
            "netease",
            "netease163/../",
            "NETEASE126",
            "yeah",
        ] {
            assert_eq!(
                provider_help_url(provider).unwrap_err(),
                AppError::validation("providerId")
            );
        }
        assert!(
            parse::<ProviderHelpArgs>(&InvokeBody::Json(
                json!({"providerId":"qq", "url":"https://attacker.invalid"})
            ))
            .is_err()
        );
    }

    #[test]
    fn draft_verification_accepts_only_the_supplied_connection_and_attempt_id() {
        let connection = json!({
            "emailAddress":"fixture@example.test", "providerId":"neteaseYeah",
            "imap":{"host":"imap.yeah.net","port":993,"tls":"implicit","username":"fixture@example.test"},
            "smtp":{"host":"smtp.yeah.net","port":465,"tls":"implicit","username":"fixture@example.test"},
            "authentication":{"kind":"password","mode":"shared","password":{"action":"replace","value":"synthetic-secret"}},
            "route":{"kind":"direct"}
        });
        let args = json!({"request":{"verificationId":"attempt", "connection":connection}});
        assert!(parse::<WriteArgs<VerifyDraftConnection>>(&InvokeBody::Json(args.clone())).is_ok());
        for extra in ["accountId", "secretRef", "actor"] {
            let mut forged = args.clone();
            forged["request"][extra] = json!("synthetic-secret");
            let error = parse::<WriteArgs<VerifyDraftConnection>>(&InvokeBody::Json(forged))
                .err()
                .unwrap();
            assert_eq!(error, AppError::validation("request"));
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("synthetic-secret")
            );
        }
        let mut unknown_connection = args;
        unknown_connection["request"]["connection"]["secretRef"] = json!("synthetic-secret");
        assert!(
            parse::<WriteArgs<VerifyDraftConnection>>(&InvokeBody::Json(unknown_connection))
                .is_err()
        );
        let malformed = json!({"request":{"verificationId":"attempt", "connection":{
            "emailAddress":["synthetic-secret"]
        }}});
        assert_eq!(
            parse::<WriteArgs<VerifyDraftConnection>>(&InvokeBody::Json(malformed))
                .err()
                .unwrap(),
            AppError::validation("request")
        );
        assert!(
            parse::<WriteArgs<CancelDraftVerification>>(&InvokeBody::Json(json!({
                "request":{"verificationId":"attempt"}
            })))
            .is_ok()
        );
        for extra in ["accountId", "connection", "password", "secretRef"] {
            let mut args = json!({"request":{"verificationId":"attempt"}});
            args["request"][extra] = json!("synthetic-secret");
            assert!(parse::<WriteArgs<CancelDraftVerification>>(&InvokeBody::Json(args)).is_err());
        }
    }

    #[test]
    fn email_commands_accept_only_their_named_arguments() {
        let list = parse::<EmailListArgs>(&InvokeBody::Json(json!({
            "query": {"folderRole":"inbox", "limit":100, "text":"中文"}
        })))
        .unwrap();
        assert_eq!(list.query.limit, Some(100));
        assert_eq!(list.query.text.as_deref(), Some("中文"));
        let id = uuid::Uuid::new_v4().to_string();
        let detail = parse::<EmailGetArgs>(&InvokeBody::Json(json!({"emailId":id}))).unwrap();
        assert_eq!(detail.email_id, id);
        for body in [
            json!({}),
            json!({"query":null}),
            json!({"query":{}, "actor":"forged"}),
        ] {
            let error = parse::<EmailListArgs>(&InvokeBody::Json(body))
                .err()
                .unwrap();
            assert_eq!(error, AppError::validation("request"));
        }
        assert!(parse::<EmailGetArgs>(&InvokeBody::Json(json!({"accountId":id}))).is_err());
        assert!(
            parse::<EmailGetArgs>(&InvokeBody::Json(json!({"emailId":id,"path":"secret"})))
                .is_err()
        );
        let token = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let link = parse::<LinkOpenArgs>(&InvokeBody::Json(json!({
            "emailId":id,
            "index":0,
            "documentToken":token,
            "requestId":request_id
        })))
        .unwrap();
        assert_eq!(link.email_id, id);
        assert_eq!(link.index, 0);
        assert!(
            parse::<LinkRequestArgs>(&InvokeBody::Json(json!({
                "emailId": id, "index": 0, "documentToken": token
            })))
            .is_ok()
        );
        for body in [
            json!({"emailId":id,"index":0}),
            json!({"emailId":id,"index":0,"documentToken":token,"url":"https://example.invalid"}),
            json!({"emailId":id,"index":-1,"documentToken":token}),
        ] {
            assert!(parse::<LinkRequestArgs>(&InvokeBody::Json(body)).is_err());
        }
        for body in [
            json!({"emailId":id,"index":-1,"documentToken":token,"requestId":request_id}),
            json!({"emailId":id,"index":0,"documentToken":token}),
            json!({"emailId":id,"index":0,"documentToken":token,"requestId":request_id,"url":"https://untrusted.invalid"}),
        ] {
            assert!(parse::<LinkOpenArgs>(&InvokeBody::Json(body)).is_err());
        }
    }
}

/// Shows a sample notification with the saved settings, so the user can see
/// (and the system can ask for permission to show) what new mail will look like.
#[tauri::command]
pub(crate) async fn notifications_test(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    #[cfg(feature = "body-webview-test")]
    {
        let _ = (app, window, state, request);
        Ok(respond(Err(AppError::new(ErrorCode::Unsupported))))
    }
    #[cfg(not(feature = "body-webview-test"))]
    {
        let args = authorize(&window, "notifications_test")
            .and_then(|_| parse::<EmptyArgs>(request.body()));
        Ok(respond(match args {
            Ok(_) => state.settings(|store| store.get()).await.map(|snapshot| {
                crate::notifications::test(
                    &app,
                    &snapshot.settings.notifications,
                    snapshot.effective_locale,
                )
            }),
            Err(error) => Err(error),
        }))
    }
}

#[cfg(not(feature = "body-webview-test"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundArgs {
    sound: mailx_core::NotificationSound,
}

/// Plays one of MailX's own notification sounds; the system sound and "none"
/// play nothing here.
#[tauri::command]
pub(crate) async fn notifications_preview_sound(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<()>, ()> {
    #[cfg(feature = "body-webview-test")]
    {
        let _ = (window, request);
        Ok(respond(Err(AppError::new(ErrorCode::Unsupported))))
    }
    #[cfg(not(feature = "body-webview-test"))]
    {
        let args = authorize(&window, "notifications_preview_sound")
            .and_then(|_| parse::<SoundArgs>(request.body()));
        Ok(respond(
            args.map(|args| crate::notifications::preview(args.sound)),
        ))
    }
}
