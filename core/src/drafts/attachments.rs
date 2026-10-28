//! Compose attachments: DTOs, limits, file name cleanup and media type
//! detection. Persistence lives in `storage::attachments`; the service methods
//! are on `AccountService`.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    AppError, ErrorCode,
    mail::{filename::safe_file_name, render},
    storage::bodies::{BlobStore, StoredBlob},
};

/// Where an attachment came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentOrigin {
    /// Added by the user.
    User,
    /// Carried over from the original message by a forward.
    Forwarded,
}

impl AttachmentOrigin {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Forwarded => "forwarded",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "forwarded" => Some(Self::Forwarded),
            _ => None,
        }
    }
}

/// One file attached to a draft; the bytes stay in the core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachmentSummary {
    pub id: String,
    pub file_name: String,
    pub media_type: String,
    #[ts(type = "number")]
    pub size: u64,
    pub origin: AttachmentOrigin,
}

/// What a draft may carry, resolved per account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPolicy {
    /// Sum of the raw attachment bytes of one draft.
    #[ts(type = "number")]
    pub max_total_bytes: u64,
    pub max_count: u32,
    /// Raw attachment capacity, in whole MB, that the account's SMTP server
    /// advertised (`SIZE` / 1.37 for base64 and MIME overhead, rounded down).
    /// Only a hint for the composer; `maxTotalBytes` stays the enforced limit.
    /// `None` until a send has learned it.
    pub server_limit_mb: Option<u32>,
}

impl Default for AttachmentPolicy {
    fn default() -> Self {
        Self {
            max_total_bytes: 20 * 1024 * 1024,
            max_count: 20,
            server_limit_mb: None,
        }
    }
}

impl AttachmentPolicy {
    /// The default policy plus what the server's `SIZE` says. `SIZE 0` is the
    /// protocol's "no stated limit" and counts as nothing learned.
    pub fn with_server_size(size: Option<u64>) -> Self {
        Self {
            server_limit_mb: size.filter(|size| *size > 0).map(server_limit_mb),
            ..Self::default()
        }
    }
}

/// MB of raw attachment a message of `size` bytes can carry: base64 grows
/// content by 4/3 and line breaks and headers add a little more, ~1.37 overall.
fn server_limit_mb(size: u64) -> u32 {
    let raw = u128::from(size) * 100 / 137;
    u32::try_from(raw / (1024 * 1024)).unwrap_or(u32::MAX)
}

/// One file a forward carries over, already in the blob store.
pub(crate) struct ForwardedFile {
    pub file_name: String,
    pub media_type: String,
    pub blob: StoredBlob,
}

/// Decodes the source message's attachments and writes them to the blob store,
/// for a forward draft (sending.md section 16.8). Inline resources are left for
/// the later `cid:` batch. The limits are deliberately not applied here: an
/// over-limit source still yields a draft the user can trim, and queueing
/// applies the policy again.
pub(crate) fn forwarded_files(
    raw: &[u8],
    blobs: &BlobStore,
) -> Result<Vec<ForwardedFile>, AppError> {
    if raw.is_empty() {
        // No raw copy to read; nothing to carry.
        return Ok(Vec::new());
    }
    let decoded = render::attachment_contents(raw)
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    decoded
        .into_iter()
        .map(|part| {
            // Same cleanup as an added file, so a forwarded name stays the
            // sender's; only a nameless part gets a name from its type.
            let file_name = part
                .file_name
                .as_deref()
                .and_then(sanitize_file_name)
                .unwrap_or_else(|| safe_file_name(None, &part.mime_type));
            let media_type = sniff_media_type(&part.bytes, &file_name);
            Ok(ForwardedFile {
                file_name,
                media_type,
                blob: blobs.put(&part.bytes)?,
            })
        })
        .collect()
}

/// Adds a file to a draft; the bytes travel beside the request, not in it.
/// Fails with `conflict` unless `expectedRevision` is the current revision.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddAttachmentRequest {
    pub draft_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
    /// The name the host got from the user; the core cleans it up.
    pub file_name: String,
}

/// Removes one attachment from a draft.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveAttachmentRequest {
    pub draft_id: String,
    pub attachment_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
}

