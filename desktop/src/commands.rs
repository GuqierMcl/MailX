use mailx_core::{
    Account, ApiResult, AppError, CreateAccountRequest, ErrorCode, UpdateAccountRequest,
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
        Ok(_) => state.run(|service| service.list()).await,
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
                .run(move |service| service.get(&args.account_id))
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
        Ok(args) => state.run(move |service| service.create(args.request)).await,
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
        Ok(args) => state.run(move |service| service.update(args.request)).await,
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
}
