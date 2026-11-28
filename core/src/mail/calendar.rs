//! L0 iCalendar (`text/calendar` / `.ics`) parsing and safe RFC 5545 `.ics`
//! generation (`docs/architecture/action-items.md` §2.1 & §3.3).
use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_parser::{MessageParser, MimeHeaders, PartType};

use super::action_items::{
    ActionItemExportInput, ActionItemKind, ActionItemSource, ActionItemStatus, DetectedActionItem,
    MAX_LOCATION_CHARS, MAX_NOTES_CHARS, MAX_TITLE_CHARS, clean_text, deterministic_ics_uid,
    ics_item_key, sanitize_http_url, sanitize_rrule,
};
use crate::{AppError, ErrorCode};

pub const MAX_ICS_PART_BYTES: usize = 256 * 1024;
pub const MAX_ICS_EVENTS_PER_MESSAGE: usize = 16;

/// Extracts deduplicated L0 calendar events from a raw RFC 5322 / MIME email.
/// Oversized or malformed calendar parts are silently skipped so they never
/// block normal email rendering.
pub fn extract_ics_from_eml(raw: &[u8]) -> Vec<DetectedActionItem> {
    let Some(message) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut seen_keys = HashSet::new();

    for part in &message.parts {
        let is_calendar_mime = part.content_type().is_some_and(|ct| {
            ct.ctype().eq_ignore_ascii_case("text")
                && ct
                    .subtype()
                    .is_some_and(|s| s.eq_ignore_ascii_case("calendar"))
        });
        let is_ics_filename = part
            .attachment_name()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".ics"));
        if !is_calendar_mime && !is_ics_filename {
            continue;
        }
        let bytes: &[u8] = match &part.body {
            PartType::Text(t) => t.as_bytes(),
            PartType::Html(h) => h.as_bytes(),
            PartType::Binary(b) | PartType::InlineBinary(b) => b.as_ref(),
            PartType::Multipart(_) | PartType::Message(_) => continue,
        };
        if bytes.is_empty() || bytes.len() > MAX_ICS_PART_BYTES {
            continue;
        }
        let text = String::from_utf8_lossy(bytes);
        for item in parse_ics(&text) {
            if seen_keys.insert(item.id.clone()) {
                items.push(item);
                if items.len() >= MAX_ICS_EVENTS_PER_MESSAGE {
                    return items;
                }
            }
        }
    }
    items
}

/// Parses an RFC 5545 iCalendar text into normalized `DetectedActionItem`s.
pub fn parse_ics(ics: &str) -> Vec<DetectedActionItem> {
    if ics.len() > MAX_ICS_PART_BYTES {
        return Vec::new();
    }
    let lines = unfold_lines(ics);
    let mut method = String::new();
    let mut in_vevent = false;
    let mut in_nested = 0usize; // Skip VALARM etc. inside VEVENT
    let mut current = RawVEvent::default();
    let mut items = Vec::new();
    let mut seen_keys = HashSet::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("BEGIN:VEVENT") {
            in_vevent = true;
            in_nested = 0;
            current = RawVEvent::default();
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VEVENT") {
            if in_vevent {
                let finished = std::mem::take(&mut current);
                if let Some(item) = finished.into_item(&method)
                    && seen_keys.insert(item.id.clone())
                {
                    items.push(item);
                    if items.len() >= MAX_ICS_EVENTS_PER_MESSAGE {
                        break;
                    }
                }
                in_vevent = false;
                in_nested = 0;
            }
            continue;
        }
        if in_vevent {
            if trimmed.len() > 6 && trimmed[..6].eq_ignore_ascii_case("BEGIN:") {
                in_nested += 1;
                continue;
            }
            if trimmed.len() > 4 && trimmed[..4].eq_ignore_ascii_case("END:") {
                in_nested = in_nested.saturating_sub(1);
                continue;
            }
            if in_nested > 0 {
                continue;
            }
            if let Some((name, params, value)) = parse_property(&line) {
                current.apply(name, params, value);
            }
        } else if let Some((name, _, value)) = parse_property(&line)
            && name.eq_ignore_ascii_case("METHOD")
        {
            method = value.trim().to_ascii_uppercase();
        }
    }
    items
}

#[derive(Default)]
struct RawVEvent {
    uid: Option<String>,
    recurrence_id: Option<String>,
    summary: Option<String>,
    dtstart: Option<ParsedDt>,
    dtend: Option<ParsedDt>,
    duration: Option<Duration>,
    rrule: Option<String>,
    location: Option<String>,
    description: Option<String>,
    url: Option<String>,
    status: Option<String>,
}

