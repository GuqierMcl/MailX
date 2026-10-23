//! A scriptable in-process SMTP server for tests. It listens on 127.0.0.1 only
//! and records what it saw in order, including when the test's
//! `on_data_started` hook ran relative to the first body byte.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Stage {
    Greeting,
    Ehlo,
    Auth,
    Mail,
    Rcpt,
    Data,
    Final,
}

#[derive(Clone)]
pub(crate) enum Mode {
    Plain,
    /// Offers STARTTLS (unless `offer_starttls` is off) and upgrades with this config.
    StartTls(Arc<rustls::ServerConfig>),
    /// TLS from the first byte.
    Implicit(Arc<rustls::ServerConfig>),
}

#[derive(Clone)]
pub(crate) struct Script {
    pub mode: Mode,
    pub offer_starttls: bool,
    /// Mechanisms advertised in EHLO; empty advertises no AUTH at all.
    pub auth: Vec<&'static str>,
    pub password: &'static str,
    pub size: Option<u64>,
    /// Canned replies replacing the normal answer at a stage.
    pub replies: HashMap<Stage, &'static str>,
    /// Recipients answered with `550`.
    pub reject_rcpt: Vec<&'static str>,
    /// Never answer at this stage (the connection stays open).
    pub stall: Option<Stage>,
    /// Close without a reply once the body terminator arrived.
    pub drop_after_body: bool,
    /// Bytes written in the same segment as the `220` to STARTTLS.
    pub inject_after_starttls: Vec<u8>,
    pub final_reply: &'static str,
}

impl Script {
    pub(crate) fn new(mode: Mode) -> Self {
        Self {
            mode,
            offer_starttls: true,
            auth: vec!["PLAIN", "LOGIN"],
            password: "hunter2",
            size: None,
            replies: HashMap::new(),
            reject_rcpt: Vec::new(),
            stall: None,
            drop_after_body: false,
            inject_after_starttls: Vec::new(),
            final_reply: "250 2.0.0 queued as 42",
        }
    }

    pub(crate) fn reply(mut self, stage: Stage, line: &'static str) -> Self {
        self.replies.insert(stage, line);
        self
    }
}

#[derive(Default, Debug)]
pub(crate) struct SeenInner {
    /// Commands and markers in arrival order. `BODY` is the first body line;
    /// `HOOK` is pushed by the test's `on_data_started`.
    pub log: Vec<String>,
    /// The body after dot un-stuffing, without the terminator line.
    pub body: Vec<u8>,
    pub connections: usize,
    pub tls_handshakes_failed: usize,
}

pub(crate) type Seen = Arc<Mutex<SeenInner>>;

pub(crate) fn saw(seen: &Seen, prefix: &str) -> bool {
    seen.lock()
        .unwrap()
        .log
        .iter()
        .any(|entry| entry.starts_with(prefix))
}

pub(crate) fn commands(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().log.clone()
}

pub(crate) async fn start(script: Script) -> (u16, Seen) {
    start_at(0, script).await
}

/// Like `start`, on a given port (0 picks one), so a test can bring a server
/// up where connections were refused a moment ago.
pub(crate) async fn start_at(port: u16, script: Script) -> (u16, Seen) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Seen::default();
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            record.lock().unwrap().connections += 1;
            let script = script.clone();
            let record = record.clone();
            tokio::spawn(async move {
                if let Mode::Implicit(config) = &script.mode {
                    match tokio_rustls::TlsAcceptor::from(config.clone())
                        .accept(socket)
                        .await
                    {
                        Ok(tls) => serve(Box::new(tls), script, record).await,
                        Err(_) => record.lock().unwrap().tls_handshakes_failed += 1,
                    }
                } else {
                    serve(Box::new(socket), script, record).await;
                }
            });
        }
    });
    (port, seen)
}

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

async fn stall() {
    tokio::time::sleep(Duration::from_secs(3600)).await;
}

async fn serve(socket: Box<dyn Duplex>, script: Script, seen: Seen) {
    let _ = run(socket, script, seen).await;
}

async fn say(socket: &mut BufReader<Box<dyn Duplex>>, text: &str) -> std::io::Result<()> {
    socket.get_mut().write_all(text.as_bytes()).await?;
    socket.get_mut().flush().await
}

async fn read_line(socket: &mut BufReader<Box<dyn Duplex>>) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    Ok((socket.read_line(&mut line).await? > 0).then_some(line))
}

