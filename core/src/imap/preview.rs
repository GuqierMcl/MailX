//! Batched partial fetch of message starts for list previews.
//!
//! Runs on the dedicated session from `open_body_session`, so routing, TLS,
//! authentication and the receive gate are exactly those of a body download.
//! `BODY.PEEK` leaves `\Seen` alone, and every literal is bounded before it is
//! read. The reply framing is shared with the full-body collector.
use std::time::Duration;

use async_imap::imap_proto::{AttributeValue, MessageSection, Response, SectionPath, Status};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::time::timeout;

use super::body::{
    Budget, Limits, MAX_CONTROL_BYTES, MAX_LINE_BYTES, READ_CHUNK, network, protocol,
    read_response, unescape_quoted,
};
use super::{ImapError, Session};

/// How much of each message body a preview asks for.
pub(crate) const PREVIEW_BYTES: u32 = 16 * 1024;
/// Most UIDs in one `UID FETCH`.
pub(crate) const PREVIEW_BATCH: usize = 50;
/// Most `TEXT` bytes accepted per message from a server that ignored the
/// partial range. Longer replies are truncated locally to [`PREVIEW_BYTES`];
/// beyond this the message is [`PartialFetch::Oversized`]. It also sizes the
/// response budget and decides which messages are fetched alone.
pub(crate) const PREVIEW_FALLBACK_BYTES: u64 = 1024 * 1024;
/// Most message bytes (each capped at the fallback) one batch may add up to,
/// so a server that ignores the partial range cannot make a batch transfer
/// tens of MiB in a single request.
pub(crate) const PREVIEW_BATCH_BYTES: u64 = 4 * 1024 * 1024;
const HEADER_LIMIT: usize = 64 * 1024;
/// Room for literals the size estimate does not cover.
const PAYLOAD_SLACK: usize = 4 * 1024;
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(60);
const TAG: &str = "MAILXPREV";

/// One message's start, as returned by the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PartialFetch {
    /// `HEADER.FIELDS` and `TEXT<0.n>` literals.
    Data { header: Vec<u8>, text: Vec<u8> },
    /// A literal exceeded [`PREVIEW_FALLBACK_BYTES`] (a server that ignored the
    /// partial range) or the header limit; its data was dropped. Shorter
    /// over-long text is truncated to [`PREVIEW_BYTES`] and stays `Data`.
    Oversized,
}

impl Session {
    /// Fetches the start of up to [`PREVIEW_BATCH`] messages of the mailbox
    /// already EXAMINEd (and UIDVALIDITY-checked) on this session. UIDs the
    /// server no longer has are absent from the result. `sizes` holds each
    /// message's stored RFC822.SIZE, parallel to `uids`; the response budget
    /// is derived from it. The session is consumed so a failed transfer
    /// cannot contaminate later work.
    pub(crate) async fn fetch_previews(
        mut self,
        uids: &[u32],
        sizes: &[Option<u64>],
    ) -> Result<Vec<(u32, PartialFetch)>, ImapError> {
        if uids.is_empty()
            || uids.len() > PREVIEW_BATCH
            || uids.len() != sizes.len()
            || uids.contains(&0)
        {
            return Err(protocol());
        }
        let guard = self.body_guard.clone().ok_or_else(protocol)?;
        let result = timeout(
            PREVIEW_TIMEOUT,
            fetch(self.inner.get_mut(), uids, limits(sizes)),
        )
        .await
        .map_err(|_| ImapError::Timeout)?;
        guard.check(result)
    }
}

/// The batch's response budget: per message, its text capped at the fallback
/// (an unknown size counts as the cap) plus the header allowance.
fn limits(sizes: &[Option<u64>]) -> Limits {
    let text: u64 = sizes
        .iter()
        .map(|size| {
            size.map_or(PREVIEW_FALLBACK_BYTES, |size| {
                size.min(PREVIEW_FALLBACK_BYTES)
            })
        })
        .sum();
    Limits {
        payload: (text as usize).saturating_add(sizes.len() * HEADER_LIMIT + PAYLOAD_SLACK),
        control: MAX_CONTROL_BYTES,
        line: MAX_LINE_BYTES,
    }
}

