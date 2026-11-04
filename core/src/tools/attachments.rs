//! Local file attachments on drafts. File paths never leave the input boundary.
use std::fs::File;
use std::io::Read;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::drafts::{DraftAttachmentInfo, ensure_editable, load_draft};
use super::{Action, Annotations, Audience, AudiencePolicy, Principal, ToolCall, ToolError};
use super::{ToolErrorCode, ToolSpec, Tools};
use crate::{AddAttachmentRequest, AttachmentOrigin, Draft, RemoveAttachmentRequest};

const MAX_PATH_CHARS: usize = 4096;
const MAX_FILE_NAME_CHARS: usize = 255;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddDraftAttachmentInput {
    pub draft_id: String,
    pub expected_revision: u32,
    /// Absolute path to a regular file on the computer running MailX.
    pub file_path: String,
    /// Optional attachment name; otherwise use the file's name.
    pub file_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddDraftAttachmentOutput {
    pub draft_id: String,
    pub revision: u32,
    pub attachment: DraftAttachmentInfo,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveDraftAttachmentInput {
    pub draft_id: String,
    pub expected_revision: u32,
    pub attachment_id: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveDraftAttachmentOutput {
    pub draft_id: String,
    pub revision: u32,
    pub removed: bool,
}

pub(super) fn register(tools: &mut Tools) {
    let add = ToolSpec::new::<AddDraftAttachmentInput, AddDraftAttachmentOutput>(
        "add_draft_attachment",
        "Add a draft attachment",
        "Attach one local file to a saved draft. Use an absolute path on the MailX computer. \
         Drafts open in the editor or queued for sending cannot change; limits are 20 files and 20 MiB total.",
        Annotations {
            open_world: true,
            ..Annotations::WRITES_LOCAL
        },
    )
    .requires(&[Action::DraftWrite, Action::FilesRead])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(add, add_attachment);

    let remove = ToolSpec::new::<RemoveDraftAttachmentInput, RemoveDraftAttachmentOutput>(
        "remove_draft_attachment",
        "Remove a draft attachment",
        "Remove a regular attachment using its id from read_draft. Inline images are not supported. \
         Requires the current revision and a draft that is neither open nor queued.",
        Annotations::DESTROYS_LOCAL,
    )
    .requires(&[Action::DraftWrite])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(remove, remove_attachment);
}

fn editable_draft(
    call: &ToolCall<'_>,
    draft_id: &str,
    expected_revision: u32,
) -> Result<Draft, ToolError> {
    let draft = load_draft(call, draft_id)?;
    call.note_resource(&draft.account_id);
    ensure_editable(call, &draft.id)?;
    if draft.revision != expected_revision {
        return Err(ToolError::new(ToolErrorCode::Conflict));
    }
    Ok(draft)
}

fn add_attachment(
    call: &ToolCall<'_>,
    input: AddDraftAttachmentInput,
) -> Result<AddDraftAttachmentOutput, ToolError> {
    let current = editable_draft(call, &input.draft_id, input.expected_revision)?;
    let path = validate_path(&input.file_path)?;
    let name = match input.file_name {
        Some(name) if !name.is_empty() && name.chars().count() <= MAX_FILE_NAME_CHARS => name,
        Some(_) => return Err(ToolError::invalid("fileName", "invalid name")),
        None => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| ToolError::invalid("filePath", "missing file name"))?,
    };
    let limit = call
        .core
        .attachment_policy(&current.account_id)?
        .max_total_bytes;
    let bytes = read_file(path, limit)?;
    // File I/O runs without the database lock. Recheck editor and revision
    // after it, and let the core check revision/queue state again at commit.
    editable_draft(call, &current.id, input.expected_revision)?;
    let origin = match call.ctx.principal {
        Principal::LocalMcp { .. } => AttachmentOrigin::Mcp,
        Principal::BuiltinAgent { .. } => AttachmentOrigin::Ai,
    };
    let updated = call.core.add_attachment_with_origin(
        AddAttachmentRequest {
            draft_id: current.id,
            expected_revision: input.expected_revision,
            file_name: name,
        },
        &bytes,
        origin,
    )?;
    let added = updated
        .attachments
        .iter()
        .find(|attachment| {
            !current
                .attachments
                .iter()
                .any(|old| old.id == attachment.id)
        })
        .ok_or_else(|| ToolError::new(ToolErrorCode::Internal))?;
    call.note_resource(&added.id);
    Ok(AddDraftAttachmentOutput {
        draft_id: updated.id,
        revision: updated.revision,
        attachment: DraftAttachmentInfo {
            id: added.id.clone(),
            name: added.file_name.clone(),
            media_type: added.media_type.clone(),
            size: added.size,
            inline: added.inline,
            origin: added.origin,
        },
    })
}

fn remove_attachment(
    call: &ToolCall<'_>,
    input: RemoveDraftAttachmentInput,
) -> Result<RemoveDraftAttachmentOutput, ToolError> {
    let draft = editable_draft(call, &input.draft_id, input.expected_revision)?;
    let attachment = draft
        .attachments
        .iter()
        .find(|attachment| attachment.id == input.attachment_id)
        .ok_or_else(|| ToolError::new(ToolErrorCode::NotFound))?;
    if attachment.inline {
        return Err(ToolError::invalid(
            "attachmentId",
            "inline images cannot be removed here",
        ));
    }
    call.note_resource(&attachment.id);
    let updated = call.core.remove_attachment(RemoveAttachmentRequest {
        draft_id: draft.id,
        expected_revision: input.expected_revision,
        attachment_id: input.attachment_id,
    })?;
    Ok(RemoveDraftAttachmentOutput {
        draft_id: updated.id,
        revision: updated.revision,
        removed: true,
    })
}

fn validate_path(value: &str) -> Result<&Path, ToolError> {
    let path = Path::new(value);
    if value.chars().count() > MAX_PATH_CHARS
        || value.contains('\0')
        || !path.is_absolute()
        || path.file_name().is_none()
    {
        return Err(ToolError::invalid(
            "filePath",
            "expected an absolute local file path",
        ));
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        ) || path.components().any(|part| {
            matches!(part, Component::Normal(name) if invalid_windows_name(&name.to_string_lossy()))
        }) {
            return Err(ToolError::invalid("filePath", "expected an absolute local file path"));
        }
    }
    Ok(path)
}

#[cfg(windows)]
fn invalid_windows_name(name: &str) -> bool {
    let stem = name
        .trim_end_matches(['.', ' '])
        .split('.')
        .next()
        .unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    name.contains(':')
        || matches!(
            upper.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, ToolError> {
    let unavailable = || ToolError::new(ToolErrorCode::FileUnavailable);
    let metadata = std::fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !metadata.is_file() {
        return Err(ToolError::invalid("filePath", "expected a regular file"));
    }
    if metadata.len() > limit {
        return Err(ToolError::new(ToolErrorCode::AttachmentTooLarge));
    }
    let file = open_file(path).map_err(|_| unavailable())?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file() {
        return Err(ToolError::invalid("filePath", "expected a regular file"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ToolError::invalid("filePath", "expected a regular file"));
        }
    }
    if metadata.len() > limit {
        return Err(ToolError::new(ToolErrorCode::AttachmentTooLarge));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > limit {
        return Err(ToolError::new(ToolErrorCode::AttachmentTooLarge));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_file(path: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    // A path replaced with a FIFO between metadata and open must not block.
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    Ok(File::from(fd))
}

#[cfg(windows)]
fn open_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(test)]
mod tests;
