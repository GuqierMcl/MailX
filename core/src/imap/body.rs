//! Bounded reception of one complete message on a disposable IMAP connection.
//!
//! async-imap expands its response buffer from an untrusted literal declaration.
//! This path frames responses itself before passing a complete, bounded response
//! to the existing IMAP parser. It never feeds body responses to that buffer.

use std::ops::Range;
use std::time::Duration;

use async_imap::imap_proto::{AttributeValue, Response, Status};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::time::timeout;

use super::{COMMAND_TIMEOUT, ImapError, ImapLogin, Session};
#[cfg(test)]
use crate::accounts::types::TlsMode;
use crate::mail::render::MAX_MESSAGE_BYTES;
#[cfg(test)]
use crate::net::BoxedStream;
use crate::net::NetRoute;

mod receive;
pub(super) use receive::{PlaintextGate, ReceiveGuard};
const MAX_CONTROL_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_NESTING: usize = 32;
const READ_CHUNK: usize = 8 * 1024;
const MAX_BODY_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The dedicated body connection uses the same route, TLS, greeting and login
/// implementation as synchronization, with a receive gate on every plaintext
/// phase. It does not alter the long-lived synchronization connection.
pub(crate) async fn open_body_session(
    route: &NetRoute,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    let guard = ReceiveGuard::default();
    let result = async {
        // Unlike header sync, the body connection keeps its receive gate for
        // EXAMINE and MIME transfer as well as authentication.
        let (client, _gate) = super::connect_client(route, login, &guard).await?;
        let mut session = super::authenticate(client, login).await?;
        session.body_guard = Some(guard.clone());
        Ok(session)
    }
    .await;
    guard.check(result)
}

#[cfg(test)]
pub(crate) async fn test_body_session(
    stream: BoxedStream,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    let guard = ReceiveGuard::default();
    let (stream, _) = guard.wrap(stream);
    let result = super::test_session(stream, login).await.map(|mut session| {
        session.body_guard = Some(guard.clone());
        session
    });
    guard.check(result)
}

impl Session {
    /// Fetches a complete message without changing its read flag. The caller
    /// must first EXAMINE and validate UIDVALIDITY on this independent session.
    ///
    /// `open_body_session` places a receiving gate beneath the library parser.
    /// It validates literal sizes before exposing declarations and never reads
    /// beyond a response boundary. Thus EXAMINE leaves no hidden buffered tail
    /// when this collector takes over. Consuming the session ensures that an
    /// interrupted or rejected body transfer can never contaminate later work.
    pub(crate) async fn fetch_message(
        mut self,
        uid: u32,
        size: Option<u64>,
    ) -> Result<Option<Vec<u8>>, ImapError> {
        if uid == 0 {
            return Err(protocol());
        }
        if size.is_some_and(|value| value > MAX_MESSAGE_BYTES as u64) {
            return Err(ImapError::TooLarge);
        }
        let guard = self.body_guard.clone().ok_or_else(protocol)?;
        // Some providers truncate long tags. This is the final command on a
        // consumed, dedicated session, so a short tag distinct from async-imap's
        // tags is sufficient; completion matching remains exact.
        let tag = "MAILXBODY";
        let result = timeout(
            body_timeout(size),
            fetch(self.inner.get_mut(), tag, uid, Limits::default()),
        )
        .await
        .map_err(|_| ImapError::Timeout)?;
        guard.check(result)
    }
}

fn body_timeout(size: Option<u64>) -> Duration {
    size.map(|size| Duration::from_secs(size.div_ceil(64 * 1024)))
        .unwrap_or(MAX_BODY_TIMEOUT)
        .clamp(COMMAND_TIMEOUT, MAX_BODY_TIMEOUT)
}

#[derive(Clone, Copy)]
struct Limits {
    payload: usize,
    control: usize,
    line: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            payload: MAX_MESSAGE_BYTES,
            control: MAX_CONTROL_BYTES,
            line: MAX_LINE_BYTES,
        }
    }
}

#[derive(Default)]
struct Budget {
    declared: usize,
    received: usize,
    control: usize,
}

