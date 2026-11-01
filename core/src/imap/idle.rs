//! IMAP IDLE (RFC 2177) on top of our `Session`.
//!
//! We drive IDLE with raw commands instead of `async_imap::extensions::idle`:
//! the library's timeout restarts on every `* OK Still here` keepalive, so it
//! is not a wall-clock deadline, and a closed stream is reported exactly like a
//! manual interrupt. Here the caller owns the deadline and the stop signal, and
//! a disconnect is always reported as such.

use std::future::Future;

use async_imap::imap_proto::{MailboxDatum, Response, Status};
use tokio::time::Instant;

use super::{ImapError, Session, policy, read_failure, step};

/// Why [`IdleSession::wait`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdleWake {
    /// The server announced EXISTS / EXPUNGE / FETCH / RECENT / VANISHED.
    Changed,
    /// The caller's deadline passed; IDLE is still running on the server.
    Deadline,
    /// The caller's stop signal fired; IDLE is still running on the server.
    Stopped,
    /// The server said BYE or the stream ended.
    Disconnected,
}

/// A session that is inside an IDLE command. Leave it with [`Self::finish`].
pub(crate) struct IdleSession {
    session: Session,
    tag: async_imap::imap_proto::RequestId,
    /// A change that arrived before the server's `+ idling` continuation.
    pending: bool,
}

/// Mailbox-change notifications; keepalives, `* OK`, capability pushes and the
/// like carry no news for the sync worker.
fn is_change(response: &Response<'_>) -> bool {
    matches!(
        response,
        Response::MailboxData(MailboxDatum::Exists(_) | MailboxDatum::Recent(_))
            | Response::Expunge(_)
            | Response::Fetch(..)
            | Response::Vanished { .. }
    )
}

fn is_bye(response: &Response<'_>) -> bool {
    matches!(
        response,
        Response::Data {
            status: Status::Bye,
            ..
        }
    )
}

impl Session {
    /// Sends `IDLE` and waits for the server's continuation. The mailbox must
    /// already be selected. Consumes the session: on error the connection is
    /// not worth keeping.
    pub(crate) async fn start_idle(mut self) -> Result<IdleSession, ImapError> {
        let (tag, pending) = step(async {
            let tag = self.inner.run_command("IDLE").await?;
            let mut pending = false;
            loop {
                let response = self
                    .inner
                    .read_response()
                    .await
                    .map_err(|error| read_failure("IDLE", &error))?
                    .ok_or_else(|| {
                        ImapError::Protocol("IMAP connection closed before completion".into())
                    })?;
                let parsed = response.parsed();
                if policy::response(parsed) {
                    return Err(ImapError::ProviderPolicy);
                }
                match parsed {
                    Response::Continue(_) => return Ok::<_, ImapError>((tag, pending)),
                    Response::Done {
                        tag: done_tag,
                        status: Status::No | Status::Bad,
                        ..
                    } if done_tag == &tag => return Err(ImapError::Rejected),
                    response if is_bye(response) => {
                        return Err(ImapError::Protocol(
                            "IMAP connection closed before completion".into(),
                        ));
                    }
                    response => pending |= is_change(response),
                }
            }
        })
        .await?;
        Ok(IdleSession {
            session: self,
            tag,
            pending,
        })
    }
}

impl IdleSession {
    /// Waits for news, the wall-clock `deadline`, or `stop`, whichever is first.
    /// Dropping the future mid-way is safe: the framed reader keeps partial
    /// responses.
    pub(crate) async fn wait(
        &mut self,
        deadline: Instant,
        stop: impl Future<Output = ()>,
    ) -> Result<IdleWake, ImapError> {
        if std::mem::take(&mut self.pending) {
            return Ok(IdleWake::Changed);
        }
        tokio::pin!(stop);
        loop {
            let read = tokio::select! {
                biased;
                () = &mut stop => return Ok(IdleWake::Stopped),
                () = tokio::time::sleep_until(deadline) => return Ok(IdleWake::Deadline),
                read = self.session.inner.read_response() => read,
            };
            let response = match read {
                Ok(Some(response)) => response,
                Ok(None) => return Ok(IdleWake::Disconnected),
                Err(error) => return Err(read_failure("IDLE", &error)),
            };
            let parsed = response.parsed();
            if policy::response(parsed) {
                return Err(ImapError::ProviderPolicy);
            }
            if is_bye(parsed) {
                return Ok(IdleWake::Disconnected);
            }
            match parsed {
                Response::Done { .. } => {
                    // The server ended IDLE on its own; it is not idling any more.
                    return Ok(IdleWake::Disconnected);
                }
                response if is_change(response) => return Ok(IdleWake::Changed),
                _ => {}
            }
        }
    }

