//! Endpoint-scoped provider compatibility implementations.

#[cfg(test)]
mod live_tls;
mod mail139;
mod microsoft;
pub(crate) mod oauth;
mod qq;
mod sina;

use crate::mail::compatibility::{ProviderCompatibility, StandardCompatibility};
use crate::net::TlsStack;

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

/// TLS stack for an IMAP endpoint. Only exact registered hosts leave rustls;
/// the choice never changes the account route or certificate verification.
pub(crate) fn imap_tls_stack(host: &str) -> TlsStack {
    if [
        sina::PLATFORM_TLS_IMAP_HOSTS,
        mail139::PLATFORM_TLS_IMAP_HOSTS,
    ]
    .into_iter()
    .flatten()
    .any(|known| host.eq_ignore_ascii_case(known))
    {
        TlsStack::Platform
    } else {
        TlsStack::Rustls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_tls_requires_an_exact_registered_imap_host() {
        for host in [
            "imap.sina.com",
            "IMAP.SINA.CN",
            "imap.vip.sina.com",
            "imap.vip.sina.cn",
            "IMAP.139.COM",
        ] {
            assert_eq!(imap_tls_stack(host), TlsStack::Platform, "{host}");
        }
        for host in [
            "smtp.sina.com",
            "imap.sina.com.evil.example",
            "evil-imap.sina.com",
            "sina.com",
            "imap.sina.com.",
            " imap.sina.com",
            "imap.sina.com:993",
            "imap.qq.com",
            "smtp.139.com",
            "imap.139.com.evil.example",
        ] {
            assert_eq!(imap_tls_stack(host), TlsStack::Rustls, "{host}");
        }
    }

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

    #[test]
    fn policy_key_bump_triggers_resync() {
        // v1 stamps predate In-Reply-To / References; they must not match.
        assert_ne!(for_imap_host("imap.example.com").policy_key(), "default:v1");
        assert_ne!(for_imap_host("imap.qq.com").policy_key(), "qq:v1");
        assert_ne!(
            for_imap_host("imap.qq.com").policy_key(),
            for_imap_host("imap.example.com").policy_key()
        );
    }
}
