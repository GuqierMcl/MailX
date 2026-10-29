use std::{sync::Arc, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    runtime::Handle,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};

use super::*;
use crate::mail::PreviewState;
use crate::{
    accounts::types::TlsMode,
    imap::{ImapLogin, PREVIEW_BYTES, PREVIEW_FALLBACK_BYTES},
    mail::BodyState,
    secrets::SecretValue,
    sync::bodies::tests::{
        Fixture, RAW, connection, create_account, fetch, fixture, seed, session, tagged_command,
        wait_until,
    },
};

type Connections = mpsc::UnboundedReceiver<oneshot::Sender<Result<imap::Session, ImapError>>>;

struct Harness {
    fixture: Fixture,
    manager: PreviewManager,
    connections: Connections,
    events: Arc<Mutex<Vec<EmailPreviews>>>,
}

fn harness(sizes: &[Option<u32>]) -> Harness {
    let fixture = fixture(sizes);
    let events: Arc<Mutex<Vec<EmailPreviews>>> = Arc::default();
    let sink = events.clone();
    let events_sink: Events = Arc::new(move |event: &CoreEvent| {
        if let CoreEvent::Previews(previews) = event {
            sink.lock().unwrap().push(previews.clone());
        }
    });
    let mut manager = PreviewManager::new(&fixture.service, Handle::current(), events_sink);
    let (connector, connections) = mpsc::unbounded_channel();
    Arc::get_mut(&mut manager.shared).unwrap().connector = Some(connector);
    Harness {
        fixture,
        manager,
        connections,
        events,
    }
}

impl Harness {
    fn request(&self, indexes: &[usize]) {
        self.manager.request(
            indexes
                .iter()
                .map(|index| self.fixture.messages[*index].clone())
                .collect(),
        );
    }

    async fn connection(&mut self) -> oneshot::Sender<Result<imap::Session, ImapError>> {
        timeout(Duration::from_secs(3), self.connections.recv())
            .await
            .unwrap()
            .unwrap()
    }

    fn preview(&self, index: usize) -> (PreviewState, String) {
        let summary = self
            .fixture
            .service
            .lock()
            .unwrap()
            .get_email(&self.fixture.messages[index])
            .unwrap()
            .summary;
        (summary.preview_state, summary.preview)
    }

    async fn idle(&self) {
        wait_until(|| self.manager.shared.state.lock().unwrap().lanes.is_empty()).await;
    }

    async fn applied(&self, sequence: u64) {
        wait_until(|| self.manager.shared.state.lock().unwrap().applied >= sequence).await;
    }
}

fn missing(harness: &Harness, index: usize) -> EmailPreviewItem {
    EmailPreviewItem {
        email_id: harness.fixture.messages[index].clone(),
        preview: String::new(),
        preview_state: PreviewState::Missing,
    }
}

fn header(content_type: &str) -> Vec<u8> {
    format!("Content-Type: {content_type}\r\n\r\n").into_bytes()
}

fn fetched(uid: u32, header: &[u8], text: &[u8]) -> Vec<u8> {
    let mut reply = format!(
        "* {uid} FETCH (UID {uid} BODY[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] {{{}}}\r\n",
        header.len()
    )
    .into_bytes();
    reply.extend_from_slice(header);
    reply.extend_from_slice(format!(" BODY[TEXT]<0> {{{}}}\r\n", text.len()).as_bytes());
    reply.extend_from_slice(text);
    reply.extend_from_slice(b")\r\n");
    reply
}

/// A preview session that expects exactly `UID FETCH {uids}` and answers
/// `reply` once released.
async fn preview_session(
    uids: &str,
    reply: Vec<u8>,
) -> (imap::Session, oneshot::Sender<()>, JoinHandle<()>) {
    preview_session_validity(uids, reply, 7).await
}