#[derive(Clone)]
struct ParsedDt {
    iso: String,
    all_day: bool,
    utc: Option<DateTime<Utc>>,
}

impl RawVEvent {
    fn apply(&mut self, name: &str, params: Vec<(&str, &str)>, value: &str) {
        let unescaped = unescape_ics_text(value);
        match name.to_ascii_uppercase().as_str() {
            "UID" => self.uid = Some(clean_text(&unescaped, 256, false)),
            "RECURRENCE-ID" => self.recurrence_id = Some(clean_text(value, 64, false)),
            "SUMMARY" => self.summary = Some(clean_text(&unescaped, MAX_TITLE_CHARS, false)),
            "DTSTART" => self.dtstart = parse_ics_datetime(value, &params),
            "DTEND" => self.dtend = parse_ics_datetime(value, &params),
            "DURATION" => self.duration = parse_ics_duration(value),
            "RRULE" => self.rrule = sanitize_rrule(value),
            "LOCATION" => self.location = Some(clean_text(&unescaped, MAX_LOCATION_CHARS, false)),
            "DESCRIPTION" => {
                self.description = Some(clean_text(&unescaped, MAX_NOTES_CHARS, true))
            }
            "URL" => self.url = sanitize_http_url(value.trim()),
            "STATUS" => self.status = Some(value.trim().to_ascii_uppercase()),
            _ => {}
        }
    }

    fn into_item(self, method: &str) -> Option<DetectedActionItem> {
        let dtstart = self.dtstart?;
        let title = self
            .summary
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Calendar event".to_owned());
        let end_at = if let Some(end) = self.dtend {
            Some(end.iso)
        } else if let (Some(start_utc), Some(dur)) = (dtstart.utc, self.duration) {
            let end_utc = start_utc + dur;
            Some(end_utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        } else {
            None
        };
        let uid = self
            .uid
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("{}:{}", title, dtstart.iso));
        let cancelled =
            method == "CANCEL" || self.status.as_deref() == Some("CANCELLED");
        let url = self
            .url
            .or_else(|| self.location.as_deref().and_then(extract_first_http_url))
            .or_else(|| self.description.as_deref().and_then(extract_first_http_url));

        Some(DetectedActionItem {
            id: ics_item_key(&uid, self.recurrence_id.as_deref()),
            kind: ActionItemKind::Event,
            source: ActionItemSource::Ics,
            status: ActionItemStatus::Pending,
            title,
            start_at: Some(dtstart.iso),
            end_at,
            all_day: dtstart.all_day,
            rrule: self.rrule,
            cancelled,
            location: self.location.filter(|s| !s.is_empty()),
            url,
            notes: self.description.filter(|s| !s.is_empty()),
            matched_text: None,
        })
    }
}

fn unfold_lines(input: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in input.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_owned());
    }
    lines
}

fn parse_property(line: &str) -> Option<(&str, Vec<(&str, &str)>, &str)> {
    // Find first ':' outside double quotes.
    let mut in_quotes = false;
    let mut colon_idx = None;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon_idx = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let colon = colon_idx?;
    let head = &line[..colon];
    let value = &line[colon + 1..];
    let mut parts = head.split(';');
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    for param in parts {
        if let Some((k, v)) = param.split_once('=') {
            params.push((k.trim(), v.trim().trim_matches('"')));
        }
    }
    Some((name, params, value))
}

fn unescape_ics_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_ics_datetime(value: &str, params: &[(&str, &str)]) -> Option<ParsedDt> {
    let v = value.trim();
    let is_date = params
        .iter()
        .any(|(k, val)| k.eq_ignore_ascii_case("VALUE") && val.eq_ignore_ascii_case("DATE"))
        || v.len() == 8;
    if is_date && v.len() >= 8 {
        let date = NaiveDate::parse_from_str(&v[..8], "%Y%m%d").ok()?;
        return Some(ParsedDt {
            iso: date.format("%Y-%m-%d").to_string(),
            all_day: true,
            utc: None,
        });
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
        return Some(ParsedDt {
            iso: utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            all_day: false,
            utc: Some(utc),
        });
    }
    let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let tzid = params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("TZID"))
        .map(|(_, val)| *val);
    if let Some(offset) = tzid.and_then(tzid_to_fixed_offset) {
        let dt = offset.from_local_datetime(&naive).single()?;
        return Some(ParsedDt {
            iso: dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            all_day: false,
            utc: Some(dt.with_timezone(&Utc)),
        });
    }
    // Floating local time: attach current local offset.
    if let Some(local_dt) = chrono::Local.from_local_datetime(&naive).earliest() {
        let fixed = local_dt.fixed_offset();
        return Some(ParsedDt {
            iso: fixed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            all_day: false,
            utc: Some(local_dt.with_timezone(&Utc)),
        });
    }
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
    Some(ParsedDt {
        iso: utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        all_day: false,
        utc: Some(utc),
    })
}

