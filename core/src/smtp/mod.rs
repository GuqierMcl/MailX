//! SMTP submission session (docs/architecture/sending.md §4).
//!
//! A small hand-written client over the byte stream `net::connect` opens, so
//! the account route, the no-fallback rule and certificate verification stay in
//! `net` (`lettre` is used for MIME only, see `message`). The session never
//! sends a password or a message byte over an unencrypted stream, never
//! continues after a failed or missing STARTTLS, and reports exactly how far
//! it got, because "the server may already have the message" and "the server
//! certainly has nothing" lead to different retry rules (sending.md §5).
//!
//! Nothing here logs. Reply texts come from the server and may contain
//! addresses, so outcomes carry only the reply code plus a sanitized,
//! length-limited text.

pub(crate) mod attachment;
#[cfg(test)]
pub(crate) mod fake_server;
pub(crate) mod message;
mod plan;
mod reply;
#[cfg(test)]
mod tests;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufReader};
use tokio::time::timeout;

use crate::accounts::types::TlsMode;
use crate::net::{self, BoxedStream, NetError, NetRoute};
use crate::secrets::SecretValue;
use message::SmtpEnvelope;
pub(crate) use plan::{SmtpPlan, plan_failure};
pub(crate) use reply::names_size_limit;
use reply::{IoFail, Reply, read_reply, sanitize};

/// Greeting identity. An address literal avoids leaking the machine's host name.
const EHLO_NAME: &str = "[127.0.0.1]";

/// Server and credentials for one SMTP login. The trust check against the
/// auth profile has already happened when this is built (`SmtpPlan`).
#[derive(Debug)]
pub(crate) struct SmtpLogin {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub username: String,
    pub password: SecretValue,
}

/// Per-step budgets. Defaults follow RFC 5321 §4.5.3.2.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timeouts {
    pub greeting: Duration,
    /// EHLO, STARTTLS, AUTH, MAIL, RCPT, RSET.
    pub command: Duration,
    /// From `DATA` to the 354 reply.
    pub data_initiation: Duration,
    /// Longest the body may go without a single byte being accepted by the
    /// server. It restarts on every byte written and is not a total cap: a slow
    /// but moving transfer of any size may take as long as it needs
    /// (sending.md section 16.6).
    pub data_write: Duration,
    /// From the end of the body to the final reply.
    pub final_reply: Duration,
    /// Best-effort `QUIT`.
    pub quit: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            greeting: Duration::from_secs(300),
            command: Duration::from_secs(300),
            data_initiation: Duration::from_secs(120),
            data_write: Duration::from_secs(120),
            final_reply: Duration::from_secs(600),
            quit: Duration::from_secs(5),
        }
    }
}

#[cfg(test)]
impl Timeouts {
    pub(crate) fn for_tests(each: Duration) -> Self {
        Self {
            greeting: each,
            command: each,
            data_initiation: each,
            data_write: each,
            final_reply: each,
            quit: each,
        }
    }
}

/// Why a session stopped before the server saw any message byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureReason {
    /// Direct connection failed.
    Connect,
    /// The proxy refused or could not reach the target. `net` does not expose a
    /// structured reason, so proxy authentication failures are not told apart
    /// and are treated as transient.
    Proxy,
    Timeout,
    /// Connection lost or I/O error mid-session.
    Network,
    /// Unparsable or unexpected server behaviour, including data after the
    /// STARTTLS reply.
    Protocol,
    /// A 4xx / 5xx reply; see the code.
    Server,
    /// The TLS handshake or certificate verification failed.
    TlsVerification,
    /// STARTTLS mode, but the server does not offer STARTTLS.
    TlsUnavailable,
    /// The server offers no AUTH mechanism MailX supports.
    AuthUnavailable,
    AuthRejected,
    MessageTooLarge,
    CredentialUntrusted,
    CredentialMissing,
    SecretStore,
    Account,
    /// The caller could not record "body started"; nothing was sent.
    DataMarkFailed,
    /// The message or envelope cannot be sent as built.
    InvalidMessage,
}