/// Longest file name kept, in bytes (the column and most file systems cap at 255).
const MAX_FILE_NAME_BYTES: usize = 255;

/// Strips control characters and path separators, keeps everything else as the
/// user named it. `None` when nothing usable is left.
pub(crate) fn sanitize_file_name(raw: &str) -> Option<String> {
    // Hosts may hand over a full path; only the last component is the name.
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = last
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}')
        })
        .collect();
    let mut name = cleaned.trim().to_owned();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.len() > MAX_FILE_NAME_BYTES {
        // Keep the extension, cut the stem on a character boundary.
        let extension = name
            .rfind('.')
            .map(|at| name[at..].to_owned())
            .filter(|ext| ext.len() <= 16)
            .unwrap_or_default();
        let budget = MAX_FILE_NAME_BYTES - extension.len();
        let mut stem = name[..name.len() - extension.len()].to_owned();
        while stem.len() > budget {
            stem.pop();
        }
        name = format!("{stem}{extension}");
    }
    Some(name)
}

/// Media type from the leading bytes, else from the extension, else
/// `application/octet-stream`. Never blocks a type.
pub(crate) fn sniff_media_type(bytes: &[u8], file_name: &str) -> String {
    by_content(bytes)
        .or_else(|| by_extension(file_name))
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn by_content(bytes: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    Some(if starts(b"%PDF-") {
        "application/pdf"
    } else if starts(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if starts(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        "image/gif"
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if starts(b"PK\x03\x04") {
        // Office files and many others are zip containers; the extension knows more.
        return None;
    } else if starts(b"\x1f\x8b") {
        "application/gzip"
    } else if starts(b"7z\xbc\xaf\x27\x1c") {
        "application/x-7z-compressed"
    } else {
        return None;
    })
}

fn by_extension(file_name: &str) -> Option<&'static str> {
    let extension = file_name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "ics" => "text/calendar",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_size_becomes_a_rounded_down_attachment_hint() {
        // 26 MiB of MIME carries about 19 MiB of raw bytes before rounding.
        assert_eq!(
            AttachmentPolicy::with_server_size(Some(26 * 1024 * 1024)).server_limit_mb,
            Some(18)
        );
        assert_eq!(
            AttachmentPolicy::with_server_size(None).server_limit_mb,
            None
        );
        assert_eq!(AttachmentPolicy::default().server_limit_mb, None);
        // A tiny limit rounds down to 0 rather than claiming room that is not there.
        assert_eq!(
            AttachmentPolicy::with_server_size(Some(1024)).server_limit_mb,
            Some(0)
        );
        // `SIZE 0` means the server stated no limit.
        assert_eq!(
            AttachmentPolicy::with_server_size(Some(0)).server_limit_mb,
            None
        );
    }

    #[test]
    fn file_names_lose_paths_and_control_characters() {
        assert_eq!(
            sanitize_file_name("../../etc/pa\nsswd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            sanitize_file_name(r"C:\dir\报告 终稿.pdf").as_deref(),
            Some("报告 终稿.pdf")
        );
        assert_eq!(
            sanitize_file_name("  a\u{0}b.txt ").as_deref(),
            Some("ab.txt")
        );
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name("dir/"), None);
        assert_eq!(sanitize_file_name("\u{7}"), None);
    }

    #[test]
    fn long_names_keep_their_extension() {
        let name = format!("{}.pdf", "文".repeat(200));
        let cleaned = sanitize_file_name(&name).unwrap();
        assert!(cleaned.len() <= MAX_FILE_NAME_BYTES);
        assert!(cleaned.ends_with(".pdf"));
    }

    #[test]
    fn media_type_prefers_content_then_extension() {
        assert_eq!(sniff_media_type(b"%PDF-1.7", "x.bin"), "application/pdf");
        assert_eq!(
            sniff_media_type(b"PK\x03\x04", "a.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(sniff_media_type(b"hello", "a.TXT"), "text/plain");
        assert_eq!(
            sniff_media_type(b"\x00\x01", "noext"),
            "application/octet-stream"
        );
        assert_eq!(
            sniff_media_type(b"\x00\x01", "a.weird"),
            "application/octet-stream"
        );
    }
}
