//! Desktop IPC commands for global email signatures and signature inline images.
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use mailx_core::{
    ApiResult, AppError, ApplyDraftSignatureRequest, Draft, ErrorCode, MAX_SIGNATURE_IMAGE_BYTES,
    MailxCore, SaveSignatureRequest, SetAccountSignatureRequest, SetDefaultSignatureRequest,
    SignaturesState, StagedSignatureImage,
};
use serde::Deserialize;
use tauri::http::HeaderMap;
use tauri::ipc::{InvokeBody, Response};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewWindow};

use crate::commands::{authorize, parse, respond};
use crate::state::DesktopState;
use crate::window_access::{WindowKind, url_allowed};

pub(crate) const SIGNATURES_EVENT: &str = "mailx://signatures-changed";
const FILE_NAME_HEADER: &str = "x-mailx-file-name";
const MAX_STAGED_PREVIEWS: usize = 64;
const MAX_PICKED_IMAGES: usize = 10;

#[derive(Default)]
struct PreviewCache {
    bytes: HashMap<String, Vec<u8>>,
    order: VecDeque<String>,
}

/// In-memory cache of validated preview bytes for newly staged signature images
/// not yet saved to SQLite. Bounded in size so aborted edits never leak memory.
#[derive(Default)]
pub(crate) struct StagedSignaturePreviews {
    inner: Mutex<PreviewCache>,
}

impl StagedSignaturePreviews {
    fn put(&self, id: String, bytes: Vec<u8>) {
        if let Ok(mut guard) = self.inner.lock() {
            if !guard.bytes.contains_key(&id) {
                guard.order.push_back(id.clone());
            }
            guard.bytes.insert(id, bytes);
            while guard.bytes.len() > MAX_STAGED_PREVIEWS {
                if let Some(oldest) = guard.order.pop_front() {
                    guard.bytes.remove(&oldest);
                } else {
                    break;
                }
            }
        }
    }

    fn get(&self, id: &str) -> Option<Vec<u8>> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.bytes.get(id).cloned())
    }
}

