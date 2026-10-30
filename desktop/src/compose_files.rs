//! Attaching local files to a draft.
//!
//! The page never names a path: a picker the host opens, or an OS drop the
//! host receives, supplies the files, and the request only says which draft
//! and revision they belong to. The host reads the bytes and the core stores
//! them, so the composer window needs no file-system permission.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mailx_core::InlineImage;
use mailx_core::{
    AddAttachmentRequest, ApiResult, AppError, AttachmentPolicy, Draft, ErrorCode, MailxCore,
    RemoveAttachmentRequest,
};
use serde::{Deserialize, Serialize};
use tauri::http::HeaderMap;
use tauri::ipc::{InvokeBody, Response};
use tauri::{AppHandle, DragDropEvent, Emitter, EventTarget, Manager, State, WebviewWindow};

use crate::commands::{authorize, parse, respond};
use crate::state::DesktopState;

/// Told to a composer when files were dropped on it; the page answers with
/// `attachments_add_dropped`. The payload is empty on purpose.
pub(crate) const FILES_DROPPED_EVENT: &str = "composer://files-dropped";

/// More than this many paths in one drop or pick is a mistake, not a use case.
const MAX_PATHS: usize = 100;

/// Paths from OS drops, waiting for the page to claim them. Only the latest
/// drop per window is kept.
#[derive(Default)]
pub(crate) struct DroppedFiles {
    pending: Mutex<HashMap<String, Vec<PathBuf>>>,
}

impl DroppedFiles {
    fn put(&self, label: &str, paths: Vec<PathBuf>) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(label.to_owned(), paths);
        }
    }

    fn take(&self, label: &str) -> Vec<PathBuf> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(label))
            .unwrap_or_default()
    }

    pub(crate) fn forget(&self, label: &str) {
        self.take(label);
    }
}

/// Called for every window event of a composer. Only a drop matters.
pub(crate) fn on_drag_drop(app: &AppHandle, label: &str, event: &DragDropEvent) {
    let DragDropEvent::Drop { paths, .. } = event else {
        return;
    };
    let Some(dropped) = app.try_state::<DroppedFiles>() else {
        return;
    };
    let mut paths = paths.clone();
    paths.truncate(MAX_PATHS);
    if paths.is_empty() {
        return;
    }
    dropped.put(label, paths);
    let _ = app.emit_to(EventTarget::webview_window(label), FILES_DROPPED_EVENT, ());
}

// ---------------------------------------------------------------- commands

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestArgs<T> {
    request: T,
}

/// What the page may say about adding: which draft, at which revision.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddFilesRequest {
    draft_id: String,
    expected_revision: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyArgs {
    account_id: String,
}

/// Opens the native multi-select picker and attaches what the user chose.
/// `None` means the picker was cancelled.
#[tauri::command]
pub(crate) async fn attachments_add(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Option<Draft>>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let args = authorize(&window, "attachments_add").and_then(|kind| {
        let args = parse::<RequestArgs<AddFilesRequest>>(request.body())?;
        crate::window_access::check_draft(kind, "attachments_add", &args.request.draft_id)?;
        Ok(args)
    });
    let result = async {
        let args = args?.request;
        let core = state.core()?;
        // The callback form never blocks the event loop; the wait happens on
        // the blocking pool. A dropped sender reads as "cancelled".
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        app.dialog()
            .file()
            .set_parent(&window)
            .set_title("添加附件")
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
            .take(MAX_PATHS)
            .map(|path| path.into_path())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        attach_files(core, args, paths).await
    }
    .await;
    Ok(respond(result))
}

/// Attaches the files the OS dropped on this composer since the page was told.
/// `None` means there was nothing waiting.
#[tauri::command]
pub(crate) async fn attachments_add_dropped(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Option<Draft>>, ()> {
    let args = authorize(&window, "attachments_add_dropped").and_then(|kind| {
        let args = parse::<RequestArgs<AddFilesRequest>>(request.body())?;
        crate::window_access::check_draft(kind, "attachments_add_dropped", &args.request.draft_id)?;
        Ok(args)
    });
    let result = async {
        let args = args?.request;
        let core = state.core()?;
        let paths = app
            .try_state::<DroppedFiles>()
            .map(|dropped| dropped.take(window.label()))
            .unwrap_or_default();
        if paths.is_empty() {
            return Ok(None);
        }
        attach_files(core, args, paths).await
    }
    .await;
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn attachments_remove(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Draft>, ()> {
    let args = authorize(&window, "attachments_remove").and_then(|kind| {
        let args = parse::<RequestArgs<RemoveAttachmentRequest>>(request.body())?;
        crate::window_access::check_draft(kind, "attachments_remove", &args.request.draft_id)?;
        Ok(args)
    });
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.remove_attachment(args.request))
                .await
        }
        Err(error) => Err(error),
    }))
}

