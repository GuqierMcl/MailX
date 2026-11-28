//! L2 built-in AI action-item (calendar event & task) extraction
//! (`docs/architecture/action-items.md` §2.3).
use std::{collections::HashSet, fmt::Write as _};

use chrono::{DateTime, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Inner, agent, prompt::ACTION_ITEMS_PROMPT_VERSION};
use crate::{
    AppError, ErrorCode,
    mail::action_items::{
        ActionItemKind, ActionItemSource, ActionItemStatus, DetectedActionItem, MAX_LOCATION_CHARS,
        MAX_MATCHED_TEXT_CHARS, MAX_NOTES_CHARS, MAX_TITLE_CHARS, ai_item_key, clean_text,
        sanitize_http_url,
    },
    settings::AssistantSettings,
    storage::action_items::ActionExtractionKey,
    tools::{Principal, RequestContext, ToolError, ToolErrorCode},
};

const MAX_ID_CHARS: usize = 128;
const MAX_BODY_CHARS: usize = 20_000;
const MAX_HEADER_CHARS: usize = 300;
const MAX_TIME_ZONE_CHARS: usize = 64;
pub(crate) const MAX_AI_ACTION_ITEMS: usize = 10;

pub(super) fn valid_id(email_id: &str) -> bool {
    !email_id.is_empty() && email_id.chars().count() <= MAX_ID_CHARS
}

pub(super) fn normalize_time_zone(raw: Option<&str>) -> String {
    let Some(tz) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return "UTC".to_owned();
    };
    if tz.len() <= MAX_TIME_ZONE_CHARS
        && tz
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'+'))
    {
        tz.to_owned()
    } else {
        "UTC".to_owned()
    }
}

pub(super) struct Source {
    pub message_id: String,
    pub time_zone: String,
    pub input: String,
    pub sha256: String,
    pub allowed_links: HashSet<String>,
}

impl Source {
    pub(super) fn key(&self) -> ActionExtractionKey<'_> {
        ActionExtractionKey {
            message_id: &self.message_id,
            source_sha256: &self.sha256,
            prompt_version: ACTION_ITEMS_PROMPT_VERSION,
        }
    }
}

fn flat(text: &str, max: usize) -> String {
    clean_text(text, max, false)
}

pub(super) async fn read(
    inner: &std::sync::Arc<Inner>,
    settings: &AssistantSettings,
    email_id: &str,
    time_zone: &str,
) -> Result<Source, AppError> {
    let inner = inner.clone();
    let settings = settings.clone();
    let email_id = email_id.to_owned();
    let time_zone = time_zone.to_owned();
    tokio::task::spawn_blocking(move || read_blocking(&inner, &settings, &email_id, &time_zone))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
}

fn read_blocking(
    inner: &Inner,
    settings: &AssistantSettings,
    email_id: &str,
    time_zone: &str,
) -> Result<Source, AppError> {
    let grant =
        agent::grant_for(inner, settings).ok_or_else(|| AppError::new(ErrorCode::Internal))?;
    let context = RequestContext::new(
        Principal::BuiltinAgent {
            run_id: "action-items".to_owned(),
        },
        grant,
        "action-items".to_owned(),
    );
    let output = inner
        .tools
        .call(
            &context,
            "mail_read",
            json!({ "emailId": email_id, "maxChars": MAX_BODY_CHARS }),
        )
        .map_err(|error| match error {
            ToolError {
                code: ToolErrorCode::BodyNotCached,
                ..
            } => AppError::new(ErrorCode::BodyNotCached),
            ToolError {
                code: ToolErrorCode::Internal,
                ..
            } => AppError::new(ErrorCode::Internal),
            _ => AppError::new(ErrorCode::NotFound),
        })?;
    let allowed_links: HashSet<String> = inner
        .core
        .accounts()?
        .email_sanitized_links(email_id)
        .unwrap_or_default()
        .into_iter()
        .collect();
    Ok(build_source(email_id, time_zone, &output, allowed_links))
}