/// Maps common Windows and IANA TZID names to fixed offsets (covering Outlook
/// invitations such as `China Standard Time`).
fn tzid_to_fixed_offset(tzid: &str) -> Option<FixedOffset> {
    let normalized = tzid.trim().trim_matches('"');
    let secs = match normalized {
        "UTC" | "Etc/UTC" | "GMT" | "Greenwich Standard Time" => 0,
        "China Standard Time"
        | "Asia/Shanghai"
        | "Asia/Chongqing"
        | "Asia/Hong_Kong"
        | "Asia/Taipei"
        | "Asia/Singapore"
        | "Taipei Standard Time"
        | "Singapore Standard Time" => 8 * 3600,
        "Tokyo Standard Time" | "Asia/Tokyo" | "Korea Standard Time" | "Asia/Seoul" => 9 * 3600,
        "India Standard Time" | "Asia/Kolkata" => 5 * 3600 + 1800,
        "Central European Standard Time"
        | "W. Europe Standard Time"
        | "Romance Standard Time"
        | "Europe/Berlin"
        | "Europe/Paris" => 3600,
        "GMT Standard Time" | "Europe/London" => 0,
        "Eastern Standard Time" | "America/New_York" => -5 * 3600,
        "Central Standard Time" | "America/Chicago" => -6 * 3600,
        "Mountain Standard Time" | "America/Denver" => -7 * 3600,
        "Pacific Standard Time" | "America/Los_Angeles" => -8 * 3600,
        "AUS Eastern Standard Time" | "Australia/Sydney" => 10 * 3600,
        _ => return None,
    };
    FixedOffset::east_opt(secs)
}

fn parse_ics_duration(raw: &str) -> Option<Duration> {
    let s = raw.trim();
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let rest = rest.strip_prefix('P')?;
    let mut total_secs: i64 = 0;
    let mut in_time = false;
    let mut num: i64 = 0;
    for ch in rest.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' => num = num.saturating_mul(10).saturating_add((ch as u8 - b'0') as i64),
            'W' => {
                total_secs = total_secs.saturating_add(num.saturating_mul(7 * 86400));
                num = 0;
            }
            'D' => {
                total_secs = total_secs.saturating_add(num.saturating_mul(86400));
                num = 0;
            }
            'H' if in_time => {
                total_secs = total_secs.saturating_add(num.saturating_mul(3600));
                num = 0;
            }
            'M' if in_time => {
                total_secs = total_secs.saturating_add(num.saturating_mul(60));
                num = 0;
            }
            'S' if in_time => {
                total_secs = total_secs.saturating_add(num);
                num = 0;
            }
            _ => return None,
        }
    }
    if total_secs == 0 {
        return None;
    }
    Some(Duration::seconds(if neg { -total_secs } else { total_secs }))
}

fn extract_first_http_url(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | ',' | '.'));
        if (cleaned.starts_with("https://") || cleaned.starts_with("http://"))
            && let Some(valid) = sanitize_http_url(cleaned)
        {
            return Some(valid);
        }
    }
    None
}

