use mailx_core::{
    Account, AccountSyncState, ApiResult, AppError, CreateAccountRequest, EmailDetail,
    EmailListQuery, EmailPage, ErrorCode, Folder, UpdateAccountRequest,
};
use serde::{Deserialize, de::DeserializeOwned};
use tauri::{State, WebviewWindow, ipc::InvokeBody};

use crate::state::{DesktopState, StartupStatus};
use crate::window_chrome::{self, WindowAction, WindowChrome};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkRequestArgs {
    email_id: String,
    index: usize,
    document_token: String,
}

fn parse<T: DeserializeOwned>(body: &InvokeBody) -> Result<T, AppError> {
    match body {
        InvokeBody::Json(value) => {
            serde_json::from_value(value.clone()).map_err(|_| AppError::validation("request"))
        }
        InvokeBody::Raw(_) => Err(AppError::validation("request")),
    }
}

fn caller_allowed(label: &str, url: &tauri::Url) -> bool {
    if label != "main" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let bundled = matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
    );
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(5173);
    bundled || development
}

fn authorize(window: &WebviewWindow) -> Result<(), AppError> {
    // Supplements Tauri's local-main-only capability. The caller cannot supply
    // its own principal, data directory, SQL or credential reference.
    if window
        .url()
        .is_ok_and(|url| caller_allowed(window.label(), &url))
    {
        Ok(())
    } else {
        Err(AppError::validation("caller"))
    }
}

fn respond<T>(result: Result<T, AppError>) -> ApiResult<T> {
    ApiResult::from_result(result, uuid::Uuid::new_v4().to_string())
}

#[tauri::command]
pub(crate) async fn app_get_startup_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<StartupStatus>, ()> {
    let args = authorize(&window).and_then(|()| parse::<EmptyArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<EmptyArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<EmptyArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<GetArgs>(request.body()));
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
    let args =
        authorize(&window).and_then(|()| parse::<WriteArgs<CreateAccountRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.create_account(args.request))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn accounts_update(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Account>, ()> {
    let args =
        authorize(&window).and_then(|()| parse::<WriteArgs<UpdateAccountRequest>>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.update_account(args.request))
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
    let args = authorize(&window).and_then(|()| parse::<GetArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<EmailListArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<EmailGetArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<EmailGetArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => match state.core() {
            Ok(core) => core.fetch_body(args.email_id).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn emails_body_links(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    protocol: State<'_, std::sync::Arc<crate::body_protocol::BodyProtocol>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<crate::body_protocol::BodyLinks>, ()> {
    let args = authorize(&window).and_then(|()| parse::<EmailGetArgs>(request.body()));
    let result = async {
        let args = args?;
        protocol.list_links(&state.core()?, args.email_id).await
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
    let args = authorize(&window).and_then(|()| parse::<LinkRequestArgs>(request.body()));
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
    let args = authorize(&window).and_then(|()| parse::<LinkOpenArgs>(request.body()));
    let result = async {
        let args = args?;
        let core = state.core()?;
        let url = protocol
            .confirm_link(&core, args.email_id, args.index)
            .await?;
        window
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|_| AppError::new(ErrorCode::Internal))
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
    let args = authorize(&window).and_then(|()| parse::<EmptyArgs>(request.body()));
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

// Window commands stay synchronous so they run on the main thread, which
// GTK settings and native window calls require.
#[tauri::command]
pub(crate) fn window_get_chrome(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<WindowChrome> {
    let args = authorize(&window).and_then(|()| parse::<EmptyArgs>(request.body()));
    respond(args.map(|_| window_chrome::chrome(&window)))
}

#[tauri::command]
pub(crate) fn window_perform(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    let args = authorize(&window).and_then(|()| parse::<WindowPerformArgs>(request.body()));
    respond(args.and_then(|args| {
        window_chrome::perform(&window, args.action).map_err(|_| AppError::new(ErrorCode::Internal))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_local_main_window_is_authorized() {
        for url in [
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
        ] {
            let url = tauri::Url::parse(url).unwrap();
            assert!(caller_allowed("main", &url));
            assert!(!caller_allowed("mail-body", &url));
        }
        for url in [
            "https://example.com",
            "http://localhost:5174",
            "https://localhost:5173",
            "http://tauri.localhost:9000",
            "mailx-body://localhost/document/example",
            "http://mailx-body.localhost/document/example",
        ] {
            assert!(!caller_allowed("main", &tauri::Url::parse(url).unwrap()));
        }
        assert_eq!(
            caller_allowed("main", &tauri::Url::parse("http://localhost:5173").unwrap()),
            cfg!(debug_assertions)
        );
    }

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
        let link =
            parse::<LinkOpenArgs>(&InvokeBody::Json(json!({"emailId":id,"index":0}))).unwrap();
        assert_eq!(link.email_id, id);
        assert_eq!(link.index, 0);
        let token = uuid::Uuid::new_v4().to_string();
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
            json!({"emailId":id,"index":-1}),
            json!({"emailId":id,"index":0,"url":"https://untrusted.invalid"}),
            json!({"emailId":id,"index":0,"confirmed":true}),
        ] {
            assert!(parse::<LinkOpenArgs>(&InvokeBody::Json(body)).is_err());
        }
    }
}