fn build_source(
    email_id: &str,
    time_zone: &str,
    output: &Value,
    allowed_links: HashSet<String>,
) -> Source {
    let text_of = |field: &str| output.get(field).and_then(Value::as_str).unwrap_or("");
    let subject = flat(text_of("subject"), MAX_HEADER_CHARS);
    let from = output.get("from");
    let from_name = flat(
        from.and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        MAX_HEADER_CHARS,
    );
    let from_address = flat(
        from.and_then(|f| f.get("address"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        MAX_HEADER_CHARS,
    );
    let received = flat(text_of("receivedAt"), 64);
    let body = text_of("text").trim();
    let cut = output
        .get("nextOffset")
        .is_some_and(|offset| !offset.is_null())
        || output
            .get("bodyTruncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    let mut hasher = Sha256::new();
    for part in [&subject, &from_name, &from_address, &received, time_zone] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher.update(body.as_bytes());
    let sha256 = format!("{:x}", hasher.finalize());

    let body = body.replace("</email>", "</ email>");
    let mut input = String::from("<email>\n");
    let _ = writeln!(input, "From: {from_name} <{from_address}>");
    let _ = writeln!(
        input,
        "Subject: {}",
        subject.replace("</email>", "</ email>")
    );
    let _ = writeln!(input, "Received: {received}");
    let _ = writeln!(input, "UserTimeZone: {time_zone}");
    if cut {
        input.push_str("Note: the text below is cut short; the email is longer.\n");
    }
    input.push('\n');
    input.push_str(&body);
    input.push_str("\n</email>");

    Source {
        message_id: email_id.to_owned(),
        time_zone: time_zone.to_owned(),
        input,
        sha256,
        allowed_links,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAiItem {
    kind: String,
    title: String,
    #[serde(default)]
    start_at: Option<String>,
    #[serde(default)]
    end_at: Option<String>,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    matched_text: Option<String>,
}

/// Parses and validates untrusted AI JSON output into `DetectedActionItem`s.
/// Any URL not verbatim in `allowed_links` is discarded to prevent prompt
/// injection of phishing links.
pub(crate) fn parse_model_output(
    raw: &str,
    allowed_links: &HashSet<String>,
) -> Result<Vec<DetectedActionItem>, AppError> {
    let json_slice = extract_json_array(raw).ok_or_else(|| AppError::new(ErrorCode::AiInvalidResponse))?;
    let parsed: Vec<RawAiItem> = serde_json::from_str(json_slice)
        .map_err(|_| AppError::new(ErrorCode::AiInvalidResponse))?;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw_item in parsed.into_iter().take(MAX_AI_ACTION_ITEMS) {
        let kind = match raw_item.kind.trim().to_ascii_lowercase().as_str() {
            "event" => ActionItemKind::Event,
            "task" => ActionItemKind::Task,
            _ => continue,
        };
        let title = clean_text(&raw_item.title, MAX_TITLE_CHARS, false);
        if title.is_empty() {
            continue;
        }
        let (start_at, all_day) = validate_time(raw_item.start_at.as_deref(), raw_item.all_day);
        let (end_at, _) = validate_time(raw_item.end_at.as_deref(), all_day);
        // Events without a valid start time are downgraded to tasks.
        let kind = if kind == ActionItemKind::Event && start_at.is_none() {
            ActionItemKind::Task
        } else {
            kind
        };
        let location = raw_item
            .location
            .as_deref()
            .map(|s| clean_text(s, MAX_LOCATION_CHARS, false))
            .filter(|s| !s.is_empty());
        let notes = raw_item
            .notes
            .as_deref()
            .map(|s| clean_text(s, MAX_NOTES_CHARS, true))
            .filter(|s| !s.is_empty());
        let matched_text = raw_item
            .matched_text
            .as_deref()
            .map(|s| clean_text(s, MAX_MATCHED_TEXT_CHARS, false))
            .filter(|s| !s.is_empty());
        let url = raw_item
            .url
            .as_deref()
            .and_then(sanitize_http_url)
            .filter(|u| allowed_links.contains(u));

        let id = ai_item_key(kind, start_at.as_deref(), &title);
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(DetectedActionItem {
            id,
            kind,
            source: ActionItemSource::Ai,
            status: ActionItemStatus::Pending,
            title,
            start_at,
            end_at,
            all_day,
            rrule: None,
            cancelled: false,
            location,
            url,
            notes,
            matched_text,
        });
    }
    Ok(out)
}

fn extract_json_array(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let start = trimmed.find('[')?;
    let end = trimmed.rfind(']')?;
    (end >= start).then(|| &trimmed[start..=end])
}

fn validate_time(raw: Option<&str>, all_day_hint: bool) -> (Option<String>, bool) {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, false);
    };
    if s.len() == 10 && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return (Some(s.to_owned()), true);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        if all_day_hint {
            return (Some(dt.format("%Y-%m-%d").to_string()), true);
        }
        return (
            Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            false,
        );
    }
    (None, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discards_url_not_in_sanitized_links_and_strips_bidi() {
        let mut allowed = HashSet::new();
        allowed.insert("https://meet.example.com/ok".to_owned());
        let raw = r#"```json
[
  {
    "kind": "event",
    "title": "Design \u202EReview",
    "startAt": "2026-12-03T14:00:00+08:00",
    "url": "https://evil.example.com/phish",
    "matchedText": "Dec 3 at 2pm"
  },
  {
    "kind": "event",
    "title": "Team Sync",
    "startAt": "2026-12-04T10:00:00+08:00",
    "url": "https://meet.example.com/ok"
  }
]
```"#;
        let items = parse_model_output(raw, &allowed).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Design Review");
        assert_eq!(items[0].url, None);
        assert_eq!(items[1].url.as_deref(), Some("https://meet.example.com/ok"));
    }
}
