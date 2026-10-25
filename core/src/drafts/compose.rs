//! Reply, reply-all and forward drafts built from a cached message
//! (docs/architecture/sending.md section 7). Pure functions: the caller reads
//! the message, its account address and its raw MIME, and no network is used.
use chrono::{DateTime, FixedOffset};
use mail_parser::{HeaderValue, MessageParser};

use super::{
    body::plain_text_to_html,
    types::DraftKind,
    validate::{DraftContent, MAX_SUBJECT_BYTES, clean_address},
};
use crate::mail::{EmailDetail, MailAddress};

/// Cap on carried-over `References` ids, newest kept, so a long thread does
/// not grow the header without bound.
const MAX_REFERENCES: usize = 50;

const REPLY_PREFIXES: &[&str] = &["re:", "回复：", "回复:", "答复：", "答复:"];
const FORWARD_PREFIXES: &[&str] = &["fwd:", "fw:", "转发：", "转发:"];

/// What a generated draft holds besides its kind and account.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Composed {
    pub content: DraftContent,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

/// Facts about the source message that the cached summary does not carry.
pub(crate) struct SourceHeaders {
    /// The reader also renders text-only messages as HTML. Only reuse that
    /// cached document when the original MIME actually had an HTML body.
    pub has_html: bool,
    /// `Message-ID` without angle brackets, if the message has one.
    pub message_id: Option<String>,
    /// Ids from the original `References` / `In-Reply-To`, without brackets.
    pub references: Vec<String>,
}

/// Reads `References` (falling back to `In-Reply-To`) from the raw message.
pub(crate) fn parse_headers(raw: &[u8], cached_message_id: Option<String>) -> SourceHeaders {
    let parsed = MessageParser::default().parse(raw);
    let ids = |value: &HeaderValue<'_>| -> Vec<String> {
        match value {
            HeaderValue::Text(text) => text.split_whitespace().map(str::to_owned).collect(),
            HeaderValue::TextList(list) => list.iter().map(|id| id.to_string()).collect(),
            _ => Vec::new(),
        }
    };
    let mut references = parsed
        .as_ref()
        .map(|message| ids(message.references()))
        .unwrap_or_default();
    if references.is_empty() {
        references = parsed
            .as_ref()
            .map(|message| ids(message.in_reply_to()))
            .unwrap_or_default();
    }
    let message_id = cached_message_id
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|message| message.message_id().map(str::to_owned))
        })
        .map(|id| id.trim().trim_matches(['<', '>']).to_owned())
        .filter(|id| !id.is_empty());
    SourceHeaders {
        has_html: parsed.as_ref().is_some_and(|message| {
            message.html_body.iter().any(|id| {
                message
                    .parts
                    .get(*id as usize)
                    .is_some_and(|part| matches!(part.body, mail_parser::PartType::Html(_)))
            })
        }),
        message_id,
        references: references
            .into_iter()
            .map(|id| id.trim().trim_matches(['<', '>']).to_owned())
            .filter(|id| valid_id(id))
            .collect(),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 998
        && !id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>')
}

/// Adds `Re: ` / `Fwd: ` unless the subject already starts with a known
/// prefix of the same family.
pub(crate) fn prefixed_subject(subject: &str, kind: DraftKind) -> String {
    let cleaned: String = subject
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let subject = cleaned.trim();
    let (prefixes, add) = match kind {
        DraftKind::Forward => (FORWARD_PREFIXES, "Fwd: "),
        _ => (REPLY_PREFIXES, "Re: "),
    };
    let lower = subject.to_lowercase();
    let result = if prefixes.iter().any(|prefix| lower.starts_with(prefix)) {
        subject.to_owned()
    } else {
        format!("{add}{subject}")
    };
    truncate(&result, MAX_SUBJECT_BYTES).trim_end().to_owned()
}

fn truncate(text: &str, maximum: usize) -> &str {
    if text.len() <= maximum {
        return text;
    }
    let mut end = maximum;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn display(entry: &MailAddress) -> String {
    match entry.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} <{}>", entry.address),
        None => entry.address.clone(),
    }
}

fn display_list(list: &[MailAddress]) -> String {
    list.iter().map(display).collect::<Vec<_>>().join(", ")
}

fn format_date(received_at: &str, offset: FixedOffset) -> String {
    DateTime::parse_from_rfc3339(received_at)
        .map(|date| {
            date.with_timezone(&offset)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| received_at.to_owned())
}

/// Adds entries whose address is neither `own` nor already present
/// (case-insensitively). Unusable entries are dropped.
fn add_recipients(into: &mut Vec<MailAddress>, taken: &mut Vec<String>, from: &[MailAddress]) {
    for entry in from {
        let Some(entry) = clean_address(entry) else {
            continue;
        };
        let key = entry.address.to_ascii_lowercase();
        if !taken.contains(&key) {
            taken.push(key);
            into.push(entry);
        }
    }
}

/// Builds the draft for `kind` (`Reply`, `ReplyAll` or `Forward`).
///
/// `offset` is the time zone the quoted date is shown in. Forwards do not
/// carry `In-Reply-To` / `References`: a forward starts a new conversation.
pub(crate) fn compose(
    source: &EmailDetail,
    original: &str,
    kind: DraftKind,
    own_address: &str,
    headers: &SourceHeaders,
    offset: FixedOffset,
    locale: crate::SupportedLocale,
) -> Composed {
    let summary = &source.summary;
    let date = format_date(&summary.received_at, offset);
    let sender = display(&summary.from);
    let mut to = Vec::new();
    let mut cc = Vec::new();
    let body = if kind == DraftKind::Forward {
        let header = locale.forward_header(
            &date,
            &sender,
            &summary.subject.replace(['\r', '\n'], " "),
            &display_list(&source.to),
        );
        format!("{}{original}", plain_text_to_html(&header))
    } else {
        let mut taken = vec![own_address.to_ascii_lowercase()];
        let primary = if source.reply_to.is_empty() {
            std::slice::from_ref(&summary.from)
        } else {
            &source.reply_to[..]
        };
        add_recipients(&mut to, &mut taken, primary);
        if kind == DraftKind::ReplyAll {
            add_recipients(&mut to, &mut taken, &source.to);
            add_recipients(&mut cc, &mut taken, &source.cc);
        } else if to.is_empty() {
            // Answering a message the account sent itself goes to its recipients.
            add_recipients(&mut to, &mut taken, &source.to);
        }
        format!(
            "{}<blockquote>{original}</blockquote>",
            plain_text_to_html(&locale.reply_intro(&date, &sender))
        )
    };

    let (in_reply_to, references) = match (&headers.message_id, kind) {
        (_, DraftKind::Forward) => (None, None),
        (id, _) => {
            let mut ids = headers.references.clone();
            if let Some(id) = id {
                ids.retain(|existing| existing != id);
                ids.push(id.clone());
            }
            let skip = ids.len().saturating_sub(MAX_REFERENCES);
            let references = ids
                .into_iter()
                .skip(skip)
                .map(|id| format!("<{id}>"))
                .collect::<Vec<_>>()
                .join(" ");
            (
                id.as_ref().map(|id| format!("<{id}>")),
                (!references.is_empty()).then_some(references),
            )
        }
    };

    Composed {
        content: DraftContent {
            to,
            cc,
            bcc: Vec::new(),
            subject: prefixed_subject(&summary.subject, kind),
            body_html: body,
            body_text: String::new(),
        },
        in_reply_to,
        references,
    }
}
