//! Compose attachments: DTOs, limits, file name cleanup and media type
//! detection. Persistence lives in `storage::attachments`; the service methods
//! are on `AccountService`.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
}

impl Default for AttachmentPolicy {
    fn default() -> Self {
        Self {
            max_total_bytes: 20 * 1024 * 1024,
            max_count: 20,
        }
    }
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
