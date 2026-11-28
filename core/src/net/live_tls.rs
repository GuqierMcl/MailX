//! Real-server TLS check, run on demand by the "Provider TLS" workflow:
//! `cargo test -p mailx-core live_provider_tls -- --ignored --nocapture`.
//!
//! Each endpoint connects through the production path (`with_tls_fallback`)
//! and must greet; no login is made. `MAILX_TLS_SURVEY="host:port ..."` checks
//! other servers instead (993 is IMAP, 587 SMTP STARTTLS, else implicit SMTP).

use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{BoxedStream, NetError, NetRoute, TlsStack};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Imap,
    Smtps,
    SmtpStartTls,
}

impl Kind {
    fn for_port(port: u16) -> Self {
        match port {
            993 => Self::Imap,
            587 => Self::SmtpStartTls,
            _ => Self::Smtps,
        }
    }
}

/// IMAP and SMTP endpoints of the account presets and OAuth providers.
const PRESET_ENDPOINTS: &[(&str, u16)] = &[
    ("imap.qq.com", 993),
    ("smtp.qq.com", 465),
    ("imap.163.com", 993),
    ("smtp.163.com", 465),
    ("imap.126.com", 993),
    ("smtp.126.com", 465),
    ("imap.yeah.net", 993),
    ("smtp.yeah.net", 465),
    ("imap.139.com", 993),
    ("smtp.139.com", 465),
    ("imap.sina.com", 993),
    ("smtp.sina.com", 465),
    ("imap.sina.cn", 993),
    ("smtp.sina.cn", 465),
    ("imap.feishu.cn", 993),
    ("smtp.feishu.cn", 465),
    ("imap.larksuite.com", 993),
    ("smtp.larksuite.com", 465),
    ("imap.gmail.com", 993),
    ("smtp.gmail.com", 465),
    ("imap.mail.me.com", 993),
    ("smtp.mail.me.com", 587),
    ("outlook.office365.com", 993),
    ("smtp-mail.outlook.com", 587),
    ("smtp.office365.com", 587),
];

#[derive(Debug)]
enum ProbeError {
    Net(NetError),
    Reply(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(error) => error.fmt(formatter),
            Self::Reply(text) => write!(formatter, "unexpected reply: {text}"),
        }
    }
}

type Reader = BufReader<BoxedStream>;

async fn line(reader: &mut Reader) -> String {
    let mut text = String::new();
    let _ = reader.read_line(&mut text).await;
    text.trim_end().to_owned()
}

/// Reads one SMTP reply (possibly multi-line) and returns its last line.
async fn reply(reader: &mut Reader) -> String {
    loop {
        let text = line(reader).await;
        if text.len() < 4 || text.as_bytes()[3] != b'-' {
            return text;
        }
    }
}

async fn command(reader: &mut Reader, text: &str, expected: &str) -> Result<String, ProbeError> {
    if !text.is_empty() {
        let _ = reader.get_mut().write_all(text.as_bytes()).await;
    }
    let answer = reply(reader).await;
    if answer.starts_with(expected) {
        Ok(answer)
    } else {
        Err(ProbeError::Reply(answer))
    }
}

/// Connects, upgrades with `stack` and reads the greeting (after STARTTLS, the
/// `EHLO` answer), then says goodbye.
async fn probe(host: &str, port: u16, stack: TlsStack) -> Result<String, ProbeError> {
    let kind = Kind::for_port(port);
    let mut stream = super::connect(&NetRoute::default(), host, port)
        .await
        .map_err(ProbeError::Net)?;
    if kind == Kind::SmtpStartTls {
        let mut reader = BufReader::new(stream);
        command(&mut reader, "", "220").await?;
        command(&mut reader, "EHLO mailx.invalid\r\n", "250").await?;
        command(&mut reader, "STARTTLS\r\n", "220").await?;
        stream = reader.into_inner();
    }
    let stream = super::upgrade_tls_on(stream, host, stack)
        .await
        .map_err(ProbeError::Net)?;
    let mut reader = BufReader::new(stream);
    let greeting = match kind {
        Kind::Imap => {
            let greeting = line(&mut reader).await;
            let _ = reader.get_mut().write_all(b"a LOGOUT\r\n").await;
            if !greeting.starts_with("* OK") {
                return Err(ProbeError::Reply(greeting));
            }
            greeting
        }
        Kind::Smtps => command(&mut reader, "", "220").await?,
        Kind::SmtpStartTls => command(&mut reader, "EHLO mailx.invalid\r\n", "250").await?,
    };
    if kind != Kind::Imap {
        let _ = reader.get_mut().write_all(b"QUIT\r\n").await;
    }
    Ok(greeting)
}

/// One production-path connection; returns the result and the stack used last.
async fn connect(host: &str, port: u16) -> (Result<String, ProbeError>, TlsStack) {
    let used = Mutex::new(TlsStack::Rustls);
    let result = super::with_tls_fallback(
        host,
        port,
        |stack| {
            *used.lock().unwrap() = stack;
            probe(host, port, stack)
        },
        |error| matches!(error, ProbeError::Net(error) if error.is_tls_incompatible()),
    )
    .await;
    (result, used.into_inner().unwrap())
}

#[tokio::test]
#[ignore = "contacts real mail servers"]
async fn live_provider_tls() {
    let survey = std::env::var("MAILX_TLS_SURVEY").ok();
    let endpoints: Vec<(String, u16)> = match &survey {
        Some(list) => list
            .split_whitespace()
            .filter_map(|entry| {
                let (host, port) = entry.rsplit_once(':')?;
                Some((host.to_owned(), port.parse().ok()?))
            })
            .collect(),
        None => PRESET_ENDPOINTS
            .iter()
            .map(|&(host, port)| (host.to_owned(), port))
            .collect(),
    };
    let mut failures = Vec::new();
    for (host, port) in endpoints {
        let (mut result, mut stack) = connect(&host, port).await;
        if let Err(error) = &result {
            // Shared runners see transient network failures; retry once.
            println!("{host}:{port} {stack:?} retrying after -> {error}");
            tokio::time::sleep(Duration::from_secs(3)).await;
            (result, stack) = connect(&host, port).await;
        }
        match result {
            Ok(greeting) => println!("{host}:{port} {stack:?} OK -> {greeting}"),
            Err(error) => {
                println!("{host}:{port} {stack:?} FAIL -> {error}");
                failures.push(format!("{host}:{port}"));
            }
        }
    }
    assert!(failures.is_empty(), "failed endpoints: {failures:?}");
}
