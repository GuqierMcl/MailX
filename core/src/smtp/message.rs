//! Outgoing MIME construction (docs/architecture/sending.md §8).
//!
//! A pure function from the draft content and a few generated values to the
//! exact bytes that will be sent, plus the SMTP envelope. `lettre` only builds
//! and encodes the message; nothing here touches the network.
//!
//! Address policy: step 1's conservative local validator
//! (`drafts::validate::valid_address`: ASCII dot-atom local part, ASCII host,
//! no quoted or internationalized forms) stays authoritative because it is
//! stricter than lettre's parser (lettre accepts quoted local parts and IDNA
//! domains). The builder additionally requires lettre to accept the address,
//! so both agree before a byte is produced.

use std::str::FromStr;

use chrono::{DateTime, FixedOffset, Local, Offset, Utc};
use lettre::address::Envelope;
use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Mailbox, SinglePart};
use lettre::{Address, Message};
use uuid::Uuid;

use crate::AppError;
use crate::drafts::types::Draft;
use crate::drafts::validate::valid_address;
use crate::mail::MailAddress;

/// SMTP envelope: `MAIL FROM` and the deduplicated `RCPT TO` list.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SmtpEnvelope {
    pub from: String,
    pub recipients: Vec<String>,
}

/// What is persisted at enqueue time and sent unchanged on every attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuiltMessage {
    pub mime: Vec<u8>,
    pub envelope: SmtpEnvelope,
    /// With angle brackets, as written in the header.
    pub message_id: String,
}

pub(crate) struct OutgoingMessage<'a> {
    pub sender_address: &'a str,
    pub sender_name: Option<&'a str>,
    pub draft: &'a Draft,
    /// With angle brackets; see [`new_message_id`].
    pub message_id: &'a str,
    pub date: DateTime<FixedOffset>,
}

/// `<uuid@domain-of-the-sender-address>`.
pub(crate) fn new_message_id(sender_address: &str) -> Result<String, AppError> {
    let address = checked_address(sender_address, "account.emailAddress")?;
    Ok(format!("<{}@{}>", Uuid::new_v4(), address.domain()))
}

/// The current time in the machine's local offset, for the `Date` header.
pub(crate) fn local_now() -> DateTime<FixedOffset> {
    let offset = Local::now().offset().fix();
    Utc::now().with_timezone(&offset)
}

