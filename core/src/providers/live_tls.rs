//! Real-server TLS checks, run on demand by the "Provider TLS" workflow:
//! `cargo test -p mailx-core live_provider_tls -- --ignored --nocapture`.
//!
//! Every endpoint must greet through the production stack choice: registered
//! IMAP hosts use platform TLS, everything else starts on rustls and falls back
//! through `net::with_tls_fallback`. No login is made.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::net::{self, BoxedStream, NetRoute, TlsStack};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Imap,
    Smtps,
    SmtpStartTls,
}

/// IMAP and SMTP endpoints the account presets and OAuth providers use.
const ENDPOINTS: &[(&str, u16, Kind)] = &[
    ("imap.qq.com", 993, Kind::Imap),
    ("smtp.qq.com", 465, Kind::Smtps),
    ("imap.163.com", 993, Kind::Imap),
    ("smtp.163.com", 465, Kind::Smtps),
    ("imap.126.com", 993, Kind::Imap),
    ("smtp.126.com", 465, Kind::Smtps),
    ("imap.yeah.net", 993, Kind::Imap),
    ("smtp.yeah.net", 465, Kind::Smtps),
    ("imap.139.com", 993, Kind::Imap),
    ("smtp.139.com", 465, Kind::Smtps),
    ("imap.sina.com", 993, Kind::Imap),
    ("smtp.sina.com", 465, Kind::Smtps),
    ("imap.sina.cn", 993, Kind::Imap),
    ("smtp.sina.cn", 465, Kind::Smtps),
    ("imap.vip.sina.com", 993, Kind::Imap),
    ("smtp.vip.sina.com", 465, Kind::Smtps),
    ("imap.vip.sina.cn", 993, Kind::Imap),
    ("smtp.vip.sina.cn", 465, Kind::Smtps),
    ("imap.feishu.cn", 993, Kind::Imap),
    ("smtp.feishu.cn", 465, Kind::Smtps),
    ("imap.larksuite.com", 993, Kind::Imap),
    ("smtp.larksuite.com", 465, Kind::Smtps),
    ("imap.gmail.com", 993, Kind::Imap),
    ("smtp.gmail.com", 465, Kind::Smtps),
    ("imap.mail.me.com", 993, Kind::Imap),
    ("smtp.mail.me.com", 587, Kind::SmtpStartTls),
    ("outlook.office365.com", 993, Kind::Imap),
    ("smtp-mail.outlook.com", 587, Kind::SmtpStartTls),
    ("smtp.office365.com", 587, Kind::SmtpStartTls),
    // Not a preset: an unregistered server that only works through the fallback.
    ("smtp.wo.cn", 465, Kind::Smtps),
];

async fn line(reader: &mut BufReader<BoxedStream>) -> String {
    let mut text = String::new();
    let _ = reader.read_line(&mut text).await;
    text.trim_end().to_owned()
}

/// Reads one SMTP reply (possibly multi-line) and returns its last line.
async fn reply(reader: &mut BufReader<BoxedStream>) -> String {
    loop {
        let text = line(reader).await;
        if text.len() < 4 || text.as_bytes()[3] != b'-' {
            return text;
        }
    }
}