fn emit_signatures_changed(app: &AppHandle, state: &SignaturesState) {
    for (label, window) in app.webview_windows() {
        if WindowKind::parse(&label).is_some() && window.url().is_ok_and(|url| url_allowed(&url)) {
            let _ = app.emit_to(
                EventTarget::webview_window(label),
                SIGNATURES_EVENT,
                state.clone(),
            );
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestArgs<T> {
    request: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewArgs {
    signature_id: Option<String>,
    image_id: String,
}

#[tauri::command]
pub(crate) async fn signatures_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ApiResult<SignaturesState>, ()> {
    let result = match authorize(&window, "signatures_list") {
        Ok(_) => state.run(|core| core.signatures_state()).await,
        Err(error) => Err(error),
    };
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn signatures_save(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SignaturesState>, ()> {
    let parsed = authorize(&window, "signatures_save")
        .and_then(|_| parse::<RequestArgs<SaveSignatureRequest>>(request.body()));
    let result = match parsed {
        Ok(args) => {
            state
                .run(move |core| core.save_signature(args.request))
                .await
        }
        Err(error) => Err(error),
    };
    if let Ok(ref updated) = result {
        emit_signatures_changed(&app, updated);
    }
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn signatures_delete(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SignaturesState>, ()> {
    let parsed =
        authorize(&window, "signatures_delete").and_then(|_| parse::<IdArgs>(request.body()));
    let result = match parsed {
        Ok(args) => state.run(move |core| core.delete_signature(&args.id)).await,
        Err(error) => Err(error),
    };
    if let Ok(ref updated) = result {
        emit_signatures_changed(&app, updated);
    }
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn signatures_set_default(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SignaturesState>, ()> {
    let parsed = authorize(&window, "signatures_set_default")
        .and_then(|_| parse::<RequestArgs<SetDefaultSignatureRequest>>(request.body()));
    let result = match parsed {
        Ok(args) => {
            state
                .run(move |core| core.set_default_signature(args.request))
                .await
        }
        Err(error) => Err(error),
    };
    if let Ok(ref updated) = result {
        emit_signatures_changed(&app, updated);
    }
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn signatures_set_account_default(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<SignaturesState>, ()> {
    let parsed = authorize(&window, "signatures_set_account_default")
        .and_then(|_| parse::<RequestArgs<SetAccountSignatureRequest>>(request.body()));
    let result = match parsed {
        Ok(args) => {
            state
                .run(move |core| core.set_account_default_signature(args.request))
                .await
        }
        Err(error) => Err(error),
    };
    if let Ok(ref updated) = result {
        emit_signatures_changed(&app, updated);
    }
    Ok(respond(result))
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<(String, Vec<u8>), AppError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::validation("fileName"))?
        .to_owned();
    let meta = std::fs::metadata(path).map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    if !meta.is_file() {
        return Err(AppError::new(ErrorCode::UnsupportedImage));
    }
    if meta.len() > max_bytes {
        return Err(AppError::new(ErrorCode::AttachmentTooLarge));
    }
    let bytes = std::fs::read(path).map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    if bytes.len() as u64 > max_bytes {
        return Err(AppError::new(ErrorCode::AttachmentTooLarge));
    }
    Ok((name, bytes))
}

fn stage_from_paths(
    core: &MailxCore,
    paths: &[std::path::PathBuf],
) -> Result<Vec<(StagedSignatureImage, Vec<u8>)>, AppError> {
    let mut staged = Vec::with_capacity(paths.len());
    for path in paths {
        let (name, bytes) = read_bounded(path, MAX_SIGNATURE_IMAGE_BYTES)?;
        let (meta, _mime, preview) = core.prepare_signature_image(&name, &bytes)?;
        staged.push((meta, preview));
    }
    Ok(staged)
}

#[tauri::command]
pub(crate) async fn signatures_add_image(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    previews: State<'_, StagedSignaturePreviews>,
) -> Result<ApiResult<Option<Vec<StagedSignatureImage>>>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let auth = authorize(&window, "signatures_add_image");
    let result = async {
        auth?;
        let core = state.core()?;
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        app.dialog()
            .file()
            .set_parent(&window)
            .set_title("插入图片")
            .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_files(move |paths| {
                let _ = reply.send(paths);
            });
        let picked = tauri::async_runtime::spawn_blocking(move || answer.recv().ok().flatten())
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let Some(picked) = picked else {
            return Ok(None);
        };
        let paths = picked
            .into_iter()
            .take(MAX_PICKED_IMAGES)
            .map(|path| path.into_path())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        let items = tauri::async_runtime::spawn_blocking(move || stage_from_paths(&core, &paths))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let mut out = Vec::with_capacity(items.len());
        for (meta, preview) in items {
            previews.put(meta.id.clone(), preview);
            out.push(meta);
        }
        Ok(Some(out))
    }
    .await;
    Ok(respond(result))
}

fn file_name_header(headers: &HeaderMap) -> Result<String, AppError> {
    let raw = headers
        .get(FILE_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::validation("request"))?;
    let name = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| AppError::validation("request"))?
        .into_owned();
    if name.is_empty() {
        return Err(AppError::validation("request"));
    }
    Ok(name)
}

#[tauri::command]
pub(crate) async fn signatures_add_image_bytes(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    previews: State<'_, StagedSignaturePreviews>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<StagedSignatureImage>, ()> {
    let parsed = authorize(&window, "signatures_add_image_bytes").and_then(|_| {
        let name = file_name_header(request.headers())?;
        let InvokeBody::Raw(bytes) = request.body() else {
            return Err(AppError::validation("request"));
        };
        if bytes.len() as u64 > MAX_SIGNATURE_IMAGE_BYTES {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        Ok((name, bytes.clone()))
    });
    let result = match parsed {
        Ok((name, bytes)) => state
            .run(move |core| core.prepare_signature_image(&name, &bytes))
            .await
            .map(|(meta, _mime, preview)| {
                previews.put(meta.id.clone(), preview);
                meta
            }),
        Err(error) => Err(error),
    };
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn signatures_image_preview(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    previews: State<'_, StagedSignaturePreviews>,
    request: tauri::ipc::Request<'_>,
) -> Result<Response, AppError> {
    authorize(&window, "signatures_image_preview")?;
    let args = parse::<PreviewArgs>(request.body())?;
    if let Some(bytes) = previews.get(&args.image_id) {
        return Ok(Response::new(bytes));
    }
    let Some(signature_id) = args.signature_id else {
        return Err(AppError::new(ErrorCode::NotFound));
    };
    let (_mime, bytes) = state
        .run(move |core| core.signature_image_preview(&signature_id, &args.image_id))
        .await?;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub(crate) async fn drafts_apply_signature(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let parsed = authorize(&window, "drafts_apply_signature").and_then(|kind| {
        let args = parse::<RequestArgs<ApplyDraftSignatureRequest>>(request.body())?;
        crate::window_access::check_draft(kind, "drafts_apply_signature", &args.request.draft_id)?;
        Ok(args)
    });
    let result = match parsed {
        Ok(args) => {
            state
                .run(move |core| core.draft_apply_signature(args.request))
                .await
        }
        Err(error) => Err(error),
    };
    Ok(respond(result))
}