/// Like [`preview_session`], but the mailbox reports `validity`; any value but
/// the seeded 7 ends the exchange right after EXAMINE.
async fn preview_session_validity(
    uids: &str,
    reply: Vec<u8>,
    validity: u32,
) -> (imap::Session, oneshot::Sender<()>, JoinHandle<()>) {
    let (client, server): (DuplexStream, DuplexStream) = tokio::io::duplex(64 * 1024);
    let (release, released) = oneshot::channel();
    let expected = format!(
        "UID FETCH {uids} (UID BODY.PEEK[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] BODY.PEEK[TEXT]<0.{PREVIEW_BYTES}>)"
    );
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        server
            .get_mut()
            .write_all(b"* OK synthetic\r\n")
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "LOGIN \"fixture\" \"synthetic-password\"").await;
        server
            .get_mut()
            .write_all(format!("{tag} OK\r\n").as_bytes())
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "CAPABILITY").await;
        server
            .get_mut()
            .write_all(format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n").as_bytes())
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "EXAMINE \"INBOX\"").await;
        server
            .get_mut()
            .write_all(
                format!("* OK [UIDVALIDITY {validity}]\r\n{tag} OK [READ-ONLY]\r\n").as_bytes(),
            )
            .await
            .unwrap();
        if validity != 7 {
            let _ = released.await;
            return;
        }
        let tag = tagged_command(&mut server, &expected).await;
        if released.await.is_err() {
            return;
        }
        let _ = server.get_mut().write_all(&reply).await;
        let _ = server
            .get_mut()
            .write_all(format!("{tag} OK\r\n").as_bytes())
            .await;
        let mut rest = String::new();
        let _ = server.read_line(&mut rest).await;
    });
    let login = ImapLogin {
        host: "imap.example.test".into(),
        port: 993,
        tls: TlsMode::Implicit,
        username: "fixture".into(),
        password: SecretValue::new("synthetic-password".into()),
    };
    let session = imap::test_body_session(Box::new(client), &login)
        .await
        .unwrap();
    (session, release, server)
}

#[tokio::test]
async fn ready_failed_and_untouched_outcomes_are_stored_and_announced() {
    let mut harness = harness(&[None, None, None]);
    harness.request(&[0, 1, 2]);
    let request = harness.connection().await;
    let mut reply = fetched(1, &header("text/plain"), b"Hello   preview\r\nline two");
    reply.extend(fetched(
        2,
        &header("text/html"),
        b"<style>p{color:red}</style>",
    ));
    let (session, release, server) = preview_session("1,2,3", reply).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    assert_eq!(
        harness.preview(0),
        (PreviewState::Ready, "Hello preview line two".into())
    );
    assert_eq!(harness.preview(1), (PreviewState::Failed, String::new()));
    // Absent from the reply (gone from the server): decided later by header sync.
    assert_eq!(harness.preview(2), (PreviewState::Missing, String::new()));
    let events = harness.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].items,
        [
            EmailPreviewItem {
                email_id: harness.fixture.messages[0].clone(),
                preview: "Hello preview line two".into(),
                preview_state: PreviewState::Ready,
            },
            EmailPreviewItem {
                email_id: harness.fixture.messages[1].clone(),
                preview: String::new(),
                preview_state: PreviewState::Failed,
            },
        ]
    );
    // The row the server did not return is announced as an attempt without result.
    assert_eq!(events[1].items, [missing(&harness, 2)]);
}

#[tokio::test]
async fn resolved_ids_are_not_fetched_again() {
    let mut harness = harness(&[None, None]);
    harness.request(&[0]);
    let request = harness.connection().await;
    let (session, release, server) =
        preview_session("1", fetched(1, &header("text/plain"), b"first")).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    // The second request names both rows but only the unresolved one is asked for.
    harness.request(&[0, 1]);
    let request = harness.connection().await;
    let (session, release, server) =
        preview_session("2", fetched(2, &header("text/plain"), b"second")).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    assert_eq!(harness.preview(1), (PreviewState::Ready, "second".into()));
    // Everything already resolved: nothing to connect for.
    harness.request(&[0, 1]);
    harness.applied(3).await;
    assert!(harness.connections.try_recv().is_err());
    assert!(
        harness
            .manager
            .shared
            .state
            .lock()
            .unwrap()
            .lanes
            .is_empty()
    );
}

