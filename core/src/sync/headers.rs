//! Best-effort decoding of metadata fetched without message bodies.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use mail_parser::{Address, MessageParser};

use crate::imap::RemoteHeader;
use crate::mail::MailAddress;

/// Parsed metadata before the storage layer assigns its local message identity.
/// Deliberately not Debug: headers must never enter diagnostic output.
#[derive(Clone)]
pub(crate) struct MessageRecord {
    pub uid: u32,
    pub message_id_header: Option<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_address: String,
    pub to_addresses: Vec<MailAddress>,
    pub cc_addresses: Vec<MailAddress>,
    pub reply_to_addresses: Vec<MailAddress>,
    pub received_at: String,
    pub sent_at: Option<String>,
    pub size: Option<u32>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub flags: Vec<String>,
    pub has_attachment: bool,
}

/// Malformed or absent fields degrade independently; no message rejects a batch.
pub(crate) fn parse_header(remote: &RemoteHeader, sync_time: DateTime<Utc>) -> MessageRecord {
    let parsed = MessageParser::default().parse_headers(&remote.header);
    let from = addresses(parsed.as_ref().and_then(|message| message.from()))
        .into_iter()
        .next();
    let sent_at = parsed
        .as_ref()
        .and_then(|message| message.date())
        .and_then(header_date);
    let received_at = remote
        .internal_date
        .map(|date| date.with_timezone(&Utc))
        .or(sent_at)
        .unwrap_or(sync_time);
    MessageRecord {
        uid: remote.uid,
        message_id_header: parsed
            .as_ref()
            .and_then(|message| message.message_id())
            .map(str::to_owned),
        subject: parsed
            .as_ref()
            .and_then(|message| message.subject())
            .unwrap_or_default()
            .to_owned(),
        from_name: from.as_ref().and_then(|address| address.name.clone()),
        from_address: from.map(|address| address.address).unwrap_or_default(),
        to_addresses: addresses(parsed.as_ref().and_then(|message| message.to())),
        cc_addresses: addresses(parsed.as_ref().and_then(|message| message.cc())),
        reply_to_addresses: addresses(parsed.as_ref().and_then(|message| message.reply_to())),
        received_at: utc_string(received_at),
        sent_at: sent_at.map(utc_string),
        size: remote.size,
        is_read: remote
            .flags
            .iter()
            .any(|flag| flag.eq_ignore_ascii_case("\\Seen")),
        is_flagged: remote
            .flags
            .iter()
            .any(|flag| flag.eq_ignore_ascii_case("\\Flagged")),
        flags: remote.flags.clone(),
        has_attachment: remote.has_attachment,
    }
}

fn addresses(addresses: Option<&Address<'_>>) -> Vec<MailAddress> {
    let Some(addresses) = addresses else {
        return Vec::new();
    };
    addresses
        .iter()
        .filter_map(|entry| {
            let address = entry.address.as_deref()?.trim();
            let (local, domain) = address.rsplit_once('@')?;
            if local.is_empty() || domain.is_empty() || address.chars().any(char::is_control) {
                return None;
            }
            Some(MailAddress {
                name: entry
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                address: address.to_owned(),
            })
        })
        .collect()
}

fn header_date(date: &mail_parser::DateTime) -> Option<DateTime<Utc>> {
    // mail-parser is intentionally lenient; validate calendar dates before
    // converting them, so e.g. February 30 does not silently roll into March.
    if !date.is_valid() {
        return None;
    }
    let local = NaiveDate::from_ymd_opt(
        i32::from(date.year),
        u32::from(date.month),
        u32::from(date.day),
    )?
    .and_hms_opt(
        u32::from(date.hour),
        u32::from(date.minute),
        u32::from(date.second),
    )?;
    let offset_seconds = (i32::from(date.tz_hour) * 3600 + i32::from(date.tz_minute) * 60)
        * if date.tz_before_gmt { -1 } else { 1 };
    FixedOffset::east_opt(offset_seconds)?
        .from_local_datetime(&local)
        .single()
        .map(|date| date.with_timezone(&Utc))
}