/// Generates a safe RFC 5545 `.ics` document (`METHOD:PUBLISH`, no
/// `ORGANIZER`/`ATTENDEE`/`VALARM`, 75-byte UTF-8 safe line folding, strict
/// value escaping, deterministic `UID`).
pub fn generate_ics(input: &ActionItemExportInput) -> Result<String, AppError> {
    let title = clean_text(&input.title, MAX_TITLE_CHARS, false);
    if title.is_empty() {
        return Err(AppError::validation("actionItem.title"));
    }
    if input.email_id.trim().is_empty() || input.item_id.trim().is_empty() {
        return Err(AppError::validation("actionItem.id"));
    }

    let uid = deterministic_ics_uid(input.email_id.trim(), input.item_id.trim());
    let dtstamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let location = input
        .location
        .as_deref()
        .map(|s| clean_text(s, MAX_LOCATION_CHARS, false))
        .filter(|s| !s.is_empty());
    let notes = input
        .notes
        .as_deref()
        .map(|s| clean_text(s, MAX_NOTES_CHARS, true))
        .filter(|s| !s.is_empty());
    let url = match input.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(sanitize_http_url(raw).ok_or_else(|| AppError::validation("actionItem.url"))?),
        None => None,
    };
    let rrule = input.rrule.as_deref().and_then(sanitize_rrule);

    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//MailX//Action Items//EN".to_owned(),
        "CALSCALE:GREGORIAN".to_owned(),
        "METHOD:PUBLISH".to_owned(),
    ];

    match input.kind {
        ActionItemKind::Event => {
            let start_raw = input
                .start_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::validation("actionItem.startAt"))?;
            lines.push("BEGIN:VEVENT".to_owned());
            lines.push(format!("UID:{uid}"));
            lines.push(format!("DTSTAMP:{dtstamp}"));
            if input.all_day {
                let start_date = parse_date_only(start_raw)
                    .ok_or_else(|| AppError::validation("actionItem.startAt"))?;
                let end_date = input
                    .end_at
                    .as_deref()
                    .and_then(parse_date_only)
                    .filter(|end| *end > start_date)
                    .unwrap_or_else(|| start_date + Duration::days(1));
                lines.push(format!("DTSTART;VALUE=DATE:{}", start_date.format("%Y%m%d")));
                lines.push(format!("DTEND;VALUE=DATE:{}", end_date.format("%Y%m%d")));
            } else {
                let start_utc = parse_rfc3339_utc(start_raw)
                    .ok_or_else(|| AppError::validation("actionItem.startAt"))?;
                let end_utc = input
                    .end_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc)
                    .filter(|end| *end > start_utc)
                    .unwrap_or_else(|| start_utc + Duration::hours(1));
                lines.push(format!("DTSTART:{}", start_utc.format("%Y%m%dT%H%M%SZ")));
                lines.push(format!("DTEND:{}", end_utc.format("%Y%m%dT%H%M%SZ")));
            }
            lines.push(format!("SUMMARY:{}", escape_ics_text(&title)));
            if let Some(loc) = location {
                lines.push(format!("LOCATION:{}", escape_ics_text(&loc)));
            }
            if let Some(desc) = notes {
                lines.push(format!("DESCRIPTION:{}", escape_ics_text(&desc)));
            }
            if let Some(u) = url {
                lines.push(format!("URL:{u}"));
            }
            if let Some(rule) = rrule {
                lines.push(format!("RRULE:{rule}"));
            }
            lines.push("END:VEVENT".to_owned());
        }
        ActionItemKind::Task => {
            lines.push("BEGIN:VTODO".to_owned());
            lines.push(format!("UID:{uid}"));
            lines.push(format!("DTSTAMP:{dtstamp}"));
            lines.push(format!("SUMMARY:{}", escape_ics_text(&title)));
            if let Some(due_raw) = input
                .start_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if input.all_day {
                    let due_date = parse_date_only(due_raw)
                        .ok_or_else(|| AppError::validation("actionItem.startAt"))?;
                    lines.push(format!("DUE;VALUE=DATE:{}", due_date.format("%Y%m%d")));
                } else {
                    let due_utc = parse_rfc3339_utc(due_raw)
                        .ok_or_else(|| AppError::validation("actionItem.startAt"))?;
                    lines.push(format!("DUE:{}", due_utc.format("%Y%m%dT%H%M%SZ")));
                }
            }
            if let Some(desc) = notes {
                lines.push(format!("DESCRIPTION:{}", escape_ics_text(&desc)));
            }
            if let Some(u) = url {
                lines.push(format!("URL:{u}"));
            }
            lines.push("STATUS:NEEDS-ACTION".to_owned());
            lines.push("END:VTODO".to_owned());
        }
    }

    lines.push("END:VCALENDAR".to_owned());

    let mut output = String::new();
    for line in lines {
        fold_ics_line(&line, &mut output);
    }
    if output.is_empty() {
        return Err(AppError::new(ErrorCode::Internal));
    }
    Ok(output)
}

