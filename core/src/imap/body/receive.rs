//! Do not let async-imap see an unvalidated literal declaration or read ahead
//! beyond a response boundary before the body collector takes over.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::{Budget, ImapError, Limits, READ_CHUNK, literal_length};
use crate::net::BoxedStream;

#[derive(Clone, Debug, Default)]
pub(in crate::imap) struct ReceiveGuard(Arc<AtomicU8>);

impl ReceiveGuard {
    pub(in crate::imap) fn wrap(&self, stream: BoxedStream) -> (BoxedStream, PlaintextGate) {
        let active = Arc::new(AtomicBool::new(true));
        let stream = ReceiveStream {
            stream,
            guard: self.clone(),
            active: active.clone(),
            stage: Stage::Line,
            line: Vec::new(),
            nesting: 0,
            budget: Budget::default(),
            limits: Limits::default(),
        };
        (Box::new(stream), PlaintextGate(active))
    }

    pub(in crate::imap) fn check<T>(&self, result: Result<T, ImapError>) -> Result<T, ImapError> {
        match self.0.load(Ordering::Acquire) {
            1 => Err(ImapError::TooLarge),
            2 => Err(super::protocol()),
            3 => Err(ImapError::ProviderPolicy),
            _ => result,
        }
    }
}

/// Disable the initial plaintext gate only after STARTTLS has completed, before
/// passing that stream through TLS. A new gate wraps the decrypted TLS stream.
pub(in crate::imap) struct PlaintextGate(Arc<AtomicBool>);

impl PlaintextGate {
    pub(in crate::imap) fn disable(self) {
        self.0.store(false, Ordering::Release);
    }
}

enum Stage {
    Line,
    EmitLine { offset: usize, literal: usize },
    Literal(usize),
}

struct ReceiveStream {
    stream: BoxedStream,
    guard: ReceiveGuard,
    active: Arc<AtomicBool>,
    stage: Stage,
    line: Vec<u8>,
    nesting: usize,
    budget: Budget,
    limits: Limits,
}

impl fmt::Debug for ReceiveStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundedImapStream")
    }
}

impl ReceiveStream {
    fn reject(&self, error: ImapError) -> io::Error {
        self.guard.0.store(
            match error {
                ImapError::TooLarge => 1,
                ImapError::ProviderPolicy => 3,
                _ => 2,
            },
            Ordering::Release,
        );
        io::Error::new(io::ErrorKind::InvalidData, "invalid IMAP frame")
    }
}