async fn fetch<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    tag: &str,
    uid: u32,
    limits: Limits,
) -> Result<Option<Vec<u8>>, ImapError> {
    let command = format!("{tag} UID FETCH {uid} (UID BODY.PEEK[])\r\n");
    stream
        .write_all(command.as_bytes())
        .await
        .map_err(|_| network())?;
    stream.flush().await.map_err(|_| network())?;
    let mut reader = BufReader::with_capacity(READ_CHUNK, stream);
    let mut budget = Budget::default();
    let mut body = None;
    loop {
        let mut raw = read_response(&mut reader, &mut budget, limits).await?;
        match classify_response(&raw, tag, uid)? {
            Reply::Body { range, quoted } => {
                if body.is_some() {
                    return Err(protocol());
                }
                if range.len() > limits.payload {
                    return Err(ImapError::TooLarge);
                }
                // The parser borrows literals from raw. Reuse that allocation
                // rather than retaining both the full response and a body copy.
                let length = range.len();
                raw.copy_within(range, 0);
                raw.truncate(length);
                if quoted {
                    unescape_quoted(&mut raw);
                }
                body = Some(raw);
            }
            Reply::Complete => return Ok(body),
            Reply::Unsolicited => {}
        }
    }
}

async fn read_response<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    budget: &mut Budget,
    limits: Limits,
) -> Result<Vec<u8>, ImapError> {
    let mut raw = Vec::new();
    let mut nesting = 0;
    loop {
        let line = read_line(reader, budget, limits).await?;
        let literal = literal_length(&line, &mut nesting)?;
        append(&mut raw, &line, limits)?;
        let Some(length) = literal else {
            return Ok(raw);
        };
        // Validate the declaration before reserving or reading its payload.
        budget.declared = budget
            .declared
            .checked_add(length)
            .filter(|value| *value <= limits.payload)
            .ok_or(ImapError::TooLarge)?;
        let mut remaining = length;
        let mut chunk = [0; READ_CHUNK];
        while remaining > 0 {
            let requested = remaining.min(chunk.len());
            let received = reader
                .read(&mut chunk[..requested])
                .await
                .map_err(|_| network())?;
            if received == 0 {
                return Err(protocol());
            }
            budget.received = budget
                .received
                .checked_add(received)
                .filter(|value| *value <= limits.payload)
                .ok_or(ImapError::TooLarge)?;
            append(&mut raw, &chunk[..received], limits)?;
            remaining -= received;
        }
    }
}

async fn read_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    budget: &mut Budget,
    limits: Limits,
) -> Result<Vec<u8>, ImapError> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|_| network())?;
        if available.is_empty() {
            return Err(protocol());
        }
        let length = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |offset| offset + 1);
        let complete = available[length - 1] == b'\n';
        if line.len().saturating_add(length) > limits.line {
            return Err(protocol());
        }
        budget.control = budget
            .control
            .checked_add(length)
            .filter(|value| *value <= limits.control)
            .ok_or_else(protocol)?;
        line.extend_from_slice(&available[..length]);
        reader.consume(length);
        if complete {
            if !line.ends_with(b"\r\n") {
                return Err(protocol());
            }
            return Ok(line);
        }
    }
}

/// Bound parser nesting and identify a literal only outside quoted strings.
/// Literal bytes themselves never pass through this syntax scan.
fn literal_length(line: &[u8], nesting: &mut usize) -> Result<Option<usize>, ImapError> {
    let mut quoted = false;
    let mut escaped = false;
    let mut marker = None;
    for (index, byte) in line[..line.len() - 2].iter().copied().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'(' => {
                    *nesting += 1;
                    if *nesting > MAX_NESTING {
                        return Err(protocol());
                    }
                }
                b')' => *nesting = nesting.saturating_sub(1),
                b'{' => marker = Some(index),
                _ => {}
            }
        }
    }
    if quoted {
        return Err(protocol());
    }
    let Some(marker) = marker.filter(|_| line.ends_with(b"}\r\n")) else {
        return Ok(None);
    };
    // Status response text can contain braces without declaring a literal.
    // Other responses (for example LIST mailbox names) may have a literal
    // outside parentheses, so nesting alone must never exempt a declaration.
    if *nesting == 0
        && matches!(
            Response::parse(line),
            Ok((
                [],
                Response::Done { .. } | Response::Data { .. } | Response::Continue(_)
            ))
        )
    {
        return Ok(None);
    }
    let digits = &line[marker + 1..line.len() - 3];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(protocol());
    }
    digits
        .iter()
        .try_fold(0usize, |number, digit| {
            number
                .checked_mul(10)
                .and_then(|number| number.checked_add(usize::from(*digit - b'0')))
                .ok_or(ImapError::TooLarge)
        })
        .map(Some)
}