#[tokio::test]
async fn the_newest_request_replaces_work_that_has_not_started() {
    let mut harness = harness(&[None, None, None, None]);
    harness.request(&[0]);
    let running = harness.connection().await;
    // A batch is running (blocked on its connection); two more requests arrive.
    harness.request(&[1, 2]);
    harness.request(&[3]);
    harness.applied(3).await;
    let (session, release, server) =
        preview_session("1", fetched(1, &header("text/plain"), b"one")).await;
    assert!(running.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    // The running batch finished; only the latest wanted set follows it.
    let next = harness.connection().await;
    let (session, release, server_two) =
        preview_session("4", fetched(4, &header("text/plain"), b"four")).await;
    assert!(next.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    server_two.abort();
    assert_eq!(harness.preview(0).0, PreviewState::Ready);
    assert_eq!(harness.preview(1).0, PreviewState::Missing);
    assert_eq!(harness.preview(2).0, PreviewState::Missing);
    assert_eq!(harness.preview(3).0, PreviewState::Ready);
    // Rows 1 and 2 were only replaced in the queue: unwanted, so no event.
    let events = harness.events.lock().unwrap();
    let announced: Vec<&String> = events
        .iter()
        .flat_map(|event| event.items.iter().map(|item| &item.email_id))
        .collect();
    assert_eq!(announced.len(), 2);
    assert!(!announced.contains(&&harness.fixture.messages[1]));
    assert!(!announced.contains(&&harness.fixture.messages[2]));
}

#[tokio::test]
async fn previews_never_queue_a_body_download_behind_them() {
    let mut harness = harness(&[None, None]);
    harness.request(&[0]);
    // The preview batch is in flight (holding its connection attempt open).
    let preview = harness.connection().await;
    let body_job = fetch(&harness.fixture, 1);
    let request = connection(&mut harness.fixture).await;
    let (session, release, server) = session(2, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    let detail = timeout(Duration::from_secs(3), body_job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.body_state, BodyState::Ready);
    assert_eq!(detail.summary.preview_state, PreviewState::Ready);
    server.await.unwrap();
    // The preview was still waiting the whole time.
    assert_eq!(harness.preview(0).0, PreviewState::Missing);
    drop(preview);
    harness.idle().await;
}

#[tokio::test]
async fn transient_failures_leave_rows_missing_and_drop_the_queue() {
    let mut harness = harness(&[None, None]);
    harness.request(&[0, 1]);
    let request = harness.connection().await;
    assert!(request.send(Err(ImapError::Timeout)).is_ok());
    harness.idle().await;
    assert_eq!(harness.preview(0).0, PreviewState::Missing);
    assert_eq!(harness.preview(1).0, PreviewState::Missing);
    {
        // The failed batch and the dropped rest of the queue are both announced.
        let events = harness.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].items.len(), 2);
        assert!(events[0].items.contains(&missing(&harness, 0)));
        assert!(events[0].items.contains(&missing(&harness, 1)));
    }
    // A later request starts over.
    harness.request(&[0]);
    let _ = harness.connection().await;
}

#[tokio::test]
async fn a_changed_uid_validity_announces_the_batch_as_missing() {
    let mut harness = harness(&[None, None]);
    harness.request(&[0, 1]);
    let request = harness.connection().await;
    let (session, release, server) = preview_session_validity("1,2", Vec::new(), 8).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    assert_eq!(harness.preview(0).0, PreviewState::Missing);
    let events = harness.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].items,
        [missing(&harness, 0), missing(&harness, 1)]
    );
}

