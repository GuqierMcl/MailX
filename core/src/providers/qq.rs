//! QQ preserves the submitted Message-ID in X-OQ-MSGID when rewriting it.

use crate::mail::compatibility::ProviderCompatibility;

const ORIGINAL_MESSAGE_ID: &str = "X-OQ-MSGID";
const MAX_MESSAGE_ID_BYTES: usize = 998;

pub(super) struct QqCompatibility;

impl ProviderCompatibility for QqCompatibility {
    fn policy_key(&self) -> &'static str {
        // v2: In-Reply-To and References joined the base header set (see default:v2).
        "qq:v2"
    }

    fn extra_header_fields(&self) -> &'static [&'static str] {
        &[ORIGINAL_MESSAGE_ID]
    }

    fn original_message_id(&self, message: &mail_parser::Message<'_>) -> Option<String> {
        let mut headers = message.parts.first()?.headers.iter().filter(|header| {
            header
                .name
                .as_str()
                .eq_ignore_ascii_case(ORIGINAL_MESSAGE_ID)
        });
        let header = headers.next()?;
        if headers.next().is_some() {
            return None;
        }
        // Inspect original bytes: decoding/unfolding an unstructured header can
        // discard controls or make several values look like one identity.
        let raw = message
            .raw_message
            .get(header.offset_start as usize..header.offset_end as usize)?;
        let raw = std::str::from_utf8(raw).ok()?;
        let value = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        if value.len() > MAX_MESSAGE_ID_BYTES {
            return None;
        }
        let value = value.trim_matches(' ');
        let inner = value.strip_prefix('<')?.strip_suffix('>')?;
        let (left, right) = inner.split_once('@')?;
        // MailX creates dot-atom Message-IDs. Restrict reconciliation to that
        // unambiguous form instead of accepting obsolete or quoted syntax.
        if !dot_atom(left) || !dot_atom(right) {
            return None;
        }
        Some(inner.to_owned())
    }
}

fn dot_atom(value: &str) -> bool {
    value.split('.').all(|atom| {
        !atom.is_empty()
            && atom.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'/'
                            | b'='
                            | b'?'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'{'
                            | b'|'
                            | b'}'
                            | b'~'
                    )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original(headers: &str) -> Option<String> {
        let raw = format!("Message-ID: <rewritten@qq.com>\r\n{headers}\r\nbody");
        let message = mail_parser::MessageParser::default().parse(raw.as_bytes())?;
        QqCompatibility.original_message_id(&message)
    }

    #[test]
    fn reads_original_id_for_standard_and_vip_qq_senders_without_changing_case() {
        for id in ["A8b-123@qq.com", "A8b-123@vip.qq.com", "A8b-123@VIP.QQ.COM"] {
            assert_eq!(
                original(&format!("x-oq-msgid: <{id}>\r\n")),
                Some(id.into())
            );
        }
        assert_eq!(original("Subject: no provider identity\r\n"), None);
    }

    #[test]
    fn rejects_duplicate_and_multiple_original_ids() {
        for headers in [
            "X-OQ-MSGID: <a@qq.com> <b@qq.com>\r\n",
            "X-OQ-MSGID: <a@qq.com>, <b@qq.com>\r\n",
            "X-OQ-MSGID: <a@qq.com>\r\nx-oq-msgid: <a@qq.com>\r\n",
            "X-OQ-MSGID: <a@qq.com>\r\nX-OQ-MSGID: \r\n",
        ] {
            assert_eq!(original(headers), None, "{headers:?}");
        }
    }

    #[test]
    fn rejects_malformed_control_and_oversized_values() {
        for value in [
            "",
            "a@qq.com",
            "<>",
            "<a>",
            "<@qq.com>",
            "<a@>",
            "<a@@qq.com>",
            "<<a@qq.com>>",
            "<a b@qq.com>",
            "<a@q..com>",
            "<.a@qq.com>",
            "<a@qq.com.>",
            "<用户@qq.com>",
            "<a\t@qq.com>",
            "\t<a@qq.com>",
            "<a\0@qq.com>",
            "<a\u{7f}@qq.com>",
            "<a\r\n @qq.com>",
            "<a@qq.com> (comment)",
            "=?UTF-8?Q?<a@qq.com>?=",
        ] {
            assert_eq!(
                original(&format!("X-OQ-MSGID: {value}\r\n")),
                None,
                "{value:?}"
            );
        }
        let long = format!("<{}@qq.com>", "a".repeat(MAX_MESSAGE_ID_BYTES));
        assert_eq!(original(&format!("X-OQ-MSGID: {long}\r\n")), None);
        let padding = " ".repeat(MAX_MESSAGE_ID_BYTES);
        assert_eq!(
            original(&format!("X-OQ-MSGID: {padding}<a@qq.com>\r\n")),
            None
        );
    }
}
