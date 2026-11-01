//! Scripted IMAP server for IDLE tests, where the server must interleave
//! pushes with client lines instead of answering one prefix-matched command at
//! a time.

use std::future::Future;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::task::JoinHandle;

use super::{ImapLogin, Session, test_session};
use crate::accounts::types::TlsMode;
use crate::secrets::SecretValue;

/// The server end of an authenticated in-memory connection.
pub(crate) struct TestPeer(BufReader<DuplexStream>);

impl TestPeer {
    /// Reads one client line; returns `(tag, command)`. An untagged line such
    /// as `DONE` has an empty tag.
    pub(crate) async fn line(&mut self) -> Option<(String, String)> {
        let mut line = String::new();
        if self.0.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let line = line.trim_end().to_owned();
        if line == "DONE" {
            return Some((String::new(), line));
        }
        let (tag, command) = line.split_once(' ')?;
        Some((tag.to_owned(), command.to_owned()))
    }

    /// Reads a client line that must start with `prefix` and returns its tag.
    pub(crate) async fn expect(&mut self, prefix: &str) -> String {
        let (tag, command) = self.line().await.expect("client closed the connection");
        assert!(
            command.starts_with(prefix),
            "expected {prefix:?}, got {command:?}"
        );
        tag
    }

    /// Sends raw server text; the caller supplies the CRLFs.
    pub(crate) async fn send(&mut self, text: &str) {
        let _ = self.0.get_mut().write_all(text.as_bytes()).await;
    }

    /// Answers `EXAMINE` with a minimal valid mailbox.
    pub(crate) async fn examine(&mut self) {
        let tag = self.expect("EXAMINE").await;
        self.send(&format!(
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 7] v\r\n* OK [UIDNEXT 2] n\r\n{tag} OK [READ-ONLY] done\r\n"
        ))
        .await;
    }

    /// Reads `LOGOUT` and answers BYE + completion.
    pub(crate) async fn logout(&mut self) {
        let tag = self.expect("LOGOUT").await;
        self.send(&format!("* BYE bye\r\n{tag} OK done\r\n")).await;
    }

    /// Performs the server half of IDLE entry: reads `IDLE`,
    /// answers `+ idling`, and returns the IDLE tag.
    pub(crate) async fn enter_idle(&mut self) -> String {
        let tag = self.expect("IDLE").await;
        self.send("+ idling\r\n").await;
        tag
    }

    /// Reads `DONE` and completes the given IDLE tag.
    pub(crate) async fn leave_idle(&mut self, tag: &str) {
        self.expect("DONE").await;
        self.send(&format!("{tag} OK IDLE terminated\r\n")).await;
    }
}

/// Logs in against a peer that then hands the connection to `script`. The
/// returned task ends when `script` does, dropping (closing) the stream.
pub(crate) async fn scripted_session<F, Fut>(
    capabilities: &str,
    script: F,
) -> (Session, JoinHandle<()>)
where
    F: FnOnce(TestPeer) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (client, server) = tokio::io::duplex(64 * 1024);
    let capabilities = capabilities.to_owned();
    let task = tokio::spawn(async move {
        let mut peer = TestPeer(BufReader::new(server));
        peer.send("* OK IMAP4rev1 ready\r\n").await;
        let tag = peer.expect("LOGIN").await;
        peer.send(&format!("{tag} OK\r\n")).await;
        let tag = peer.expect("CAPABILITY").await;
        peer.send(&format!("* CAPABILITY {capabilities}\r\n{tag} OK\r\n"))
            .await;
        script(peer).await;
    });
    let login = ImapLogin {
        host: "imap.example.com".into(),
        port: 993,
        tls: TlsMode::Implicit,
        username: "lin@example.com".into(),
        password: SecretValue::new("secret".into()),
    };
    let session = test_session(Box::new(client), &login).await.unwrap();
    (session, task)
}