async fn probe(host: &str, port: u16, kind: Kind, stack: TlsStack) -> Result<String, String> {
    let fail = |error: net::NetError| error.to_string();
    let mut stream = net::connect(&NetRoute::default(), host, port)
        .await
        .map_err(fail)?;
    if kind == Kind::SmtpStartTls {
        let mut reader = BufReader::new(stream);
        for (command, expected) in [
            (None, "220"),
            (Some("EHLO mailx.invalid"), "250"),
            (Some("STARTTLS"), "220"),
        ] {
            if let Some(command) = command {
                let _ = reader
                    .get_mut()
                    .write_all(format!("{command}\r\n").as_bytes())
                    .await;
            }
            let text = reply(&mut reader).await;
            if !text.starts_with(expected) {
                return Err(format!("before TLS: {text}"));
            }
        }
        stream = reader.into_inner();
    }
    let stream = net::upgrade_tls_on(stream, host, stack)
        .await
        .map_err(fail)?;
    let mut reader = BufReader::new(stream);
    let (greeting, expected, quit) = match kind {
        Kind::Imap => (line(&mut reader).await, "* OK", "a LOGOUT\r\n"),
        Kind::Smtps => (reply(&mut reader).await, "220", "QUIT\r\n"),
        Kind::SmtpStartTls => {
            // STARTTLS has no second greeting; EHLO proves the TLS channel.
            let _ = reader.get_mut().write_all(b"EHLO mailx.invalid\r\n").await;
            (reply(&mut reader).await, "250", "QUIT\r\n")
        }
    };
    let _ = reader.get_mut().write_all(quit.as_bytes()).await;
    if greeting.starts_with(expected) {
        Ok(greeting)
    } else {
        Err(format!("unexpected greeting: {greeting}"))
    }
}

fn registered_stack(host: &str, kind: Kind) -> TlsStack {
    match kind {
        Kind::Imap => super::imap_tls_stack(host),
        Kind::Smtps | Kind::SmtpStartTls => TlsStack::Rustls,
    }
}

async fn check(endpoints: impl IntoIterator<Item = (String, u16, Kind)>) -> Vec<String> {
    let mut failures = Vec::new();
    for (host, port, kind) in endpoints {
        let used = std::sync::Mutex::new(TlsStack::Rustls);
        let result = net::with_tls_fallback(
            &host,
            port,
            registered_stack(&host, kind),
            |stack| {
                *used.lock().unwrap() = stack;
                probe(&host, port, kind, stack)
            },
            |error: &String| error.contains("no TLS parameters in common"),
        )
        .await;
        let stack = used.into_inner().unwrap();
        match result {
            Ok(text) => println!("{host}:{port} {stack:?} OK -> {text}"),
            Err(error) => {
                println!("{host}:{port} {stack:?} FAIL -> {error}");
                failures.push(format!("{host}:{port}"));
            }
        }
        if stack == TlsStack::Platform {
            // Informational: if rustls starts working, platform TLS is not needed.
            let rustls = match probe(&host, port, kind, TlsStack::Rustls).await {
                Ok(_) => "OK, platform TLS may no longer be needed".to_owned(),
                Err(error) => error,
            };
            println!("{host}:{port} Rustls (reference) -> {rustls}");
        }
    }
    failures
}

#[tokio::test]
#[ignore = "contacts real mail servers"]
async fn live_provider_tls_presets() {
    let endpoints = ENDPOINTS
        .iter()
        .map(|&(host, port, kind)| (host.to_owned(), port, kind));
    let failures = check(endpoints).await;
    assert!(failures.is_empty(), "failed endpoints: {failures:?}");
}

/// Ad-hoc survey of other servers, e.g. before registering a new provider:
/// `MAILX_TLS_SURVEY="imap.example.com:993 smtp.example.com:587" cargo test ...`.
/// Port 993 is IMAP, 587 SMTP STARTTLS, anything else implicit-TLS SMTP.
#[tokio::test]
#[ignore = "contacts real mail servers"]
async fn live_provider_tls_survey() {
    let Ok(list) = std::env::var("MAILX_TLS_SURVEY") else {
        return;
    };
    let endpoints = list.split_whitespace().filter_map(|entry| {
        let (host, port) = entry.rsplit_once(':')?;
        let port: u16 = port.parse().ok()?;
        let kind = match port {
            993 => Kind::Imap,
            587 => Kind::SmtpStartTls,
            _ => Kind::Smtps,
        };
        Some((host.to_owned(), port, kind))
    });
    let failures = check(endpoints).await;
    assert!(failures.is_empty(), "failed endpoints: {failures:?}");
}