impl fmt::Display for FailureReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connect => "connect",
            Self::Proxy => "proxy",
            Self::Timeout => "timeout",
            Self::Network => "network",
            Self::Protocol => "protocol",
            Self::Server => "server",
            Self::TlsVerification => "tlsVerification",
            Self::TlsUnavailable => "tlsUnavailable",
            Self::AuthUnavailable => "authUnavailable",
            Self::AuthRejected => "authRejected",
            Self::MessageTooLarge => "messageTooLarge",
            Self::CredentialUntrusted => "credentialUntrusted",
            Self::CredentialMissing => "credentialMissing",
            Self::SecretStore => "secretStore",
            Self::Account => "account",
            Self::DataMarkFailed => "dataMarkFailed",
            Self::InvalidMessage => "invalidMessage",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreDataFailure {
    pub reason: FailureReason,
    pub code: Option<u16>,
    /// Sanitized server text, at most [`reply::MAX_TEXT_CHARS`] characters.
    pub text: Option<String>,
}

impl PreDataFailure {
    pub(crate) fn new(reason: FailureReason) -> Self {
        Self {
            reason,
            code: None,
            text: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerReply {
    pub code: u16,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RejectedRecipient {
    pub address: String,
    pub code: u16,
    pub text: String,
}

/// What happened after the body started, when no reply was usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnknownReason {
    Timeout,
    ConnectionLost,
    MalformedReply,
}

/// The result of one attempt. Only `Transient` is safe to retry automatically
/// and only `Accepted` means the server took responsibility for the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SmtpOutcome {
    /// Before the body started: the server has nothing; retry later.
    Transient(PreDataFailure),
    /// Before the body started: retrying will not help.
    Permanent(PreDataFailure),
    /// Before the body started: one or more recipients were refused. The
    /// session sent `RSET` and did not send the message to anyone.
    RecipientsRejected(Vec<RejectedRecipient>),
    /// After the body: 2xx final reply.
    Accepted(ServerReply),
    /// After the body: explicit 4xx / 5xx final reply. Not retried automatically.
    /// A 5xx whose text names a size limit is still this variant; the scheduler
    /// reads it with `names_size_limit` and records `messageTooLarge`
    /// (sending.md section 16.4).
    RejectedAfterData(ServerReply),
    /// After the body: no usable final reply. The server may have the message.
    Unknown(UnknownReason),
}

impl fmt::Display for SmtpOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transient(failure) | Self::Permanent(failure) => {
                let kind = if matches!(self, Self::Transient(_)) {
                    "transient"
                } else {
                    "permanent"
                };
                write!(formatter, "{kind} before data: {}", failure.reason)?;
                match (failure.code, &failure.text) {
                    (Some(code), Some(text)) => write!(formatter, " ({code} {text})"),
                    (Some(code), None) => write!(formatter, " ({code})"),
                    _ => Ok(()),
                }
            }
            Self::RecipientsRejected(list) => {
                write!(formatter, "{} recipient(s) rejected", list.len())
            }
            Self::Accepted(reply) => write!(formatter, "accepted ({} {})", reply.code, reply.text),
            Self::RejectedAfterData(reply) => {
                write!(
                    formatter,
                    "rejected after data ({} {})",
                    reply.code, reply.text
                )
            }
            Self::Unknown(reason) => write!(formatter, "unknown after data: {reason:?}"),
        }
    }
}

type DataStartedFuture = Pin<Box<dyn Future<Output = Result<(), ()>> + Send>>;

/// Called after the server's 354 and before the first message byte. The caller
/// persists its "body started" mark here; an `Err` aborts without sending.
pub(crate) struct DataStarted(Box<dyn FnOnce() -> DataStartedFuture + Send>);

impl DataStarted {
    pub(crate) fn new<F, Fut>(hook: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), ()>> + Send + 'static,
    {
        Self(Box::new(move || Box::pin(hook())))
    }
}

impl fmt::Debug for DataStarted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DataStarted")
    }
}

/// Where TLS handshakes get their trust decisions. Production always uses the
/// platform verifier through `net::upgrade_tls`; tests can substitute a trust
/// store for their throwaway CA. There is no variant that skips verification.
#[derive(Clone)]
pub(crate) enum TlsSource {
    Platform,
    #[cfg(test)]
    Test(std::sync::Arc<rustls::ClientConfig>),
}