fn utc_string(date: DateTime<Utc>) -> String {
    date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sync_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-18T04:05:06.789Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn remote(header: impl AsRef<[u8]>) -> RemoteHeader {
        RemoteHeader {
            uid: 42,
            flags: vec!["\\sEeN".into(), "\\FLAGGED".into(), "$Forwarded".into()],
            internal_date: None,
            size: Some(1024),
            header: header.as_ref().to_vec(),
            has_attachment: true,
        }
    }

    #[test]
    fn decodes_utf8_b_q_and_adjacent_encoded_words() {
        // Fixtures independently generated with Python base64/quopri, not the
        // decoder being tested: each example represents 中文主题.
        for subject in [
            "=?UTF-8?B?5Lit5paH5Li76aKY?=",
            "=?UTF-8?Q?=E4=B8=AD=E6=96=87=E4=B8=BB=E9=A2=98?=",
            "=?UTF-8?B?5Lit5paH?= =?UTF-8?B?5Li76aKY?=",
            "=?UTF-8?B?5Lit5paH?=\r\n\t=?UTF-8?B?5Li76aKY?=",
        ] {
            let record = parse_header(&remote(format!("Subject: {subject}\r\n\r\n")), sync_time());
            assert_eq!(record.subject, "中文主题");
        }
    }

    #[test]
    fn decodes_chinese_legacy_charsets_and_sender_names() {
        // Python: base64.b64encode("中文主题".encode(charset)); sender 林晓
        // is wdbP/g== in GBK. GB18030's four-byte 𠮷 exercises full_encoding.
        for (charset, encoded, expected) in [
            ("GBK", "1tDOxNb3zOI=", "中文主题"),
            ("GB2312", "1tDOxNb3zOI=", "中文主题"),
            ("GB18030", "lTSyNdbQzsTW98zi", "𠮷中文主题"),
        ] {
            let record = parse_header(
                &remote(format!(
                    "Subject: =?{charset}?B?{encoded}?=\r\nFrom: =?GBK?B?wdbP/g==?= <lin@example.com>\r\n\r\n"
                )),
                sync_time(),
            );
            assert_eq!(record.subject, expected);
            assert_eq!(record.from_name.as_deref(), Some("林晓"));
            assert_eq!(record.from_address, "lin@example.com");
        }
    }

    #[test]
    fn expands_address_groups_and_retains_all_header_metadata() {
        let record = parse_header(
            &remote(concat!(
                "From: First <first@example.com>, Second <second@example.com>\r\n",
                "To: Team: Alice <alice@example.com>, bob@example.com; Other: carol@example.com;\r\n",
                "Cc: Dan <dan@example.com>\r\n",
                "Reply-To: Replies: reply@example.com, backup@example.com;\r\n",
                "Message-ID: <unique@example.com>\r\n\r\n",
            )),
            sync_time(),
        );
        assert_eq!(record.uid, 42);
        assert_eq!(record.from_name.as_deref(), Some("First"));
        assert_eq!(record.from_address, "first@example.com");
        assert_eq!(
            record
                .to_addresses
                .iter()
                .map(|address| address.address.as_str())
                .collect::<Vec<_>>(),
            ["alice@example.com", "bob@example.com", "carol@example.com"]
        );
        assert_eq!(record.to_addresses[0].name.as_deref(), Some("Alice"));
        assert_eq!(record.cc_addresses[0].address, "dan@example.com");
        assert_eq!(record.reply_to_addresses.len(), 2);
        assert_eq!(
            record.message_id_header.as_deref(),
            Some("unique@example.com")
        );
        assert_eq!(record.size, Some(1024));
        assert!(record.is_read && record.is_flagged && record.has_attachment);
        assert_eq!(record.flags, ["\\sEeN", "\\FLAGGED", "$Forwarded"]);
    }

    #[test]
    fn missing_subject_and_broken_from_do_not_discard_other_fields() {
        for broken in [
            "",
            "<>",
            "not an address",
            "Broken <>",
            "Broken <not an address>",
        ] {
            let record = parse_header(
                &remote(format!("From: {broken}\r\nTo: valid@example.com\r\n\r\n")),
                sync_time(),
            );
            assert_eq!(record.subject, "");
            assert_eq!(record.from_address, "");
            assert!(record.from_name.is_none());
            assert_eq!(record.to_addresses[0].address, "valid@example.com");
        }
        let record = parse_header(&remote(b"\xff\x00malformed\r\n\r\n"), sync_time());
        assert_eq!(record.received_at, "2026-10-18T04:05:06.789Z");
        assert!(record.sent_at.is_none());
    }

    #[test]
    fn received_time_prefers_internaldate_then_date_then_sync_time() {
        let mut message = remote("Date: Fri, 18 Sep 2026 10:00:00 +0800\r\n\r\n");
        let from_header = parse_header(&message, sync_time());
        assert_eq!(from_header.received_at, "2026-09-18T02:00:00.000Z");
        assert_eq!(
            from_header.sent_at.as_deref(),
            Some("2026-09-18T02:00:00.000Z")
        );
        message.internal_date =
            Some(DateTime::parse_from_rfc3339("2026-09-19T08:09:10.123-05:00").unwrap());
        let from_internal = parse_header(&message, sync_time());
        assert_eq!(from_internal.received_at, "2026-09-19T13:09:10.123Z");
        assert_eq!(from_internal.sent_at, from_header.sent_at);
        for header in [
            "",
            "Date: broken\r\n\r\n",
            "Date: 30 Feb 2026 10:00:00 +0800\r\n\r\n",
        ] {
            let fallback = parse_header(&remote(header), sync_time());
            assert_eq!(fallback.received_at, "2026-10-18T04:05:06.789Z");
            assert!(fallback.sent_at.is_none());
        }
    }

    #[test]
    fn canonical_utc_dates_sort_in_chronological_order_across_timezones() {
        let dates = [
            ("18 Sep 2026 00:30:00 +0800", "2026-09-17T16:30:00.000Z"),
            ("17 Sep 2026 17:00:00 +0000", "2026-09-17T17:00:00.000Z"),
            ("17 Sep 2026 10:30:00 -0700", "2026-09-17T17:30:00.000Z"),
        ];
        let records: Vec<_> = dates
            .iter()
            .map(|(date, expected)| {
                let record = parse_header(&remote(format!("Date: {date}\r\n\r\n")), sync_time());
                assert_eq!(&record.received_at, expected);
                record
            })
            .collect();
        for pair in records.windows(2) {
            assert!(pair[0].received_at < pair[1].received_at);
            assert!(
                DateTime::parse_from_rfc3339(&pair[0].received_at).unwrap()
                    < DateTime::parse_from_rfc3339(&pair[1].received_at).unwrap()
            );
        }
    }

    #[test]
    fn empty_flags_and_missing_metadata_remain_empty() {
        let mut message = remote("\r\n");
        message.flags.clear();
        message.size = None;
        message.has_attachment = false;
        let record = parse_header(&message, sync_time());
        assert!(!record.is_read && !record.is_flagged && !record.has_attachment);
        assert!(record.message_id_header.is_none());
        assert!(
            record.to_addresses.is_empty()
                && record.cc_addresses.is_empty()
                && record.reply_to_addresses.is_empty()
        );
        assert!(record.size.is_none());
    }
}
