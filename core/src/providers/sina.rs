//! Sina's IMAP servers negotiate only TLS 1.2 with finite-field DHE or static
//! RSA key exchange, neither of which rustls implements, so the handshake runs
//! on the platform TLS library. Its SMTP servers offer ECDHE and stay on rustls.

/// Exact IMAP host names; similar-looking hosts keep the default stack.
pub(super) const PLATFORM_TLS_IMAP_HOSTS: &[&str] =
    &["imap.sina.com", "imap.sina.cn", "imap.vip.sina.com"];

/// Real-server checks, run on demand by the "Provider TLS" workflow:
/// `cargo test -p mailx-core live_provider_tls -- --ignored --nocapture`.
#[cfg(test)]
mod live {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::net::{self, NetRoute, TlsStack};

    async fn greeting(host: &str, port: u16, stack: TlsStack) -> Result<String, net::NetError> {
        let stream = net::connect(&NetRoute::default(), host, port).await?;
        let mut stream = net::upgrade_tls_on(stream, host, stack).await?;
        let mut buffer = [0; 256];
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        let quit: &[u8] = if port == 993 {
            b"a LOGOUT\r\n"
        } else {
            b"QUIT\r\n"
        };
        let _ = stream.write_all(quit).await;
        Ok(String::from_utf8_lossy(&buffer[..read]).trim().to_owned())
    }

    #[tokio::test]
    #[ignore = "contacts real Sina servers"]
    async fn live_provider_tls_sina() {
        let endpoints = [
            ("imap.sina.com", 993, "* OK"),
            ("imap.sina.cn", 993, "* OK"),
            ("imap.vip.sina.com", 993, "* OK"),
            ("smtp.sina.com", 465, "220"),
            ("smtp.sina.cn", 465, "220"),
            ("smtp.vip.sina.com", 465, "220"),
        ];
        let mut failures = Vec::new();
        for (host, port, expected) in endpoints {
            let stack = if port == 993 {
                super::super::imap_tls_stack(host)
            } else {
                TlsStack::Rustls
            };
            match greeting(host, port, stack).await {
                Ok(text) if text.starts_with(expected) => {
                    println!("{host}:{port} {stack:?} OK -> {text}");
                }
                other => {
                    println!("{host}:{port} {stack:?} FAIL -> {other:?}");
                    failures.push(host);
                }
            }
            if stack == TlsStack::Platform {
                // Informational: if rustls starts working, the registration can go.
                let rustls = greeting(host, port, TlsStack::Rustls).await;
                println!(
                    "{host}:{port} Rustls (reference) -> {}",
                    match rustls {
                        Ok(_) => "OK, platform TLS may no longer be needed".to_owned(),
                        Err(error) => error.to_string(),
                    }
                );
            }
        }
        assert!(failures.is_empty(), "failed endpoints: {failures:?}");
    }
}