/// Runs one scripted batch to completion on the next connection.
async fn answer(harness: &mut Harness, uids: &str, reply: Vec<u8>) {
    let request = harness.connection().await;
    let (session, release, server) = preview_session(uids, reply).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn a_server_ignoring_the_partial_range_is_truncated_locally() {
    let mut harness = harness(&[Some(41_000)]);
    harness.request(&[0]);
    let mut text = b"Whole text kept short ".to_vec();
    text.resize(40 * 1024, b'x');
    let request = harness.connection().await;
    let reply = fetched(1, &header("text/plain"), &text);
    let (session, release, server) = preview_session("1", reply).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    harness.idle().await;
    server.abort();
    let (state, preview) = harness.preview(0);
    assert_eq!(state, PreviewState::Ready);
    assert!(preview.starts_with("Whole text kept short xxx"));
}

#[tokio::test]
async fn a_literal_beyond_the_fallback_cap_is_recorded_as_failed() {
    let mut harness = harness(&[Some(2 * PREVIEW_FALLBACK_BYTES as u32), Some(100)]);
    harness.request(&[0, 1]);
    // Over the cap by a little, so the budget admits it and it is rejected
    // as Oversized rather than TooLarge.
    let long = vec![b'x'; PREVIEW_FALLBACK_BYTES as usize + 1];
    answer(&mut harness, "1", fetched(1, &header("text/plain"), &long)).await;
    answer(
        &mut harness,
        "2",
        fetched(2, &header("text/plain"), b"kept"),
    )
    .await;
    harness.idle().await;
    assert_eq!(harness.preview(0), (PreviewState::Failed, String::new()));
    assert_eq!(harness.preview(1), (PreviewState::Ready, "kept".into()));
}

#[tokio::test]
async fn large_messages_are_fetched_alone_and_others_still_succeed() {
    let large = 3 * PREVIEW_FALLBACK_BYTES as u32;
    let mut harness = harness(&[Some(100), Some(large), Some(200), None, Some(large)]);
    harness.request(&[0, 1, 2, 3, 4]);
    // The small ones are grouped; each large one gets its own request, in
    // order. The first large one is served in full by a server that ignores
    // the range, which outgrows its budget: only that message fails.
    answer(
        &mut harness,
        "1,3,4",
        [
            fetched(1, &header("text/plain"), b"one"),
            fetched(3, &header("text/plain"), b"three"),
            fetched(4, &header("text/plain"), b"four"),
        ]
        .concat(),
    )
    .await;
    let huge = vec![b'x'; large as usize];
    answer(&mut harness, "2", fetched(2, &header("text/plain"), &huge)).await;
    // A server honouring the range keeps the other cheap.
    answer(
        &mut harness,
        "5",
        fetched(5, &header("text/plain"), b"five"),
    )
    .await;
    harness.idle().await;
    assert_eq!(harness.preview(0), (PreviewState::Ready, "one".into()));
    assert_eq!(harness.preview(1), (PreviewState::Failed, String::new()));
    assert_eq!(harness.preview(2), (PreviewState::Ready, "three".into()));
    assert_eq!(harness.preview(3), (PreviewState::Ready, "four".into()));
    assert_eq!(harness.preview(4), (PreviewState::Ready, "five".into()));
}

#[tokio::test]
async fn batches_stop_growing_at_the_byte_cap() {
    let mib = 1024 * 1024;
    // Each message counts at most the 1 MiB cap (unknown too), so four fill
    // the 4 MiB cap and the fifth starts the next batch.
    let mut harness = harness(&[Some(mib), Some(mib), None, Some(mib), Some(mib), Some(100)]);
    harness.request(&[0, 1, 2, 3, 4, 5]);
    let ok = |uid: u32| fetched(uid, &header("text/plain"), b"x");
    answer(
        &mut harness,
        "1,2,3,4",
        [ok(1), ok(2), ok(3), ok(4)].concat(),
    )
    .await;
    answer(&mut harness, "5,6", [ok(5), ok(6)].concat()).await;
    harness.idle().await;
    for index in 0..6 {
        assert_eq!(harness.preview(index), (PreviewState::Ready, "x".into()));
    }
}

#[tokio::test]
async fn a_too_large_batch_with_wrong_sizes_retries_each_message_alone() {
    // The stored sizes claim both are small, but the server sends 2 MiB.
    let mut harness = harness(&[Some(100), Some(100)]);
    harness.request(&[0, 1]);
    let huge = vec![b'x'; 2 * PREVIEW_FALLBACK_BYTES as usize];
    answer(
        &mut harness,
        "1,2",
        [
            fetched(1, &header("text/plain"), &huge),
            fetched(2, &header("text/plain"), b"small"),
        ]
        .concat(),
    )
    .await;
    answer(&mut harness, "1", fetched(1, &header("text/plain"), &huge)).await;
    answer(
        &mut harness,
        "2",
        fetched(2, &header("text/plain"), b"small"),
    )
    .await;
    harness.idle().await;
    assert_eq!(harness.preview(0), (PreviewState::Failed, String::new()));
    assert_eq!(harness.preview(1), (PreviewState::Ready, "small".into()));
}

#[tokio::test]
async fn accounts_have_independent_lanes() {
    let mut harness = harness(&[None]);
    let other = {
        let mut service = harness.fixture.service.lock().unwrap();
        let account = create_account(&mut service, "second");
        seed(&mut service, &account, &[None]).1
    };
    harness
        .manager
        .request(vec![harness.fixture.messages[0].clone(), other[0].clone()]);
    let first = harness.connection().await;
    let second = harness.connection().await;
    drop((first, second));
    harness.idle().await;
}
