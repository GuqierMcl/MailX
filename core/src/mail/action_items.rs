//! Shared action-item (calendar event / task) types, sanitization helpers, and
//! stable key derivation used across L0 (.ics), L1 (local rules), and L2 (AI).
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_LOCATION_CHARS: usize = 300;
pub const MAX_NOTES_CHARS: usize = 4000;
pub const MAX_URL_CHARS: usize = 2048;
pub const MAX_MATCHED_TEXT_CHARS: usize = 500;
pub const MAX_RRULE_CHARS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActionItemKind {
    Event,
    Task,
}

impl ActionItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Task => "task",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActionItemSource {
    Ics,
    Rule,
    Ai,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActionItemStatus {
    #[default]
    Pending,
    Added,
    Dismissed,
}

impl ActionItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Added => "added",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "added" => Some(Self::Added),
            "dismissed" => Some(Self::Dismissed),
            "pending" => Some(Self::Pending),
            _ => None,
        }
    }
}

/// Unified candidate event or task produced by L0 (.ics), L1 (local rules), or
/// L2 (built-in AI).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DetectedActionItem {
    /// Stable key used for persistence (`added`/`dismissed`) and deterministic ICS UIDs.
    pub id: String,
    pub kind: ActionItemKind,
    pub source: ActionItemSource,
    pub status: ActionItemStatus,
    pub title: String,
    /// RFC 3339 timestamp with offset, or `YYYY-MM-DD` when `all_day` is true.
    pub start_at: Option<String>,
    /// RFC 3339 timestamp with offset, or `YYYY-MM-DD` when `all_day` is true.
    pub end_at: Option<String>,
    pub all_day: bool,
    /// Validated RFC 5545 RRULE value (L0 only).
    pub rrule: Option<String>,
    /// True when an L0 invitation carries `METHOD:CANCEL`.
    pub cancelled: bool,
    pub location: Option<String>,
    /// Validated `http` or `https` URL.
    pub url: Option<String>,
    pub notes: Option<String>,
    /// Short excerpt from the email body for user verification.
    pub matched_text: Option<String>,
}

/// User-edited payload submitted from the confirmation dialog to generate an
/// `.ics` file or formatted task text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemExportInput {
    pub email_id: String,
    pub item_id: String,
    pub kind: ActionItemKind,
    pub title: String,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub all_day: bool,
    pub rrule: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
}

/// Removes ASCII/Unicode control characters (keeping `\n` when `allow_newlines`
/// is true) and Unicode bidirectional override/isolate characters that could
/// spoof displayed text.
pub fn clean_text(raw: &str, max_chars: usize, allow_newlines: bool) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    let mut prev_cr = false;
    for ch in raw.chars() {
        if is_bidi_control(ch) {
            continue;
        }
        if ch == '\r' {
            if allow_newlines && count < max_chars {
                out.push('\n');
                count += 1;
            }
            prev_cr = true;
            continue;
        }
        if ch == '\n' {
            if prev_cr {
                prev_cr = false;
                continue;
            }
            if allow_newlines {
                if count >= max_chars {
                    break;
                }
                out.push('\n');
                count += 1;
            } else if count < max_chars {
                out.push(' ');
                count += 1;
            }
            continue;
        }
        prev_cr = false;
        if ch.is_control() {
            if ch == '\t' && count < max_chars {
                out.push(' ');
                count += 1;
            }
            continue;
        }
        if count >= max_chars {
            break;
        }
        out.push(ch);
        count += 1;
    }
    if allow_newlines {
        out.trim().to_owned()
    } else {
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'..='\u{200F}'
            | '\u{061C}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Validates that a URL is strictly `http://` or `https://` with a host and no
/// control or bidi characters.
pub fn sanitize_http_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_URL_CHARS
        || trimmed
            .chars()
            .any(|c| c.is_control() || is_bidi_control(c))
    {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    match parsed.scheme() {
        "http" | "https" if parsed.host_str().is_some() => Some(parsed.to_string()),
        _ => None,
    }
}

/// Validates a subset of RFC 5545 `RRULE` syntax (key=value pairs separated by
/// `;` with safe ASCII characters only).
pub fn sanitize_rrule(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_RRULE_CHARS {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b';' | b'=' | b',' | b'-' | b'+'))
    {
        return None;
    }
    // Must contain FREQ=...
    let has_freq = trimmed.split(';').any(|part| {
        part.split_once('=')
            .is_some_and(|(k, v)| k.eq_ignore_ascii_case("FREQ") && !v.is_empty())
    });
    has_freq.then(|| trimmed.to_ascii_uppercase())
}

/// Derives a stable `item_key` for an L2 AI item so re-extraction preserves
/// `added` marks for the same event/task.
pub fn ai_item_key(kind: ActionItemKind, start_at: Option<&str>, title: &str) -> String {
    let date_part = start_at
        .and_then(|s| s.get(..10))
        .filter(|d| d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-')
        .unwrap_or("none");
    let norm_title: String = title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(32)
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(date_part.as_bytes());
    hasher.update(b":");
    hasher.update(norm_title.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("ai:{}:{}:{}", kind.as_str(), date_part, &digest[..12])
}

/// Derives a stable `item_key` for an L0 `.ics` event.
pub fn ics_item_key(uid: &str, recurrence_id: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(uid.as_bytes());
    if let Some(rec) = recurrence_id {
        hasher.update(b"|");
        hasher.update(rec.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("ics:{}", &digest[..16])
}

/// Derives a deterministic `UID` for exported `.ics` files so adding the same
/// item twice updates rather than duplicates the event in calendar apps.
pub fn deterministic_ics_uid(email_id: &str, item_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(email_id.as_bytes());
    hasher.update(b":");
    hasher.update(item_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{}@mailx.local", &digest[..24])
}

/// Formats a task as clean plain text for clipboard copying (Phase 1 primary
/// action for tasks).
pub fn format_task_text(input: &ActionItemExportInput) -> String {
    let title = clean_text(&input.title, MAX_TITLE_CHARS, false);
    let mut lines = vec![format!("- [ ] {title}")];
    if let Some(start) = input
        .start_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("  Due: {start}"));
    }
    if let Some(url) = input.url.as_deref().and_then(sanitize_http_url) {
        lines.push(format!("  Link: {url}"));
    }
    if let Some(notes) = input
        .notes
        .as_deref()
        .map(|n| clean_text(n, MAX_NOTES_CHARS, true))
        .filter(|n| !n.is_empty())
    {
        for line in notes.lines() {
            lines.push(format!("  {line}"));
        }
    }
    lines.join("\n")
}