impl AsyncRead for ReceiveStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if !this.active.load(Ordering::Acquire) {
            return Pin::new(&mut this.stream).poll_read(context, output);
        }
        if this.guard.0.load(Ordering::Acquire) != 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IMAP receive limit reached",
            )));
        }
        loop {
            match &mut this.stage {
                Stage::Line => {
                    // No read-ahead: in particular, do not consume any bytes of
                    // the TLS handshake after the STARTTLS completion line.
                    let mut byte = [0];
                    let mut received = ReadBuf::new(&mut byte);
                    ready!(Pin::new(&mut this.stream).poll_read(context, &mut received))?;
                    if received.filled().is_empty() {
                        return Poll::Ready(Ok(()));
                    }
                    if this.line.len() == this.limits.line
                        || this.budget.control == this.limits.control
                    {
                        return Poll::Ready(Err(this.reject(super::protocol())));
                    }
                    this.budget.control += 1;
                    this.line.push(byte[0]);
                    if byte[0] == b'\n' {
                        if !this.line.ends_with(b"\r\n") {
                            return Poll::Ready(Err(this.reject(super::protocol())));
                        }
                        // Inspect only parsed status responses, never literal/header content.
                        if let Ok(([], response)) =
                            async_imap::imap_proto::Response::parse(&this.line)
                            && crate::imap::policy::response(&response)
                        {
                            return Poll::Ready(Err(this.reject(ImapError::ProviderPolicy)));
                        }
                        let literal = match literal_length(&this.line, &mut this.nesting) {
                            Ok(literal) => literal,
                            Err(error) => return Poll::Ready(Err(this.reject(error))),
                        };
                        if literal.is_none() {
                            this.nesting = 0;
                        }
                        let literal = literal.unwrap_or(0);
                        this.budget.declared = match this
                            .budget
                            .declared
                            .checked_add(literal)
                            .filter(|size| *size <= this.limits.payload)
                        {
                            Some(size) => size,
                            None => return Poll::Ready(Err(this.reject(ImapError::TooLarge))),
                        };
                        this.stage = Stage::EmitLine { offset: 0, literal };
                    }
                }
                Stage::EmitLine { offset, literal } => {
                    let copied = output.remaining().min(this.line.len() - *offset);
                    output.put_slice(&this.line[*offset..*offset + copied]);
                    *offset += copied;
                    if *offset == this.line.len() {
                        this.stage = if *literal == 0 {
                            Stage::Line
                        } else {
                            Stage::Literal(*literal)
                        };
                        this.line.clear();
                    }
                    return Poll::Ready(Ok(()));
                }
                Stage::Literal(remaining) => {
                    let mut chunk = [0; READ_CHUNK];
                    let requested = (*remaining).min(output.remaining()).min(chunk.len());
                    let mut received = ReadBuf::new(&mut chunk[..requested]);
                    ready!(Pin::new(&mut this.stream).poll_read(context, &mut received))?;
                    let received = received.filled();
                    this.budget.received = match this
                        .budget
                        .received
                        .checked_add(received.len())
                        .filter(|size| *size <= this.limits.payload)
                    {
                        Some(size) => size,
                        None => return Poll::Ready(Err(this.reject(ImapError::TooLarge))),
                    };
                    output.put_slice(received);
                    *remaining -= received.len();
                    if *remaining == 0 {
                        this.stage = Stage::Line;
                    }
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl AsyncWrite for ReceiveStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(context, bytes)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn gate_never_exposes_an_overlimit_literal_header() {
        let (client, mut server) = tokio::io::duplex(1024);
        server
            .write_all(b"* 7 FETCH (UID 42 BODY[] {536870912}\r\n")
            .await
            .unwrap();
        let guard = ReceiveGuard::default();
        let (mut client, _) = guard.wrap(Box::new(client));
        let mut bytes = [0; 512];
        assert!(client.read(&mut bytes).await.is_err());
        assert!(bytes.iter().all(|byte| *byte == 0));
        assert_eq!(guard.check(Ok(())), Err(ImapError::TooLarge));
    }

    #[tokio::test]
    async fn unsolicited_literal_outside_parentheses_cannot_bypass_the_gate() {
        let (client, mut server) = tokio::io::duplex(1024);
        server
            .write_all(b"* LIST () \"/\" {536870912}\r\n")
            .await
            .unwrap();
        let guard = ReceiveGuard::default();
        let (mut client, _) = guard.wrap(Box::new(client));
        let mut bytes = [0; 512];
        assert!(client.read(&mut bytes).await.is_err());
        assert!(bytes.iter().all(|byte| *byte == 0));
        assert_eq!(guard.check(Ok(())), Err(ImapError::TooLarge));
    }

    #[tokio::test]
    async fn gate_preserves_boundaries_and_does_not_scan_literal_contents() {
        let (client, mut server) = tokio::io::duplex(1024);
        let payload = b"{536870912}\r\n\0";
        let prefix = format!("* 1 FETCH (BODY[] {{{}}}\r\n", payload.len());
        let reply = [
            prefix.as_bytes(),
            payload,
            b" UID 42)\r\nA1 OK\r\n* 1 EXISTS\r\n",
        ]
        .concat();
        server.write_all(&reply).await.unwrap();
        let guard = ReceiveGuard::default();
        let (mut client, _) = guard.wrap(Box::new(client));
        let mut bytes = [0; 1024];
        for expected in [
            prefix.as_bytes(),
            payload,
            b" UID 42)\r\n",
            b"A1 OK\r\n",
            b"* 1 EXISTS\r\n",
        ] {
            let received = client.read(&mut bytes).await.unwrap();
            assert_eq!(&bytes[..received], expected);
        }
        assert_eq!(guard.check(Ok(())), Ok(()));
    }

    #[tokio::test]
    async fn starttls_handoff_does_not_consume_or_interpret_tls_bytes() {
        let (client, mut server) = tokio::io::duplex(1024);
        let tls_bytes = b"\x16\x03\x03\x00\x05\xff\x00\r\n{";
        server
            .write_all(&[b"A1 OK begin TLS\r\n".as_slice(), tls_bytes].concat())
            .await
            .unwrap();
        let guard = ReceiveGuard::default();
        let (mut client, gate) = guard.wrap(Box::new(client));
        let mut bytes = [0; 1024];
        let received = client.read(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..received], b"A1 OK begin TLS\r\n");
        gate.disable();
        let received = client.read(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..received], tls_bytes);
    }
}