fn plain(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

fn checked_address(address: &str, field: &str) -> Result<Address, AppError> {
    if !valid_address(address) {
        return Err(AppError::validation(field));
    }
    Address::from_str(address).map_err(|_| AppError::validation(field))
}

fn mailbox(entry: &MailAddress, field: &str) -> Result<Mailbox, AppError> {
    let name = entry
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if name.is_some_and(|name| !plain(name)) {
        return Err(AppError::validation(field));
    }
    let address = checked_address(entry.address.trim(), field)?;
    Ok(Mailbox::new(name.map(str::to_owned), address))
}

/// One `<local@domain>` token: printable ASCII, no spaces, no nested brackets.
fn message_id_token(token: &str) -> bool {
    let Some(inner) = token
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    inner.len() <= 998
        && inner.contains('@')
        && inner
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>')
}

fn message_id_list(value: &str, field: &str) -> Result<(), AppError> {
    let mut tokens = value.split(' ').peekable();
    tokens.peek().ok_or_else(|| AppError::validation(field))?;
    if tokens.all(message_id_token) {
        Ok(())
    } else {
        Err(AppError::validation(field))
    }
}

pub(crate) fn build(message: &OutgoingMessage<'_>) -> Result<BuiltMessage, AppError> {
    let draft = message.draft;
    let sender_address = checked_address(message.sender_address, "account.emailAddress")?;
    let sender_name = message
        .sender_name
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if sender_name.is_some_and(|name| !plain(name)) {
        return Err(AppError::validation("account.senderName"));
    }
    if !message_id_token(message.message_id) {
        return Err(AppError::validation("messageId"));
    }
    if !plain(&draft.subject) {
        return Err(AppError::validation("draft.subject"));
    }
    if let Some(value) = &draft.in_reply_to {
        message_id_list(value, "draft.inReplyTo")?;
    }
    if let Some(value) = &draft.references {
        message_id_list(value, "draft.references")?;
    }

    let to = draft
        .to
        .iter()
        .map(|entry| mailbox(entry, "draft.to"))
        .collect::<Result<Vec<_>, _>>()?;
    let cc = draft
        .cc
        .iter()
        .map(|entry| mailbox(entry, "draft.cc"))
        .collect::<Result<Vec<_>, _>>()?;
    let bcc = draft
        .bcc
        .iter()
        .map(|entry| mailbox(entry, "draft.bcc"))
        .collect::<Result<Vec<_>, _>>()?;

    // To + Cc + Bcc, case-insensitively deduplicated, first spelling kept.
    let mut recipients: Vec<Address> = Vec::new();
    for entry in to.iter().chain(&cc).chain(&bcc) {
        let known = recipients
            .iter()
            .any(|seen| seen.to_string().eq_ignore_ascii_case(entry.email.as_ref()));
        if !known {
            recipients.push(entry.email.clone());
        }
    }
    if recipients.is_empty() {
        return Err(AppError::validation("draft.recipients"));
    }
    let envelope = Envelope::new(Some(sender_address.clone()), recipients)
        .map_err(|_| AppError::validation("draft.recipients"))?;

    let mut builder = Message::builder()
        .from(Mailbox::new(
            sender_name.map(str::to_owned),
            sender_address.clone(),
        ))
        .subject(draft.subject.clone())
        .message_id(Some(message.message_id.to_owned()))
        .envelope(envelope.clone());
    for entry in to {
        builder = builder.to(entry);
    }
    for entry in cc {
        builder = builder.cc(entry);
    }
    // No Bcc header is ever written: Bcc recipients exist only in the envelope.
    if let Some(value) = &draft.in_reply_to {
        builder = builder.in_reply_to(value.clone());
    }
    if let Some(value) = &draft.references {
        builder = builder.references(value.clone());
    }
    let body = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(draft.body_text.clone());
    let mut built = builder
        .singlepart(body)
        .map_err(|_| AppError::validation("draft"))?;

    // lettre can only render `Date` in UTC; the local offset is set afterwards.
    let date = HeaderValue::new(
        HeaderName::new_from_ascii_str("Date"),
        message.date.to_rfc2822(),
    );
    built.headers_mut().insert_raw(date);

    Ok(BuiltMessage {
        mime: built.formatted(),
        envelope: SmtpEnvelope {
            from: sender_address.to_string(),
            recipients: envelope.to().iter().map(ToString::to_string).collect(),
        },
        message_id: message.message_id.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use mail_parser::{HeaderValue as ParsedValue, MessageParser};

    use super::*;
    use crate::drafts::types::DraftKind;

    fn who(name: Option<&str>, address: &str) -> MailAddress {
        MailAddress {
            name: name.map(str::to_owned),
            address: address.to_owned(),
        }
    }

    fn draft() -> Draft {
        Draft {
            id: "d".into(),
            account_id: "a".into(),
            kind: DraftKind::New,
            source_email_id: None,
            to: vec![who(Some("李四"), "li@example.org")],
            cc: vec![],
            bcc: vec![],
            subject: "你好，世界".into(),
            body_text: "正文\n第二行\n".into(),
            in_reply_to: None,
            references: None,
            created_at: String::new(),
            updated_at: String::new(),
            revision: 1,
        }
    }

    fn date() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 23, 9, 30, 0)
            .unwrap()
    }

    fn make(draft: &Draft, name: Option<&str>) -> Result<BuiltMessage, AppError> {
        let id = new_message_id("zhang@Mail.Example.com")?;
        build(&OutgoingMessage {
            sender_address: "zhang@Mail.Example.com",
            sender_name: name,
            draft,
            message_id: &id,
            date: date(),
        })
    }

    fn header_names(raw: &[u8]) -> Vec<String> {
        let text = String::from_utf8(raw.to_vec()).unwrap();
        let head = text.split("\r\n\r\n").next().unwrap();
        head.lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('\t'))
            .map(|line| line.split(':').next().unwrap().to_ascii_lowercase())
            .collect()
    }

    #[test]
    fn headers_are_exactly_the_documented_set() {
        let built = make(&draft(), Some("张三")).unwrap();
        assert!(built.mime.is_ascii(), "headers and body are 7-bit safe");
        let mut names = header_names(&built.mime);
        names.sort();
        assert_eq!(
            names,
            [
                "content-transfer-encoding",
                "content-type",
                "date",
                "from",
                "message-id",
                "mime-version",
                "subject",
                "to"
            ]
        );
        let text = String::from_utf8(built.mime.clone()).unwrap();
        assert!(text.contains("MIME-Version: 1.0\r\n"));
        assert!(text.contains("charset=utf-8"));
        assert!(text.contains("Date: Fri, 23 Oct 2026 09:30:00 +0800\r\n"));
        assert!(!text.to_ascii_lowercase().contains("x-mailer"));
        assert!(!text.to_ascii_lowercase().contains("user-agent"));
    }

    #[test]
    fn chinese_subject_and_names_round_trip() {
        let mut draft = draft();
        draft.cc = vec![who(Some("王五"), "wang@example.org")];
        let built = make(&draft, Some("张三")).unwrap();
        let parsed = MessageParser::default().parse(&built.mime).unwrap();
        assert_eq!(parsed.subject(), Some("你好，世界"));
        let from = parsed.from().unwrap().first().unwrap();
        assert_eq!(from.name(), Some("张三"));
        assert_eq!(from.address(), Some("zhang@Mail.Example.com"));
        let to = parsed.to().unwrap().first().unwrap();
        assert_eq!(
            (to.name(), to.address()),
            (Some("李四"), Some("li@example.org"))
        );
        assert_eq!(parsed.cc().unwrap().first().unwrap().name(), Some("王五"));
        assert_eq!(parsed.body_text(0).unwrap(), "正文\r\n第二行\r\n");
    }

    #[test]
    fn from_has_only_the_address_without_a_sender_name() {
        let built = make(&draft(), None).unwrap();
        let parsed = MessageParser::default().parse(&built.mime).unwrap();
        let from = parsed.from().unwrap().first().unwrap();
        assert_eq!(from.name(), None);
        assert_eq!(from.address(), Some("zhang@Mail.Example.com"));
    }

    #[test]
    fn message_id_uses_the_sender_domain_and_is_unique() {
        let first = new_message_id("zhang@Mail.Example.com").unwrap();
        let second = new_message_id("zhang@Mail.Example.com").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with('<') && first.ends_with("@Mail.Example.com>"));
        let built = make(&draft(), None).unwrap();
        let parsed = MessageParser::default().parse(&built.mime).unwrap();
        assert_eq!(
            parsed.message_id(),
            Some(built.message_id.trim_matches(['<', '>']))
        );
    }

    #[test]
    fn bcc_is_only_in_the_envelope() {
        let mut draft = draft();
        draft.bcc = vec![who(Some("秘密"), "secret@example.net")];
        let built = make(&draft, None).unwrap();
        let text = String::from_utf8(built.mime.clone()).unwrap();
        assert!(!text.to_ascii_lowercase().contains("bcc"));
        assert!(!text.contains("secret@example.net"));
        assert_eq!(
            built.envelope.recipients,
            ["li@example.org", "secret@example.net"]
        );
        assert_eq!(built.envelope.from, "zhang@Mail.Example.com");
    }

    #[test]
    fn envelope_recipients_are_deduplicated_case_insensitively() {
        let mut draft = draft();
        draft.to = vec![who(None, "A@Example.org"), who(None, "b@example.org")];
        draft.cc = vec![who(None, "a@example.ORG")];
        draft.bcc = vec![who(None, "B@EXAMPLE.org"), who(None, "c@example.org")];
        let built = make(&draft, None).unwrap();
        assert_eq!(
            built.envelope.recipients,
            ["A@Example.org", "b@example.org", "c@example.org"]
        );
    }

    #[test]
    fn cc_header_is_omitted_when_empty() {
        let built = make(&draft(), None).unwrap();
        assert!(!header_names(&built.mime).contains(&"cc".to_owned()));
    }

    #[test]
    fn replies_carry_threading_headers() {
        let mut draft = draft();
        draft.in_reply_to = Some("<orig@example.org>".into());
        draft.references = Some("<root@example.org> <orig@example.org>".into());
        let built = make(&draft, None).unwrap();
        let parsed = MessageParser::default().parse(&built.mime).unwrap();
        assert!(matches!(
            parsed.in_reply_to(),
            ParsedValue::Text(id) if id == "orig@example.org"
        ));
        match parsed.references() {
            ParsedValue::TextList(list) => {
                let ids: Vec<_> = list.iter().map(|id| id.to_string()).collect();
                assert_eq!(ids, ["root@example.org", "orig@example.org"]);
            }
            other => panic!("references: {other:?}"),
        }
    }

    #[test]
    fn at_least_one_recipient_is_required() {
        let mut draft = draft();
        draft.to.clear();
        let error = make(&draft, None).unwrap_err();
        assert_eq!(error.field.as_deref(), Some("draft.recipients"));
    }

    #[test]
    fn header_injection_is_refused_everywhere() {
        let evil = "x\r\nBcc: spy@example.net";
        type Change = Box<dyn Fn(&mut Draft)>;
        let cases: Vec<(&str, Change)> = vec![
            ("draft.subject", Box::new(|d| d.subject = evil.into())),
            ("draft.to", Box::new(|d| d.to[0].name = Some(evil.into()))),
            (
                "draft.to",
                Box::new(|d| d.to[0].address = "a@example.org\r\nBcc: s@example.net".into()),
            ),
            (
                "draft.inReplyTo",
                Box::new(|d| d.in_reply_to = Some("<a@b>\r\nBcc: s@c".into())),
            ),
            (
                "draft.references",
                Box::new(|d| d.references = Some("<a@b>\n<c@d>".into())),
            ),
            (
                "draft.references",
                Box::new(|d| d.references = Some("<a@b> Bcc: s".into())),
            ),
        ];
        for (field, change) in cases {
            let mut draft = draft();
            change(&mut draft);
            let error = make(&draft, None).unwrap_err();
            assert_eq!(error.field.as_deref(), Some(field), "{field}");
        }
        let error = make(&draft(), Some(evil)).unwrap_err();
        assert_eq!(error.field.as_deref(), Some("account.senderName"));
        let error = new_message_id("a@example.org\r\nBcc: s@example.net").unwrap_err();
        assert_eq!(error.field.as_deref(), Some("account.emailAddress"));
        let mut ok = draft();
        ok.subject = "ok".into();
        let error = build(&OutgoingMessage {
            sender_address: "zhang@example.com",
            sender_name: None,
            draft: &ok,
            message_id: "<a@b>\r\nBcc: s@c",
            date: date(),
        })
        .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("messageId"));
    }

    #[test]
    fn addresses_lettre_would_accept_but_the_local_rules_refuse_stay_refused() {
        for address in [
            "\"quoted local\"@example.org",
            "用户@example.org",
            "a@例え.jp",
        ] {
            let mut draft = draft();
            draft.to = vec![who(None, address)];
            assert!(make(&draft, None).is_err(), "{address}");
        }
    }
}
