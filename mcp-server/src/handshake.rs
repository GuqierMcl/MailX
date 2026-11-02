//! Token handshake that runs before any MCP bytes are exchanged.
//!
//! The bridge sends one JSON line `{"bridge":1,"token":"…"}`; the server
//! answers `{"ok":true}` or `{"ok":false}` and, on failure, closes. The line is
//! read one byte at a time so nothing past the newline is consumed: the next
//! bytes belong to the MCP transport.

use std::io;
use std::time::Duration;

use serde::Deserialize;
use subtle::ConstantTimeEq as _;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Longest accepted handshake line, including the newline.
pub const MAX_HANDSHAKE_BYTES: usize = 512;
/// Whole handshake must finish within this time.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Version of the handshake envelope.
pub const BRIDGE_PROTOCOL: u32 = 1;

#[derive(Deserialize)]
struct Hello {
    bridge: u32,
    token: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HandshakeError {
    Timeout,
    TooLong,
    Malformed,
    BadToken,
    /// Token was right but the server is at its connection limit.
    Busy,
    Io,
}

/// Reads and checks the hello line. With a valid token `admit` decides whether
/// there is room for another session; its result is returned to the caller on
/// success. Success replies `{"ok":true}`; every failure, including a full
/// server, replies `{"ok":false}` (best effort) and returns the reason.
pub(crate) async fn server_handshake<S, G>(
    stream: &mut S,
    token: &str,
    admit: impl FnOnce() -> Option<G>,
) -> Result<G, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let outcome = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_hello(stream, token))
        .await
        .unwrap_or(Err(HandshakeError::Timeout))
        .and_then(|()| admit().ok_or(HandshakeError::Busy));
    match outcome {
        Ok(admitted) => stream
            .write_all(b"{\"ok\":true}\n")
            .await
            .map(|()| admitted)
            .map_err(|_| HandshakeError::Io),
        Err(error) => {
            let _ = tokio::time::timeout(Duration::from_secs(1), reject(stream)).await;
            Err(error)
        }
    }
}

pub(crate) async fn reject<S: AsyncWrite + Unpin>(stream: &mut S) -> io::Result<()> {
    stream.write_all(b"{\"ok\":false}\n").await?;
    stream.shutdown().await
}

async fn read_hello<S>(stream: &mut S, token: &str) -> Result<(), HandshakeError>
where
    S: AsyncRead + Unpin,
{
    let mut line = Vec::with_capacity(128);
    loop {
        let byte = stream.read_u8().await.map_err(|_| HandshakeError::Io)?;
        if byte == b'\n' {
            break;
        }
        line.push(byte);
        if line.len() >= MAX_HANDSHAKE_BYTES {
            return Err(HandshakeError::TooLong);
        }
    }
    let hello: Hello = serde_json::from_slice(&line).map_err(|_| HandshakeError::Malformed)?;
    let token_matches = bool::from(hello.token.as_bytes().ct_eq(token.as_bytes()));
    if hello.bridge == BRIDGE_PROTOCOL && token_matches {
        Ok(())
    } else {
        Err(HandshakeError::BadToken)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::*;

    async fn run(input: &[u8]) -> (Result<(), HandshakeError>, Vec<u8>) {
        let (mut client, mut server) = duplex(4096);
        client.write_all(input).await.unwrap();
        let result = server_handshake(&mut server, "secret", || Some(())).await;
        drop(server);
        let mut reply = Vec::new();
        let _ = client.read_to_end(&mut reply).await;
        (result, reply)
    }

    #[tokio::test]
    async fn accepts_matching_token() {
        let (result, reply) = run(b"{\"bridge\":1,\"token\":\"secret\"}\n").await;
        assert_eq!(result, Ok(()));
        assert_eq!(reply, b"{\"ok\":true}\n");
    }

    #[tokio::test]
    async fn rejects_wrong_token_version_and_garbage() {
        for input in [
            &b"{\"bridge\":1,\"token\":\"nope\"}\n"[..],
            b"{\"bridge\":2,\"token\":\"secret\"}\n",
            b"not json\n",
            b"{\"bridge\":1}\n",
        ] {
            let (result, reply) = run(input).await;
            assert!(result.is_err());
            assert_eq!(reply, b"{\"ok\":false}\n");
        }
    }

    #[tokio::test]
    async fn full_server_rejects_even_with_a_valid_token() {
        let (mut client, mut server) = duplex(4096);
        client
            .write_all(b"{\"bridge\":1,\"token\":\"secret\"}\n")
            .await
            .unwrap();
        let result = server_handshake(&mut server, "secret", || None::<()>).await;
        assert_eq!(result, Err(HandshakeError::Busy));
        drop(server);
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"{\"ok\":false}\n");
    }

    #[tokio::test]
    async fn rejects_oversized_line() {
        let mut input = vec![b'a'; MAX_HANDSHAKE_BYTES + 10];
        input.push(b'\n');
        let (result, reply) = run(&input).await;
        assert_eq!(result, Err(HandshakeError::TooLong));
        assert_eq!(reply, b"{\"ok\":false}\n");
    }

    #[tokio::test]
    async fn does_not_consume_bytes_after_the_newline() {
        let (mut client, mut server) = duplex(4096);
        client
            .write_all(b"{\"bridge\":1,\"token\":\"secret\"}\n{\"jsonrpc\"")
            .await
            .unwrap();
        server_handshake(&mut server, "secret", || Some(()))
            .await
            .unwrap();
        let mut rest = [0_u8; 10];
        server.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"{\"jsonrpc\"");
    }

    #[tokio::test(start_paused = true)]
    async fn silent_peer_times_out() {
        let (_client, mut server) = duplex(64);
        let result = server_handshake(&mut server, "secret", || Some(())).await;
        assert_eq!(result, Err(HandshakeError::Timeout));
    }
}