fn append(raw: &mut Vec<u8>, bytes: &[u8], limits: Limits) -> Result<(), ImapError> {
    let maximum = limits.payload.saturating_add(limits.control);
    let needed = raw
        .len()
        .checked_add(bytes.len())
        .filter(|needed| *needed <= maximum)
        .ok_or(ImapError::TooLarge)?;
    if needed > raw.capacity() {
        let capacity = needed.max(raw.capacity().saturating_mul(2)).min(maximum);
        raw.try_reserve_exact(capacity - raw.len())
            .map_err(|_| protocol())?;
    }
    raw.extend_from_slice(bytes);
    Ok(())
}

enum Reply {
    Body { range: Range<usize>, quoted: bool },
    Complete,
    Unsolicited,
}

fn classify_response(raw: &[u8], tag: &str, uid: u32) -> Result<Reply, ImapError> {
    let (remaining, response) = Response::parse(raw).map_err(|_| protocol())?;
    if !remaining.is_empty() {
        return Err(protocol());
    }
    if super::policy::response(&response) {
        return Err(ImapError::ProviderPolicy);
    }
    match response {
        Response::Done {
            tag: response_tag,
            status,
            ..
        } if response_tag.as_bytes() == tag.as_bytes() => match status {
            Status::Ok => Ok(Reply::Complete),
            Status::No | Status::Bad => Err(ImapError::Rejected),
            _ => Err(protocol()),
        },
        Response::Done { .. }
        | Response::Continue(_)
        | Response::Data {
            status: Status::Bye,
            ..
        } => Err(protocol()),
        Response::Fetch(_, attributes) => {
            let mut found_uid = None;
            let mut body = None;
            for attribute in &attributes {
                match attribute {
                    AttributeValue::Uid(value) => {
                        if found_uid.replace(*value).is_some() || *value != uid {
                            return Err(protocol());
                        }
                    }
                    AttributeValue::BodySection {
                        section: None,
                        index: None,
                        data: Some(value),
                    } if body.is_none() => body = Some(value.as_ref()),
                    AttributeValue::BodySection { .. }
                    | AttributeValue::Rfc822(_)
                    | AttributeValue::Rfc822Header(_)
                    | AttributeValue::Rfc822Text(_) => return Err(protocol()),
                    _ => {}
                }
            }
            let body = body
                .filter(|_| found_uid == Some(uid))
                .ok_or_else(protocol)?;
            if body.is_empty() {
                // The dependency uses a static empty slice for BODY[] "".
                return Ok(Reply::Body {
                    range: 0..0,
                    quoted: false,
                });
            }
            let start = (body.as_ptr() as usize)
                .checked_sub(raw.as_ptr() as usize)
                .ok_or_else(protocol)?;
            let end = start.checked_add(body.len()).ok_or_else(protocol)?;
            if end > raw.len() {
                return Err(protocol());
            }
            Ok(Reply::Body {
                range: start..end,
                quoted: start > 0 && raw[start - 1] == b'"',
            })
        }
        _ => Ok(Reply::Unsolicited),
    }
}

fn unescape_quoted(bytes: &mut Vec<u8>) {
    // imap-proto preserves quoted-string backslashes in its borrowed slice.
    // Its parser has already validated that each escape is a quote/backslash.
    let mut read = 0;
    let mut write = 0;
    while read < bytes.len() {
        if bytes[read] == b'\\' {
            read += 1;
        }
        bytes[write] = bytes[read];
        write += 1;
        read += 1;
    }
    bytes.truncate(write);
}

fn protocol() -> ImapError {
    ImapError::Protocol("invalid or incomplete message response".into())
}

fn network() -> ImapError {
    ImapError::Net(crate::net::NetError::Connect(
        "message transfer failed".into(),
    ))
}

#[cfg(test)]
mod tests;