async fn run(socket: Box<dyn Duplex>, script: Script, seen: Seen) -> std::io::Result<()> {
    let mut socket = BufReader::new(socket);
    let log = |entry: String| seen.lock().unwrap().log.push(entry);
    let canned = |stage: Stage| script.replies.get(&stage).copied();

    if script.stall == Some(Stage::Greeting) {
        stall().await;
    }
    say(
        &mut socket,
        &format!(
            "{}\r\n",
            canned(Stage::Greeting).unwrap_or("220 fake ESMTP")
        ),
    )
    .await?;
    if canned(Stage::Greeting).is_some_and(|reply| !reply.starts_with('2')) {
        return Ok(());
    }

    let mut starttls = match &script.mode {
        Mode::StartTls(config) => Some(config.clone()),
        _ => None,
    };
    let mut encrypted = matches!(script.mode, Mode::Implicit(_));
    while let Some(line) = read_line(&mut socket).await? {
        let command = line.trim_end().to_owned();
        let upper = command.to_ascii_uppercase();
        // Secrets are not kept in the log.
        log(if upper.starts_with("AUTH") {
            "AUTH".to_owned()
        } else {
            command.clone()
        });

        if upper.starts_with("EHLO") {
            if script.stall == Some(Stage::Ehlo) {
                stall().await;
            }
            if let Some(reply) = canned(Stage::Ehlo) {
                say(&mut socket, &format!("{reply}\r\n")).await?;
                continue;
            }
            let mut lines = vec!["fake greets you".to_owned()];
            if starttls.is_some() && !encrypted && script.offer_starttls {
                lines.push("STARTTLS".into());
            }
            if !script.auth.is_empty() {
                lines.push(format!("AUTH {}", script.auth.join(" ")));
            }
            if let Some(size) = script.size {
                lines.push(format!("SIZE {size}"));
            }
            lines.push("8BITMIME".into());
            let last = lines.len() - 1;
            let text: String = lines
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    format!("250{}{text}\r\n", if index == last { ' ' } else { '-' })
                })
                .collect();
            say(&mut socket, &text).await?;
        } else if upper.starts_with("STARTTLS") {
            let Some(config) = starttls.take() else {
                say(&mut socket, "502 no\r\n").await?;
                continue;
            };
            let mut reply = b"220 ready\r\n".to_vec();
            reply.extend_from_slice(&script.inject_after_starttls);
            socket.get_mut().write_all(&reply).await?;
            socket.get_mut().flush().await?;
            let plain = socket.into_inner();
            match tokio_rustls::TlsAcceptor::from(config).accept(plain).await {
                Ok(tls) => {
                    socket = BufReader::new(Box::new(tls));
                    encrypted = true;
                }
                Err(_) => {
                    seen.lock().unwrap().tls_handshakes_failed += 1;
                    return Ok(());
                }
            }
        } else if upper.starts_with("AUTH") {
            if script.stall == Some(Stage::Auth) {
                stall().await;
            }
            let credentials = if upper.starts_with("AUTH PLAIN ") {
                let token = command["AUTH PLAIN ".len()..].trim();
                STANDARD
                    .decode(token)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .and_then(|text| text.split('\0').nth(2).map(str::to_owned))
            } else if upper.starts_with("AUTH LOGIN") {
                say(&mut socket, "334 VXNlcm5hbWU6\r\n").await?;
                read_line(&mut socket).await?;
                say(&mut socket, "334 UGFzc3dvcmQ6\r\n").await?;
                let password = read_line(&mut socket).await?.unwrap_or_default();
                STANDARD
                    .decode(password.trim())
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            } else {
                None
            };
            if let Some(reply) = canned(Stage::Auth) {
                say(&mut socket, &format!("{reply}\r\n")).await?;
            } else if credentials.as_deref() == Some(script.password) {
                log(format!("AUTHOK:{}", upper.split(' ').nth(1).unwrap_or("")));
                say(&mut socket, "235 2.7.0 ok\r\n").await?;
            } else {
                say(
                    &mut socket,
                    &format!(
                        "535 5.7.8 bad credentials {}\r\n",
                        credentials.unwrap_or_default()
                    ),
                )
                .await?;
            }
        } else if upper.starts_with("MAIL FROM") {
            if script.stall == Some(Stage::Mail) {
                stall().await;
            }
            let reply = canned(Stage::Mail).unwrap_or("250 ok");
            say(&mut socket, &format!("{reply}\r\n")).await?;
        } else if upper.starts_with("RCPT TO") {
            if script.stall == Some(Stage::Rcpt) {
                stall().await;
            }
            let rejected = script.reject_rcpt.iter().any(|address| {
                command
                    .to_ascii_lowercase()
                    .contains(&address.to_ascii_lowercase())
            });
            let reply = if rejected {
                "550 5.1.1 no such user"
            } else {
                canned(Stage::Rcpt).unwrap_or("250 ok")
            };
            say(&mut socket, &format!("{reply}\r\n")).await?;
        } else if upper.starts_with("RSET") {
            say(&mut socket, "250 reset\r\n").await?;
        } else if upper.starts_with("DATA") {
            if script.stall == Some(Stage::Data) {
                stall().await;
            }
            if let Some(reply) = canned(Stage::Data) {
                say(&mut socket, &format!("{reply}\r\n")).await?;
                continue;
            }
            say(&mut socket, "354 go ahead\r\n").await?;
            let mut first = true;
            loop {
                let mut raw = Vec::new();
                if socket.read_until(b'\n', &mut raw).await? == 0 {
                    return Ok(());
                }
                if first {
                    log("BODY".to_owned());
                    first = false;
                }
                if raw == b".\r\n" {
                    break;
                }
                let unstuffed = raw.strip_prefix(b".").unwrap_or(&raw).to_vec();
                seen.lock().unwrap().body.extend_from_slice(&unstuffed);
            }
            log("BODY-END".to_owned());
            if script.drop_after_body {
                return Ok(());
            }
            if script.stall == Some(Stage::Final) {
                stall().await;
            }
            let reply = canned(Stage::Final).unwrap_or(script.final_reply);
            say(&mut socket, &format!("{reply}\r\n")).await?;
        } else if upper.starts_with("QUIT") {
            say(&mut socket, "221 bye\r\n").await?;
            return Ok(());
        } else {
            say(&mut socket, "250 ok\r\n").await?;
        }
    }
    Ok(())
}
