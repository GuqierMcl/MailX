//! Endpoint-scoped provider compatibility implementations.

pub(crate) mod oauth;
mod qq;

use crate::mail::compatibility::{ProviderCompatibility, StandardCompatibility};

static STANDARD: StandardCompatibility = StandardCompatibility;
static QQ: qq::QqCompatibility = qq::QqCompatibility;

/// Manual accounts use the same adaptation as presets when they connect to the
/// same endpoint. Unknown endpoints retain standard RFC header interpretation.
pub(crate) fn for_imap_host(host: &str) -> &'static dyn ProviderCompatibility {
    if host.eq_ignore_ascii_case("imap.qq.com") {
        &QQ
    } else {
        &STANDARD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_requires_the_exact_imap_endpoint() {
        let qq = for_imap_host("imap.qq.com");
        assert_eq!(for_imap_host("IMAP.QQ.COM").policy_key(), qq.policy_key());
        for host in [
            "imap.example.com",
            "imap.qq.com.evil.example",
            "evil-imap.qq.com",
            "qq.com",
            "vip.qq.com",
            "imap.qq.com.",
            " imap.qq.com",
            "imap.qq.com:993",
        ] {
            let policy = for_imap_host(host);
            assert_eq!(policy.policy_key(), STANDARD.policy_key(), "{host}");
            assert!(policy.extra_header_fields().is_empty(), "{host}");
        }
    }

    #[test]
    fn unknown_endpoints_ignore_provider_headers() {
        let message = mail_parser::MessageParser::default()
            .parse(b"X-OQ-MSGID: <original@qq.com>\r\n\r\nbody")
            .unwrap();
        assert_eq!(
            for_imap_host("imap.example.com").original_message_id(&message),
            None
        );
    }
}