impl TlsSource {
    async fn upgrade(&self, stream: BoxedStream, host: &str) -> Result<BoxedStream, NetError> {
        match self {
            Self::Platform => net::upgrade_tls(stream, host).await,
            #[cfg(test)]
            Self::Test(config) => net::upgrade_tls_with(stream, host, config.clone()).await,
        }
    }
}

/// Sends one message. Opens its own connection and always closes it. Production
/// passes `TlsSource::Platform`.
pub(crate) async fn send_with_tls(
    route: &NetRoute,
    login: &SmtpLogin,
    envelope: &SmtpEnvelope,
    mime: &[u8],
    timeouts: &Timeouts,
    tls: &TlsSource,
    on_data_started: DataStarted,
) -> Attempt {
    match session(route, login, envelope, mime, timeouts, tls, on_data_started).await {
        Ok(attempt) | Err(attempt) => attempt,
    }
}

struct Conn {
    reader: BufReader<BoxedStream>,
}

impl Conn {
    fn new(stream: BoxedStream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    /// Writes `bytes` (if any) and reads one reply, all within `budget`.
    async fn exchange(&mut self, bytes: Option<&[u8]>, budget: Duration) -> Result<Reply, IoFail> {
        let attempt = async {
            if let Some(bytes) = bytes {
                let stream = self.reader.get_mut();
                stream.write_all(bytes).await.map_err(|_| IoFail::Io)?;
                stream.flush().await.map_err(|_| IoFail::Io)?;
            }
            read_reply(&mut self.reader).await
        };
        timeout(budget, attempt)
            .await
            .map_err(|_| IoFail::Timeout)?
    }

    async fn command(&mut self, line: &str, budget: Duration) -> Result<Reply, IoFail> {
        self.exchange(Some(format!("{line}\r\n").as_bytes()), budget)
            .await
    }

    /// Best effort `QUIT`; the result is irrelevant.
    async fn quit(&mut self, budget: Duration) {
        let _ = self.command("QUIT", budget).await;
    }
}

fn transient(reason: FailureReason) -> SmtpOutcome {
    SmtpOutcome::Transient(PreDataFailure::new(reason))
}

fn permanent(reason: FailureReason) -> SmtpOutcome {
    SmtpOutcome::Permanent(PreDataFailure::new(reason))
}

fn net_outcome(error: NetError) -> SmtpOutcome {
    match error {
        NetError::Connect(_) => transient(FailureReason::Connect),
        NetError::Proxy(_) => transient(FailureReason::Proxy),
        NetError::Timeout => transient(FailureReason::Timeout),
        NetError::Tls(_) => permanent(FailureReason::TlsVerification),
    }
}

/// An I/O failure before the body started.
fn io_outcome(error: IoFail) -> SmtpOutcome {
    transient(match error {
        IoFail::Timeout => FailureReason::Timeout,
        IoFail::Io | IoFail::Eof => FailureReason::Network,
        IoFail::Malformed => FailureReason::Protocol,
    })
}

/// A reply that is not the expected one, before the body started. A 5xx at the
/// `MAIL FROM` or `DATA` stage whose text names a size limit becomes
/// `messageTooLarge`; a bare `552` stays `server`, because it can also mean a
/// full mailbox (sending.md section 16.4).
fn refusal(reply: &Reply, secret: &SecretValue, size_verdict: bool) -> SmtpOutcome {
    let failure = PreDataFailure {
        reason: FailureReason::Server,
        code: Some(reply.code),
        text: Some(sanitize(&reply.text(), secret)),
    };
    match reply.code / 100 {
        4 => SmtpOutcome::Transient(failure),
        5 if size_verdict && names_size_limit(&reply.text()) => {
            SmtpOutcome::Permanent(PreDataFailure {
                reason: FailureReason::MessageTooLarge,
                ..failure
            })
        }
        5 => SmtpOutcome::Permanent(failure),
        // 2xx where 3xx was needed and similar: a server we cannot talk to.
        _ => SmtpOutcome::Permanent(PreDataFailure {
            reason: FailureReason::Protocol,
            ..failure
        }),
    }
}

/// An unexpected reply at any other stage; the size vocabulary means nothing there.
fn unexpected(reply: &Reply, secret: &SecretValue) -> SmtpOutcome {
    refusal(reply, secret, false)
}

/// The last `EHLO`-advertised `SIZE` is recorded here so the account keeps it
/// for the composer's hint (sending.md section 16.4).
#[derive(Debug)]
pub(crate) struct Attempt {
    pub outcome: SmtpOutcome,
    /// `SIZE` the server advertised on the encrypted connection; `None` when
    /// the session ended before a usable `EHLO` or the server named no limit.
    pub advertised_size: Option<u64>,
}

struct Capabilities {
    starttls: bool,
    auth: Vec<String>,
    size: Option<u64>,
}

fn capabilities(reply: &Reply) -> Capabilities {
    let mut found = Capabilities {
        starttls: false,
        auth: Vec::new(),
        size: None,
    };
    // The first line is the server's greeting text, not a keyword.
    for line in reply.lines.iter().skip(1) {
        let mut words = line.split_ascii_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        match keyword.to_ascii_uppercase().as_str() {
            "STARTTLS" => found.starttls = true,
            "AUTH" => found
                .auth
                .extend(words.map(|word| word.to_ascii_uppercase())),
            // Pre-RFC 4954 spelling, `AUTH=PLAIN LOGIN`.
            keyword if keyword.starts_with("AUTH=") => {
                found.auth.push(keyword["AUTH=".len()..].to_owned());
                found
                    .auth
                    .extend(words.map(|word| word.to_ascii_uppercase()));
            }
            "SIZE" => found.size = words.next().and_then(|value| value.parse().ok()),
            _ => {}
        }
    }
    found
}

async fn ehlo(
    conn: &mut Conn,
    budget: Duration,
    secret: &SecretValue,
) -> Result<Capabilities, SmtpOutcome> {
    let reply = conn
        .command(&format!("EHLO {EHLO_NAME}"), budget)
        .await
        .map_err(io_outcome)?;
    if reply.code / 100 != 2 {
        return Err(unexpected(&reply, secret));
    }
    Ok(capabilities(&reply))
}

async fn authenticate(
    conn: &mut Conn,
    login: &SmtpLogin,
    caps: &Capabilities,
    budget: Duration,
) -> Result<(), SmtpOutcome> {
    let supports = |name: &str| caps.auth.iter().any(|mechanism| mechanism == name);
    let password = login.password.expose_secret();
    let done = |reply: Reply| -> Result<(), SmtpOutcome> {
        match reply.code {
            235 => Ok(()),
            code if (400..600).contains(&code) => {
                let failure = PreDataFailure {
                    reason: FailureReason::AuthRejected,
                    code: Some(code),
                    text: Some(sanitize(&reply.text(), &login.password)),
                };
                // 454 and friends are temporary; everything else is a verdict on the credentials.
                Err(if code / 100 == 4 {
                    SmtpOutcome::Transient(PreDataFailure {
                        reason: FailureReason::Server,
                        ..failure
                    })
                } else {
                    SmtpOutcome::Permanent(failure)
                })
            }
            _ => Err(unexpected(&reply, &login.password)),
        }
    };
    if supports("PLAIN") {
        let token = STANDARD.encode(format!("\0{}\0{password}", login.username));
        let reply = conn
            .command(&format!("AUTH PLAIN {token}"), budget)
            .await
            .map_err(io_outcome)?;
        done(reply)
    } else if supports("LOGIN") {
        let reply = conn
            .command("AUTH LOGIN", budget)
            .await
            .map_err(io_outcome)?;
        if reply.code != 334 {
            return done(reply);
        }
        let reply = conn
            .command(&STANDARD.encode(&login.username), budget)
            .await
            .map_err(io_outcome)?;
        if reply.code != 334 {
            return done(reply);
        }
        let reply = conn
            .command(&STANDARD.encode(password), budget)
            .await
            .map_err(io_outcome)?;
        done(reply)
    } else {
        Err(permanent(FailureReason::AuthUnavailable))
    }
}

/// A diagnostic authenticates and quits without constructing an envelope or
/// entering any delivery phase. The caller owns its total timeout/cancellation.
pub(crate) async fn verify_connection(
    route: &NetRoute,
    login: &SmtpLogin,
    timeouts: &Timeouts,
    tls: &TlsSource,
) -> Result<(), SmtpOutcome> {
    let (mut conn, _) = authenticated_connection(route, login, timeouts, tls).await?;
    conn.quit(timeouts.quit).await;
    Ok(())
}

async fn authenticated_connection(
    route: &NetRoute,
    login: &SmtpLogin,
    timeouts: &Timeouts,
    tls: &TlsSource,
) -> Result<(Conn, Capabilities), SmtpOutcome> {
    let secret = &login.password;

    let stream = net::connect(route, &login.host, login.port)
        .await
        .map_err(net_outcome)?;
    let stream = match login.tls {
        TlsMode::Implicit => tls
            .upgrade(stream, &login.host)
            .await
            .map_err(net_outcome)?,
        TlsMode::StartTls => stream,
    };
    let mut conn = Conn::new(stream);

    let greeting = conn
        .exchange(None, timeouts.greeting)
        .await
        .map_err(io_outcome)?;
    if greeting.code != 220 {
        return Err(unexpected(&greeting, secret));
    }
    let mut caps = ehlo(&mut conn, timeouts.command, secret).await?;

    if login.tls == TlsMode::StartTls {
        if !caps.starttls {
            conn.quit(timeouts.quit).await;
            return Err(permanent(FailureReason::TlsUnavailable));
        }
        let reply = conn
            .command("STARTTLS", timeouts.command)
            .await
            .map_err(io_outcome)?;
        if reply.code != 220 {
            return Err(unexpected(&reply, secret));
        }
        // Nothing may follow the 220 until our handshake: bytes already in the
        // read buffer were sent in the clear by whoever controls the stream
        // (STARTTLS injection, RFC 3207 section 6) and must not be trusted or
        // replayed after the upgrade. Refuse instead of discarding.
        if !conn.reader.buffer().is_empty() {
            return Err(permanent(FailureReason::Protocol));
        }
        let plain = conn.reader.into_inner();
        let secured = tls.upgrade(plain, &login.host).await.map_err(net_outcome)?;
        conn = Conn::new(secured);
        // Capabilities learned before TLS are discarded, not trusted.
        caps = ehlo(&mut conn, timeouts.command, secret).await?;
    }

    authenticate(&mut conn, login, &caps, timeouts.command).await?;

    Ok((conn, caps))
}

/// Dot-stuffs `message` for the DATA phase: every line break becomes CRLF
/// (bare LF and bare CR included), lines starting with `.` get another `.`,
/// and the data ends with `CRLF . CRLF`.
pub(crate) fn data_payload(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 16);
    let mut at_line_start = true;
    let mut index = 0;
    while index < message.len() {
        let byte = message[index];
        match byte {
            b'\r' | b'\n' => {
                if byte == b'\r' && message.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                out.extend_from_slice(b"\r\n");
                at_line_start = true;
            }
            _ => {
                if at_line_start && byte == b'.' {
                    out.push(b'.');
                }
                out.push(byte);
                at_line_start = false;
            }
        }
        index += 1;
    }
    if !at_line_start {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// Writes the body in chunks. Only a stall counts: `idle` runs without a single
/// byte accepted ends the attempt, and there is no cap on the total time, so a
/// slow link can still deliver a large message (sending.md section 16.6). The
/// body may be partly on the wire by then, so the caller records `unknown`.
async fn write_payload<W: AsyncWrite + Unpin>(
    stream: &mut W,
    payload: &[u8],
    idle: Duration,
) -> Result<(), UnknownReason> {
    const CHUNK: usize = 16 * 1024;
    for chunk in payload.chunks(CHUNK) {
        match timeout(idle, stream.write_all(chunk)).await {
            Err(_) => return Err(UnknownReason::Timeout),
            Ok(Err(_)) => return Err(UnknownReason::ConnectionLost),
            Ok(Ok(())) => {}
        }
    }
    match timeout(idle, stream.flush()).await {
        Err(_) => Err(UnknownReason::Timeout),
        Ok(Err(_)) => Err(UnknownReason::ConnectionLost),
        Ok(Ok(())) => Ok(()),
    }
}

/// `Ok` carries a finished outcome too; `Err` is only a convenience for `?`.
async fn session(
    route: &NetRoute,
    login: &SmtpLogin,
    envelope: &SmtpEnvelope,
    mime: &[u8],
    timeouts: &Timeouts,
    tls: &TlsSource,
    on_data_started: DataStarted,
) -> Result<Attempt, Attempt> {
    let bare = |outcome| Attempt {
        outcome,
        advertised_size: None,
    };
    if envelope.recipients.is_empty() || !mime.is_ascii() {
        return Err(bare(permanent(FailureReason::InvalidMessage)));
    }
    let secret = &login.password;
    let (mut conn, caps) = authenticated_connection(route, login, timeouts, tls)
        .await
        .map_err(bare)?;
    let advertised_size = caps.size;
    let with_size = |outcome| Attempt {
        outcome,
        advertised_size,
    };
    run_transaction(
        &mut conn,
        &caps,
        envelope,
        mime,
        timeouts,
        secret,
        on_data_started,
    )
    .await
    .map(with_size)
    .map_err(with_size)
}

async fn run_transaction(
    conn: &mut Conn,
    caps: &Capabilities,
    envelope: &SmtpEnvelope,
    mime: &[u8],
    timeouts: &Timeouts,
    secret: &SecretValue,
    on_data_started: DataStarted,
) -> Result<SmtpOutcome, SmtpOutcome> {
    let payload = data_payload(mime);
    // `SIZE 0` is the protocol's "no stated limit".
    if let Some(limit) = caps.size.filter(|limit| *limit > 0)
        && (mime.len() as u64) > limit
    {
        conn.quit(timeouts.quit).await;
        return Err(permanent(FailureReason::MessageTooLarge));
    }

    let mut mail = format!("MAIL FROM:<{}>", envelope.from);
    if caps.size.is_some_and(|limit| limit > 0) {
        mail.push_str(&format!(" SIZE={}", mime.len()));
    }
    let reply = conn
        .command(&mail, timeouts.command)
        .await
        .map_err(io_outcome)?;
    if reply.code / 100 != 2 {
        return Err(refusal(&reply, secret, true));
    }

    let mut rejected = Vec::new();
    for recipient in &envelope.recipients {
        let reply = conn
            .command(&format!("RCPT TO:<{recipient}>"), timeouts.command)
            .await
            .map_err(io_outcome)?;
        if reply.code / 100 != 2 {
            rejected.push(RejectedRecipient {
                address: recipient.clone(),
                code: reply.code,
                text: sanitize(&reply.text(), secret),
            });
        }
    }
    if !rejected.is_empty() {
        let _ = conn.command("RSET", timeouts.command).await;
        conn.quit(timeouts.quit).await;
        return Ok(SmtpOutcome::RecipientsRejected(rejected));
    }

    let reply = conn
        .command("DATA", timeouts.data_initiation)
        .await
        .map_err(io_outcome)?;
    if reply.code != 354 {
        return Err(refusal(&reply, secret, true));
    }

    // The server now waits for the body. Record the mark first; if that fails,
    // drop the connection (QUIT would be read as message text) without a byte sent.
    if (on_data_started.0)().await.is_err() {
        return Err(transient(FailureReason::DataMarkFailed));
    }

    if let Err(reason) = write_payload(conn.reader.get_mut(), &payload, timeouts.data_write).await {
        return Ok(SmtpOutcome::Unknown(reason));
    }
    let outcome = match timeout(timeouts.final_reply, read_reply(&mut conn.reader)).await {
        Err(_) => SmtpOutcome::Unknown(UnknownReason::Timeout),
        Ok(Err(IoFail::Timeout)) => SmtpOutcome::Unknown(UnknownReason::Timeout),
        Ok(Err(IoFail::Io | IoFail::Eof)) => SmtpOutcome::Unknown(UnknownReason::ConnectionLost),
        Ok(Err(IoFail::Malformed)) => SmtpOutcome::Unknown(UnknownReason::MalformedReply),
        Ok(Ok(reply)) => {
            let summary = ServerReply {
                code: reply.code,
                text: sanitize(&reply.text(), secret),
            };
            match reply.code / 100 {
                2 => SmtpOutcome::Accepted(summary),
                4 | 5 => SmtpOutcome::RejectedAfterData(summary),
                _ => SmtpOutcome::Unknown(UnknownReason::MalformedReply),
            }
        }
    };
    if matches!(
        outcome,
        SmtpOutcome::Accepted(_) | SmtpOutcome::RejectedAfterData(_)
    ) {
        conn.quit(timeouts.quit).await;
    }
    Ok(outcome)
}
