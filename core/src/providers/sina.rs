//! Sina's IMAP servers negotiate only TLS 1.2 with finite-field DHE or static
//! RSA key exchange, neither of which rustls implements, so the handshake runs
//! on the platform TLS library. Its SMTP servers offer ECDHE and stay on rustls.

/// Exact IMAP host names; similar-looking hosts keep the default stack.
pub(super) const PLATFORM_TLS_IMAP_HOSTS: &[&str] = &[
    "imap.sina.com",
    "imap.sina.cn",
    "imap.vip.sina.com",
    "imap.vip.sina.cn",
];