    /// Sends `DONE` and consumes the tagged completion, returning the session
    /// to command mode. Also reports whether a mailbox change was announced
    /// while leaving, which the caller must not lose. Bounded by the command
    /// timeout.
    pub(crate) async fn finish(mut self) -> Result<(Session, bool), ImapError> {
        let changed = step(async {
            let mut changed = std::mem::take(&mut self.pending);
            self.session.inner.run_command_untagged("DONE").await?;
            loop {
                let response = self
                    .session
                    .inner
                    .read_response()
                    .await
                    .map_err(|error| read_failure("DONE", &error))?
                    .ok_or_else(|| {
                        ImapError::Protocol("IMAP connection closed before completion".into())
                    })?;
                let parsed = response.parsed();
                if policy::response(parsed) {
                    return Err(ImapError::ProviderPolicy);
                }
                match parsed {
                    Response::Done {
                        tag,
                        status: Status::Ok,
                        ..
                    } if tag == &self.tag => return Ok::<_, ImapError>(changed),
                    Response::Done { .. } => return Err(ImapError::Rejected),
                    response if is_bye(response) => {
                        return Err(ImapError::Protocol(
                            "IMAP connection closed before completion".into(),
                        ));
                    }
                    response => changed |= is_change(response),
                }
            }
        })
        .await?;
        Ok((self.session, changed))
    }

    /// Records a change seen while the IDLE was being re-entered, so the next
    /// [`Self::wait`] reports it.
    pub(crate) fn note_change(&mut self) {
        self.pending = true;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::*;
    use crate::imap::scripted_session;

    const FAR: Duration = Duration::from_secs(3600);

    fn never() -> std::future::Pending<()> {
        std::future::pending()
    }

    #[tokio::test]
    async fn change_events_surface() {
        for push in [
            "* 5 EXISTS\r\n",
            "* 2 EXPUNGE\r\n",
            "* 1 FETCH (FLAGS (\\Seen))\r\n",
            "* 1 RECENT\r\n",
            "* VANISHED 4:6\r\n",
        ] {
            let (session, server) =
                scripted_session("IMAP4rev1 IDLE", move |mut peer| async move {
                    peer.enter_idle().await;
                    peer.send(push).await;
                    peer.line().await;
                })
                .await;
            let mut idle = session.start_idle().await.unwrap();
            let wake = idle.wait(Instant::now() + FAR, never()).await.unwrap();
            assert_eq!(wake, IdleWake::Changed, "{push}");
            drop(idle);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn keepalives_and_other_untagged_data_are_ignored() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            peer.enter_idle().await;
            peer.send("* OK Still here\r\n* OK [ALERT] hello\r\n").await;
            tokio::time::sleep(Duration::from_secs(5)).await;
            peer.send("* 7 EXISTS\r\n").await;
            peer.line().await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle.wait(Instant::now() + FAR, never()).await.unwrap();
        assert_eq!(wake, IdleWake::Changed);
        drop(idle);
        server.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn keepalives_do_not_extend_the_deadline() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            peer.enter_idle().await;
            for _ in 0..100 {
                peer.send("* OK Still here\r\n").await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle
            .wait(Instant::now() + Duration::from_secs(10), never())
            .await
            .unwrap();
        assert_eq!(wake, IdleWake::Deadline);
        drop(idle);
        server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_expiry_allows_a_clean_re_idle() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            let tag = peer.enter_idle().await;
            peer.leave_idle(&tag).await;
            let tag = peer.enter_idle().await;
            peer.send("* 9 EXISTS\r\n").await;
            peer.leave_idle(&tag).await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle
            .wait(Instant::now() + Duration::from_secs(600), never())
            .await
            .unwrap();
        assert_eq!(wake, IdleWake::Deadline);
        let (session, changed) = idle.finish().await.unwrap();
        assert!(!changed);
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle.wait(Instant::now() + FAR, never()).await.unwrap();
        assert_eq!(wake, IdleWake::Changed);
        idle.finish().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn manual_stop_is_reported_and_done_is_sent() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            let tag = peer.enter_idle().await;
            peer.leave_idle(&tag).await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle
            .wait(Instant::now() + FAR, std::future::ready(()))
            .await
            .unwrap();
        assert_eq!(wake, IdleWake::Stopped);
        idle.finish().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn bye_and_stream_end_are_disconnects() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            peer.enter_idle().await;
            peer.send("* BYE server shutting down\r\n").await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle.wait(Instant::now() + FAR, never()).await.unwrap();
        assert_eq!(wake, IdleWake::Disconnected);
        server.await.unwrap();

        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            peer.enter_idle().await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle.wait(Instant::now() + FAR, never()).await.unwrap();
        assert_eq!(wake, IdleWake::Disconnected);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn changes_announced_while_leaving_idle_are_reported() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            let tag = peer.enter_idle().await;
            peer.expect("DONE").await;
            peer.send(&format!("* 3 EXISTS\r\n{tag} OK done\r\n")).await;
        })
        .await;
        let mut idle = session.start_idle().await.unwrap();
        let wake = idle
            .wait(Instant::now() + FAR, std::future::ready(()))
            .await
            .unwrap();
        assert_eq!(wake, IdleWake::Stopped);
        let (_, changed) = idle.finish().await.unwrap();
        assert!(changed);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refused_idle_is_an_error() {
        let (session, server) = scripted_session("IMAP4rev1 IDLE", |mut peer| async move {
            let tag = peer.expect("IDLE").await;
            peer.send(&format!("{tag} NO not now\r\n")).await;
        })
        .await;
        assert!(matches!(
            session.start_idle().await,
            Err(ImapError::Rejected)
        ));
        server.await.unwrap();
    }
}
