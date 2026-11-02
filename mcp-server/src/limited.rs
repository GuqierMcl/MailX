//! Message size guard in front of the MCP transport.
//!
//! rmcp's line limit silently drops an over-long message and keeps the
//! session open, so a client waiting for a reply would hang. The request's id
//! is not known without parsing it (which is exactly what the limit protects
//! against), so a JSON-RPC error cannot be produced. This reader instead ends
//! the session as soon as a line exceeds the limit, which the client sees as a
//! closed connection.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

/// Largest accepted MCP message (one JSON line), in bytes.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

pub(crate) struct LimitedRead<R> {
    inner: R,
    limit: usize,
    line_len: usize,
}

impl<R> LimitedRead<R> {
    pub(crate) fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            line_len: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LimitedRead<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let this = &mut *self;
                let mut exceeded = false;
                for &byte in &buf.filled()[before..] {
                    if byte == b'\n' {
                        this.line_len = 0;
                    } else {
                        this.line_len += 1;
                        exceeded |= this.line_len > this.limit;
                    }
                }
                if exceeded {
                    // Do not hand the offending bytes to the parser.
                    buf.set_filled(before);
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP message exceeds the size limit",
                    )));
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::*;

    #[tokio::test]
    async fn passes_lines_within_the_limit_and_resets_per_line() {
        let (mut tx, rx) = duplex(1024);
        tx.write_all(b"12345\n12345\n").await.unwrap();
        drop(tx);
        let mut reader = LimitedRead::new(rx, 5);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"12345\n12345\n");
    }

    #[tokio::test]
    async fn errors_when_a_line_exceeds_the_limit() {
        let (mut tx, rx) = duplex(1024);
        tx.write_all(b"123456").await.unwrap();
        drop(tx);
        let mut reader = LimitedRead::new(rx, 5);
        let mut out = Vec::new();
        let error = reader.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
