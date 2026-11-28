//! China Mobile 139 Mail's IMAP server negotiates only TLS 1.2 with static RSA
//! key exchange, which rustls does not implement, so the handshake runs on the
//! platform TLS library. Its SMTP server offers ECDHE and stays on rustls.

/// Exact IMAP host names; similar-looking hosts keep the default stack.
pub(super) const PLATFORM_TLS_IMAP_HOSTS: &[&str] = &["imap.139.com"];