fn command(uids: &[u32]) -> String {
    let set = uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{TAG} UID FETCH {set} (UID BODY.PEEK[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] BODY.PEEK[TEXT]<0.{PREVIEW_BYTES}>)\r\n"
    )
}

async fn fetch<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    uids: &[u32],
    limits: Limits,
) -> Result<Vec<(u32, PartialFetch)>, ImapError> {
    stream
        .write_all(command(uids).as_bytes())
        .await
        .map_err(|_| network())?;
    stream.flush().await.map_err(|_| network())?;
    let mut reader = BufReader::with_capacity(READ_CHUNK, stream);
    let mut budget = Budget::default();
    let mut fetched: Vec<(u32, PartialFetch)> = Vec::new();
    loop {
        let raw = read_response(&mut reader, &mut budget, limits).await?;
        match classify(&raw, uids)? {
            Reply::Message(uid, part) => {
                // A repeated UID is a protocol error, not a later overwrite.
                if fetched.iter().any(|(seen, _)| *seen == uid) {
                    return Err(protocol());
                }
                fetched.push((uid, part));
            }
            Reply::Complete => return Ok(fetched),
            Reply::Unsolicited => {}
        }
    }
}

enum Reply {
    Message(u32, PartialFetch),
    Complete,
    Unsolicited,
}

fn classify(raw: &[u8], requested: &[u32]) -> Result<Reply, ImapError> {
    let (remaining, response) = Response::parse(raw).map_err(|_| protocol())?;
    if !remaining.is_empty() {
        return Err(protocol());
    }
    if super::policy::response(&response) {
        return Err(ImapError::ProviderPolicy);
    }
    match response {
        Response::Done { tag, status, .. } if tag.as_bytes() == TAG.as_bytes() => match status {
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
            let mut uid = None;
            let mut header = None;
            let mut text: Option<Vec<u8>> = None;
            for attribute in &attributes {
                match attribute {
                    AttributeValue::Uid(value) => {
                        if uid.replace(*value).is_some() || !requested.contains(value) {
                            return Err(protocol());
                        }
                    }
                    AttributeValue::BodySection {
                        section: Some(SectionPath::Full(MessageSection::Header)),
                        index: None,
                        data,
                    } if header.is_none() => header = Some(owned(raw, data.as_deref())),
                    AttributeValue::BodySection {
                        section: Some(SectionPath::Full(MessageSection::Text)),
                        index: Some(0),
                        data,
                    } if text.is_none() => text = Some(owned(raw, data.as_deref())),
                    AttributeValue::BodySection { .. }
                    | AttributeValue::Rfc822(_)
                    | AttributeValue::Rfc822Header(_)
                    | AttributeValue::Rfc822Text(_) => return Err(protocol()),
                    _ => {}
                }
            }
            let (Some(uid), Some(header), Some(mut text)) = (uid, header, text) else {
                return Err(protocol());
            };
            let part = if text.len() as u64 > PREVIEW_FALLBACK_BYTES || header.len() > HEADER_LIMIT
            {
                PartialFetch::Oversized
            } else {
                // A server that ignored the partial range sent all of it.
                text.truncate(PREVIEW_BYTES as usize);
                PartialFetch::Data { header, text }
            };
            Ok(Reply::Message(uid, part))
        }
        _ => Ok(Reply::Unsolicited),
    }
}

/// Copies a section out of the borrowed parse, undoing quoted-string escapes
/// (imap-proto leaves them in place). `NIL` and `""` both become empty.
fn owned(raw: &[u8], data: Option<&[u8]>) -> Vec<u8> {
    let Some(data) = data.filter(|data| !data.is_empty()) else {
        return Vec::new();
    };
    let start = (data.as_ptr() as usize).saturating_sub(raw.as_ptr() as usize);
    let mut bytes = data.to_vec();
    if start > 0 && raw.get(start - 1) == Some(&b'"') {
        unescape_quoted(&mut bytes);
    }
    bytes
}

#[cfg(test)]
mod tests;