#[tauri::command]
pub(crate) async fn attachments_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<AttachmentPolicy>, ()> {
    let args =
        authorize(&window, "attachments_policy").and_then(|_| parse::<PolicyArgs>(request.body()));
    Ok(respond(match args {
        Ok(args) => {
            state
                .run(move |core| core.attachment_policy(&args.account_id))
                .await
        }
        Err(error) => Err(error),
    }))
}

// ------------------------------------------------------------ inline images

/// Header names of `inline_images_add_bytes`; the body is the image itself.
const DRAFT_HEADER: &str = "x-mailx-draft-id";
const REVISION_HEADER: &str = "x-mailx-expected-revision";
const FILE_NAME_HEADER: &str = "x-mailx-file-name";

/// The draft after the images were stored, and the `cid:` handles to insert.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InlineImagesAdded {
    draft: Draft,
    inserted: Vec<InlineImage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewArgs {
    draft_id: String,
    attachment_id: String,
}

/// Opens the native image picker and stores the chosen pictures as inline
/// images in one step. `None` means the picker was cancelled.
#[tauri::command]
pub(crate) async fn inline_images_add(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Option<InlineImagesAdded>>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let args = authorize(&window, "inline_images_add").and_then(|kind| {
        let args = parse::<RequestArgs<AddFilesRequest>>(request.body())?;
        crate::window_access::check_draft(kind, "inline_images_add", &args.request.draft_id)?;
        Ok(args)
    });
    let result = async {
        let args = args?.request;
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
            .take(MAX_PATHS)
            .map(|path| path.into_path())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        let added =
            tauri::async_runtime::spawn_blocking(move || insert_from_paths(&core, args, &paths))
                .await
                .map_err(|_| AppError::new(ErrorCode::Internal))??;
        Ok(Some(added))
    }
    .await;
    Ok(respond(result))
}

/// Stores one image whose bytes the page holds (paste, or a file it already
/// read). The request body is the raw image; see the header constants.
#[tauri::command]
pub(crate) async fn inline_images_add_bytes(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<InlineImagesAdded>, ()> {
    let args = authorize(&window, "inline_images_add_bytes").and_then(|kind| {
        let (args, file_name) = raw_request(request.headers())?;
        crate::window_access::check_draft(kind, "inline_images_add_bytes", &args.draft_id)?;
        let InvokeBody::Raw(bytes) = request.body() else {
            return Err(AppError::validation("request"));
        };
        Ok((args, file_name, bytes.clone()))
    });
    Ok(respond(match args {
        Ok((args, file_name, bytes)) => {
            state
                .run(move |core| {
                    let draft = core.get_draft(&args.draft_id)?;
                    let limit = core.attachment_policy(&draft.account_id)?.max_total_bytes;
                    if bytes.len() as u64 > limit {
                        return Err(AppError::new(ErrorCode::AttachmentTooLarge));
                    }
                    add_inline(core, &args, vec![(file_name, bytes)])
                })
                .await
        }
        Err(error) => Err(error),
    }))
}

/// The transcoded preview of an inline image, as raw bytes. A failure rejects
/// the call with the serialized `AppError`.
#[tauri::command]
pub(crate) async fn inline_image_preview(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<Response, AppError> {
    let kind = authorize(&window, "inline_image_preview")?;
    let args = parse::<PreviewArgs>(request.body())?;
    crate::window_access::check_draft(kind, "inline_image_preview", &args.draft_id)?;
    let (_media_type, bytes) = state
        .run(move |core| core.inline_image_preview(&args.draft_id, &args.attachment_id))
        .await?;
    Ok(Response::new(bytes))
}

/// Reads the draft, revision and file name the page put in the headers.
fn raw_request(headers: &HeaderMap) -> Result<(AddFilesRequest, String), AppError> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| AppError::validation("request"))
    };
    let draft_id = text(DRAFT_HEADER)?.to_owned();
    let expected_revision = text(REVISION_HEADER)?
        .parse::<u32>()
        .map_err(|_| AppError::validation("request"))?;
    let file_name = percent_encoding::percent_decode_str(text(FILE_NAME_HEADER)?)
        .decode_utf8()
        .map_err(|_| AppError::validation("request"))?
        .into_owned();
    if file_name.is_empty() {
        return Err(AppError::validation("request"));
    }
    Ok((
        AddFilesRequest {
            draft_id,
            expected_revision,
        },
        file_name,
    ))
}

fn insert_from_paths(
    core: &MailxCore,
    request: AddFilesRequest,
    paths: &[PathBuf],
) -> Result<InlineImagesAdded, AppError> {
    if paths.is_empty() {
        return Err(AppError::validation("files"));
    }
    let draft = core.get_draft(&request.draft_id)?;
    let limit = core.attachment_policy(&draft.account_id)?.max_total_bytes;
    let files = paths
        .iter()
        .map(|path| read_file(path, limit))
        .collect::<Result<Vec<_>, _>>()?;
    add_inline(core, &request, files)
}