fn parse_date_only(raw: &str) -> Option<NaiveDate> {
    let prefix = raw.trim().get(..10)?;
    NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok()
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn escape_ics_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

/// Folds a single content line at 75 octets on UTF-8 char boundaries per
/// RFC 5545 §3.1, terminating with CRLF.
fn fold_ics_line(line: &str, out: &mut String) {
    let mut remaining = line;
    let mut first = true;
    while !remaining.is_empty() {
        let limit = if first { 75 } else { 74 };
        if !first {
            out.push(' ');
        }
        if remaining.len() <= limit {
            out.push_str(remaining);
            out.push_str("\r\n");
            break;
        }
        let mut split = limit;
        while split > 0 && !remaining.is_char_boundary(split) {
            split -= 1;
        }
        if split == 0 {
            // Single char > limit (impossible for UTF-8, max 4 bytes).
            split = remaining.chars().next().map_or(0, char::len_utf8);
        }
        out.push_str(&remaining[..split]);
        out.push_str("\r\n");
        remaining = &remaining[split..];
        first = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_tzid_rrule_and_deduplicates_uid() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
UID:weekly-sync-1@example.com\r\n\
DTSTART;TZID=\"China Standard Time\":20261203T143000\r\n\
DURATION:PT1H\r\n\
RRULE:FREQ=WEEKLY;BYDAY=TH\r\n\
SUMMARY:Q4 Review\\, Team Sync\r\n\
LOCATION:Room 302\r\n\
DESCRIPTION:Join https://meet.example.com/q4\\nBring notes\r\n\
ORGANIZER:mailto:boss@example.com\r\n\
ATTENDEE:mailto:me@example.com\r\n\
BEGIN:VALARM\r\n\
TRIGGER:-PT15M\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:weekly-sync-1@example.com\r\n\
DTSTART;TZID=\"China Standard Time\":20261203T143000\r\n\
SUMMARY:Duplicate\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let items = parse_ics(ics);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.title, "Q4 Review, Team Sync");
        assert_eq!(item.start_at.as_deref(), Some("2026-12-03T14:30:00+08:00"));
        assert_eq!(item.end_at.as_deref(), Some("2026-12-03T07:30:00Z"));
        assert_eq!(item.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TH"));
        assert_eq!(item.location.as_deref(), Some("Room 302"));
        assert_eq!(item.url.as_deref(), Some("https://meet.example.com/q4"));
        assert!(!item.cancelled);
    }

    #[test]
    fn marks_method_cancel_as_cancelled() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
METHOD:CANCEL\r\n\
BEGIN:VEVENT\r\n\
UID:cancel-1\r\n\
DTSTART:20261203T100000Z\r\n\
SUMMARY:Cancelled Meeting\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let items = parse_ics(ics);
        assert_eq!(items.len(), 1);
        assert!(items[0].cancelled);
    }

    #[test]
    fn generated_ics_forces_publish_strips_attendees_and_prevents_crlf_injection() {
        let input = ActionItemExportInput {
            email_id: "email-1".into(),
            item_id: "ics:abc".into(),
            kind: ActionItemKind::Event,
            title: "Sync\r\nATTENDEE:mailto:evil@example.com\u{202E}fake".into(),
            start_at: Some("2026-12-03T14:30:00+08:00".into()),
            end_at: Some("2026-12-03T15:30:00+08:00".into()),
            all_day: false,
            rrule: Some("FREQ=WEEKLY;COUNT=4".into()),
            location: Some("Building A; Room 1, Floor 2".into()),
            url: Some("https://meet.example.com/room".into()),
            notes: Some("Line 1\nLine 2\n这是一个非常长的中文备注用来测试七十五字节安全折行不会切断多字节字符".into()),
        };
        let ics = generate_ics(&input).unwrap();
        assert!(ics.contains("METHOD:PUBLISH\r\n"));
        assert!(!ics.contains("METHOD:REQUEST"));
        assert!(!ics.contains("\r\nATTENDEE:"));
        assert!(!ics.contains("ORGANIZER:"));
        assert!(!ics.contains("VALARM"));
        assert!(!ics.contains('\u{202E}'));
        assert!(ics.contains("LOCATION:Building A\\; Room 1\\, Floor 2\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;COUNT=4\r\n"));
        // Every folded line must be <= 75 bytes excluding CRLF.
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "line exceeded 75 bytes: {line:?}");
        }
        // Deterministic UID
        let ics2 = generate_ics(&input).unwrap();
        let uid1 = ics.lines().find(|l| l.starts_with("UID:")).unwrap();
        let uid2 = ics2.lines().find(|l| l.starts_with("UID:")).unwrap();
        assert_eq!(uid1, uid2);
    }

    #[test]
    fn rejects_non_http_url_in_export() {
        let input = ActionItemExportInput {
            email_id: "email-1".into(),
            item_id: "rule:0".into(),
            kind: ActionItemKind::Event,
            title: "Meeting".into(),
            start_at: Some("2026-12-03T14:30:00Z".into()),
            end_at: None,
            all_day: false,
            rrule: None,
            location: None,
            url: Some("javascript:alert(1)".into()),
            notes: None,
        };
        assert!(generate_ics(&input).is_err());
    }
}