fn add_inline(
    core: &MailxCore,
    request: &AddFilesRequest,
    files: Vec<(String, Vec<u8>)>,
) -> Result<InlineImagesAdded, AppError> {
    let (draft, inserted) =
        core.add_inline_images(&request.draft_id, request.expected_revision, files)?;
    Ok(InlineImagesAdded { draft, inserted })
}

async fn attach_files(
    core: MailxCore,
    request: AddFilesRequest,
    paths: Vec<PathBuf>,
) -> Result<Option<Draft>, AppError> {
    let draft = tauri::async_runtime::spawn_blocking(move || add_all(&core, request, &paths))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
    Ok(Some(draft))
}

/// Adds the files one by one, each against the revision the previous one
/// produced. The first failure stops the rest; what was already added stays
/// (the page reloads the draft after an error).
fn add_all(
    core: &MailxCore,
    request: AddFilesRequest,
    paths: &[PathBuf],
) -> Result<Draft, AppError> {
    let draft = core.get_draft(&request.draft_id)?;
    let limit = core.attachment_policy(&draft.account_id)?.max_total_bytes;
    let mut revision = request.expected_revision;
    let mut latest = None;
    for path in paths {
        let (file_name, bytes) = read_file(path, limit)?;
        let added = core.add_attachment(
            AddAttachmentRequest {
                draft_id: request.draft_id.clone(),
                expected_revision: revision,
                file_name,
            },
            &bytes,
        )?;
        revision = added.revision;
        latest = Some(added);
    }
    match latest {
        Some(draft) => Ok(draft),
        None => Err(AppError::validation("files")),
    }
}

/// Reads a regular file whole, refusing anything larger than `limit` before
/// it is read so a huge file never fills memory.
fn read_file(path: &Path, limit: u64) -> Result<(String, Vec<u8>), AppError> {
    let unreadable = || AppError::new(ErrorCode::StorageUnavailable);
    let metadata = std::fs::metadata(path).map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(AppError::validation("files"));
    }
    if metadata.len() > limit {
        return Err(AppError::new(ErrorCode::AttachmentTooLarge));
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::validation("files"))?;
    let bytes = std::fs::read(path).map_err(|_| unreadable())?;
    Ok((file_name, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_paths_are_claimed_once_and_per_window() {
        let dropped = DroppedFiles::default();
        dropped.put("composer-a", vec![PathBuf::from("/a")]);
        dropped.put("composer-b", vec![PathBuf::from("/b")]);
        assert_eq!(dropped.take("composer-a"), vec![PathBuf::from("/a")]);
        assert!(dropped.take("composer-a").is_empty());
        dropped.forget("composer-b");
        assert!(dropped.take("composer-b").is_empty());
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                tauri::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn raw_inline_headers_are_decoded_and_bad_ones_refused() {
        let ok = headers(&[
            (DRAFT_HEADER, "d1"),
            (REVISION_HEADER, "7"),
            (FILE_NAME_HEADER, "%E5%9B%BE%20a.png"),
        ]);
        let (request, name) = raw_request(&ok).unwrap();
        assert_eq!(
            (
                request.draft_id.as_str(),
                request.expected_revision,
                name.as_str()
            ),
            ("d1", 7, "图 a.png")
        );
        let missing = headers(&[(DRAFT_HEADER, "d1"), (REVISION_HEADER, "7")]);
        assert!(raw_request(&missing).is_err());
        let bad_revision = headers(&[
            (DRAFT_HEADER, "d1"),
            (REVISION_HEADER, "-1"),
            (FILE_NAME_HEADER, "a.png"),
        ]);
        assert!(raw_request(&bad_revision).is_err());
        let bad_utf8 = headers(&[
            (DRAFT_HEADER, "d1"),
            (REVISION_HEADER, "1"),
            (FILE_NAME_HEADER, "%FF"),
        ]);
        assert!(raw_request(&bad_utf8).is_err());
        let empty_name = headers(&[
            (DRAFT_HEADER, "d1"),
            (REVISION_HEADER, "1"),
            (FILE_NAME_HEADER, ""),
        ]);
        assert!(raw_request(&empty_name).is_err());
    }

    #[test]
    fn a_directory_or_missing_file_is_refused_and_an_oversized_one_is_not_read() {
        let dir = std::env::temp_dir().join(format!("mailx-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_file(&dir, 10).is_err());
        assert!(read_file(&dir.join("missing"), 10).is_err());
        let file = dir.join("note.txt");
        std::fs::write(&file, b"0123456789ab").unwrap();
        assert_eq!(
            read_file(&file, 10).unwrap_err().code,
            ErrorCode::AttachmentTooLarge
        );
        let (name, bytes) = read_file(&file, 12).unwrap();
        assert_eq!((name.as_str(), bytes.len()), ("note.txt", 12));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
