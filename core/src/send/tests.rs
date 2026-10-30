//! The send queue against in-process fake SMTP servers. Undo windows, backoff
//! and the 24 hour limit run on a manual clock; sockets use real time.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Handle;
use uuid::Uuid;

use mail_parser::MimeHeaders;

use super::types::*;
use super::{Clock, SendConfig};
use crate::accounts::sync_plan::tests::{config, connection_update};
use crate::app::MailxCore;
use crate::drafts::types::Draft;
use crate::events::{CoreEvent, Events};
use crate::net::test_support::{TestTls, dead_port};
use crate::secrets::test_support::MemorySecrets;
use crate::smtp::fake_server::{self, Mode, Script, Seen, Stage};
use crate::smtp::{Timeouts, TlsSource};
use crate::{Account, ErrorCode};

const PASSWORD: &str = "mail-secret";

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
}

fn id() -> String {
    Uuid::new_v4().to_string()
}

struct Fx {
    dir: TempDir,
    core: MailxCore,
    clock: Clock,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    tls: TestTls,
    secrets: Arc<MemorySecrets>,
}

fn open_core(
    dir: &std::path::Path,
    secrets: Arc<MemorySecrets>,
    clock: Clock,
    tls: &TestTls,
    timeouts: Timeouts,
) -> (MailxCore, Arc<Mutex<Vec<CoreEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let sink: Events = Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone()));
    let core = MailxCore::open_with(
        dir,
        secrets,
        Handle::current(),
        sink,
        SendConfig {
            clock,
            timeouts,
            tls: TlsSource::Test(tls.client.clone()),
        },
    )
    .unwrap();
    (core, events)
}

fn fixture() -> Fx {
    fixture_with(Timeouts::for_tests(Duration::from_millis(500)))
}

fn fixture_with(timeouts: Timeouts) -> Fx {
    let dir = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let clock = Clock::manual(t0());
    let tls = TestTls::for_names(&["localhost"]);
    let (core, events) = open_core(dir.path(), secrets.clone(), clock.clone(), &tls, timeouts);
    Fx {
        dir,
        core,
        clock,
        events,
        tls,
        secrets,
    }
}

impl Fx {
    async fn server(&self, change: impl FnOnce(&mut Script)) -> (u16, Seen) {
        let mut script = Script::new(Mode::Implicit(self.tls.server.clone()));
        script.password = PASSWORD;
        change(&mut script);
        fake_server::start(script).await
    }

    fn account_config(email: &str, smtp_port: u16, imap_port: u16) -> serde_json::Value {
        let mut value = config(json!({"kind":"direct"}));
        value["emailAddress"] = json!(email);
        value["imap"] =
            json!({"host":"127.0.0.1","port":imap_port,"tls":"implicit","username":"lin"});
        value["smtp"] =
            json!({"host":"localhost","port":smtp_port,"tls":"implicit","username":"lin"});
        value["authentication"]["password"] = json!({"action":"replace","value":PASSWORD});
        value
    }

    fn account(&self, email: &str, smtp_port: u16) -> Account {
        let imap = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        self.core
            .create_account(
                serde_json::from_value(json!({
                    "requestId": id(), "config": Self::account_config(email, smtp_port, imap),
                }))
                .unwrap(),
            )
            .unwrap()
    }

    fn draft(&self, account: &Account) -> Draft {
        self.core
            .create_draft(
                serde_json::from_value(json!({
                    "accountId": account.id,
                    "to": [{"name": null, "address": "li@example.org"}],
                    "cc": [], "bcc": [], "subject": "hi", "bodyHtml": "hello",
                    "clientMutationId": id(),
                }))
                .unwrap(),
                "",
            )
            .unwrap()
    }

    fn request(draft: &Draft) -> QueueSendRequest {
        QueueSendRequest {
            draft_id: draft.id.clone(),
            expected_revision: draft.revision,
            client_mutation_id: id(),
        }
    }

    fn queue(&self, draft: &Draft, undo: u32) -> OutboxItem {
        self.core
            .queue_send(Caller::Composer, Self::request(draft), undo)
            .unwrap()
    }

    async fn until(
        &self,
        item: &str,
        what: &str,
        done: impl Fn(&OutboxItem) -> bool,
    ) -> OutboxItem {
        for _ in 0..1000 {
            let current = self.core.get_outbox(item).unwrap();
            if done(&current) {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for {what}: {:?}",
            self.core.get_outbox(item)
        );
    }

    async fn reaches(&self, item: &str, state: OutboxState) -> OutboxItem {
        self.until(item, &format!("{state:?}"), |current| {
            current.state == state
        })
        .await
    }

    /// Long enough for a worker poll (10 ms) to have acted on the last change.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    fn states(&self, item: &str) -> Vec<OutboxState> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                CoreEvent::Outbox(changed) if changed.outbox_id == item => Some(changed.state),
                _ => None,
            })
            .collect()
    }

    /// Waits until the announced states are `expected`, the order of the first
    /// two aside. The worker commits a state before it emits the event, and
    /// `queue_send` announces `Queued` only after its own commit, so a worker that
    /// is already running can announce `Sending` first, and a test that saw the
    /// new state in the database can be a moment ahead of the events. The UI
    /// treats events as "reload the outbox", so the order does not matter there.
    async fn states_become(&self, id: &str, expected: &[OutboxState]) {
        let matches = |got: &[OutboxState]| {
            let mut got = got.to_vec();
            if got.len() >= 2 && got[0] == OutboxState::Sending && got[1] == OutboxState::Queued {
                got.swap(0, 1);
            }
            got == expected
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !matches(&self.states(id)) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let got = self.states(id);
        assert!(matches(&got), "{got:?} is not {expected:?}");
    }
}

fn connections(seen: &Seen) -> usize {
    seen.lock().unwrap().connections
}

fn data_count(seen: &Seen) -> usize {
    seen.lock()
        .unwrap()
        .log
        .iter()
        .filter(|entry| entry.as_str() == "BODY-END")
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn queueing_is_idempotent_checks_the_revision_and_the_caller() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);

    let stale = QueueSendRequest {
        expected_revision: draft.revision + 1,
        ..Fx::request(&draft)
    };
    assert_eq!(
        fx.core
            .queue_send(Caller::Composer, stale, 30)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    for caller in [Caller::Mcp, Caller::Model, Caller::MainWindow] {
        assert_eq!(
            fx.core
                .queue_send(caller, Fx::request(&draft), 0)
                .unwrap_err()
                .code,
            ErrorCode::Forbidden
        );
    }
    fx.settle().await;
    assert_eq!(connections(&seen), 0);
    assert!(fx.core.list_outbox(None, true).unwrap().is_empty());

    let request = Fx::request(&draft);
    let first = fx
        .core
        .queue_send(Caller::Composer, request.clone(), 30)
        .unwrap();
    let again = fx.core.queue_send(Caller::Composer, request, 30).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(fx.core.list_outbox(None, true).unwrap().len(), 1);
    let other = fx.draft(&account);
    let reused = QueueSendRequest {
        client_mutation_id: Fx::request(&draft).client_mutation_id,
        ..Fx::request(&other)
    };
    // A fresh id for another draft is fine; the first id for another draft is not.
    fx.core.queue_send(Caller::Composer, reused, 30).unwrap();
    let clash = QueueSendRequest {
        client_mutation_id: first_mutation(&fx, &first),
        ..Fx::request(&fx.draft(&account))
    };
    assert_eq!(
        fx.core
            .queue_send(Caller::Composer, clash, 30)
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
}

fn first_mutation(fx: &Fx, item: &OutboxItem) -> String {
    // The mutation id is not part of the public item; read it back from the row.
    let service = fx.core.service_for_tests();
    let service = service.lock().unwrap();
    service.outbox_mutation_id(&item.id).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_sent_during_the_undo_window_and_canceling_keeps_it_unsent() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 30);
    assert_eq!(item.state, OutboxState::Queued);

    fx.clock.advance(Duration::from_secs(29));
    fx.settle().await;
    assert_eq!(connections(&seen), 0);
    assert_eq!(
        fx.core.get_outbox(&item.id).unwrap().state,
        OutboxState::Queued
    );

    let canceled = fx.core.cancel_outbox(&item.id).unwrap();
    assert_eq!(canceled.state, OutboxState::Canceled);
    fx.clock.advance(Duration::from_secs(3600));
    fx.settle().await;
    assert_eq!(connections(&seen), 0, "a canceled item is never sent");
    assert_eq!(
        fx.core.cancel_outbox(&item.id).unwrap_err().code,
        ErrorCode::Conflict
    );

    // The draft is editable again.
    let update = serde_json::from_value(json!({
        "id": draft.id, "expectedRevision": draft.revision, "accountId": null,
        "to": [{"name": null, "address": "li@example.org"}], "cc": [], "bcc": [],
        "subject": "edited", "bodyHtml": "x",
    }))
    .unwrap();
    assert_eq!(fx.core.update_draft(update).unwrap().subject, "edited");
    assert_eq!(
        fx.states(&item.id),
        [OutboxState::Queued, OutboxState::Canceled]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_item_is_sent_when_the_undo_window_ends_and_then_cannot_be_canceled() {
    let fx = fixture();
    let (port, seen) = fx.server(|script| script.stall = Some(Stage::Final)).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 5);
    fx.clock.advance(Duration::from_secs(5));
    let sending = fx.reaches(&item.id, OutboxState::Sending).await;
    assert_eq!(sending.attempts, 1);
    fx.until(&item.id, "body start mark", |i| i.data_started_at.is_some())
        .await;
    assert_eq!(
        fx.core.cancel_outbox(&item.id).unwrap_err().code,
        ErrorCode::AlreadySending
    );
    assert_eq!(
        fx.core.retry_outbox_now(&item.id).unwrap_err().code,
        ErrorCode::AlreadySending
    );
    assert_eq!(connections(&seen), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_advertised_size_is_kept_on_the_account_for_the_composer_hint() {
    let fx = fixture();
    // 26 MB of whole MIME is about 18 MB of attachments after the 1.37 factor.
    let size = 26 * 1024 * 1024;
    let (port, _seen) = fx.server(|s| s.size = Some(size)).await;
    let account = fx.account("lin@example.com", port);
    let before = fx.core.attachment_policy(&account.id).unwrap();
    assert_eq!(before.server_limit_mb, None);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    fx.reaches(&item.id, OutboxState::Accepted).await;
    fx.settle().await;
    let after = fx.core.attachment_policy(&account.id).unwrap();
    assert_eq!(after.server_limit_mb, Some(18));
    // Only a hint: the enforced limits are unchanged.
    assert_eq!(
        (after.max_total_bytes, after.max_count),
        (20 * 1024 * 1024, 20)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_without_size_leaves_the_policy_default() {
    let fx = fixture();
    let (port, _seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    fx.reaches(&item.id, OutboxState::Accepted).await;
    fx.settle().await;
    assert_eq!(
        fx.core
            .attachment_policy(&account.id)
            .unwrap()
            .server_limit_mb,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn acceptance_keeps_the_copy_deletes_the_draft_and_emits_events() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    let done = fx.reaches(&item.id, OutboxState::Accepted).await;
    assert_eq!(done.server_reply.as_ref().unwrap().code, 250);
    assert_eq!(
        done.accepted_at.as_deref(),
        Some("2026-10-23T12:00:00.000Z")
    );
    assert!(done.data_started_at.is_some());
    assert_eq!(done.draft_id, None);
    assert_eq!(
        fx.core.get_draft(&draft.id).unwrap_err().code,
        ErrorCode::NotFound
    );
    // The stored snapshot is what the server received.
    let stored = fx.core.outbox_mime_for_tests(&item.id);
    assert_eq!(seen.lock().unwrap().body, stored);
    assert!(String::from_utf8_lossy(&stored).contains(&done.message_id));
    // Accepted items leave the default list but stay available.
    assert!(fx.core.list_outbox(None, false).unwrap().is_empty());
    assert_eq!(fx.core.list_outbox(None, true).unwrap().len(), 1);

    fx.states_become(
        &item.id,
        &[
            OutboxState::Queued,
            OutboxState::Sending,
            OutboxState::Accepted,
        ],
    )
    .await;
    let drafts: Vec<DraftChange> = fx
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::DraftChanged(changed) if changed.draft_id == draft.id => {
                Some(changed.change)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        drafts,
        [
            DraftChange::Created,
            DraftChange::Locked,
            DraftChange::Deleted
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn definite_failures_are_recorded_and_never_retried() {
    type Setup = fn(&mut Script);
    let cases: [(&str, Setup, &str, Option<u16>); 7] = [
        (
            "permanent",
            |s| {
                s.replies.insert(Stage::Mail, "550 5.7.1 denied");
            },
            "server",
            Some(550),
        ),
        (
            "recipients",
            |s| s.reject_rcpt = vec!["li@example.org"],
            "recipientsRejected",
            Some(550),
        ),
        (
            "after data",
            |s| {
                s.replies.insert(Stage::Final, "554 5.6.0 bad message");
            },
            "rejectedAfterData",
            Some(554),
        ),
        (
            "too large at mail from",
            |s| {
                s.replies.insert(Stage::Mail, "552 5.3.4 message too big");
            },
            "messageTooLarge",
            Some(552),
        ),
        (
            "too large after data",
            |s| {
                s.replies
                    .insert(Stage::Final, "552 5.2.3 message too large");
            },
            "messageTooLarge",
            Some(552),
        ),
        (
            "bare 552 after data",
            |s| {
                s.replies
                    .insert(Stage::Final, "552 requested action aborted");
            },
            "rejectedAfterData",
            Some(552),
        ),
        ("auth", |s| s.password = "other", "authRejected", Some(535)),
    ];
    for (name, setup, reason, code) in cases {
        let fx = fixture();
        let (port, seen) = fx.server(setup).await;
        let account = fx.account("lin@example.com", port);
        let draft = fx.draft(&account);
        let item = fx.queue(&draft, 0);
        let failed = fx.reaches(&item.id, OutboxState::Failed).await;
        let error = failed.error.clone().unwrap();
        assert_eq!(error.reason, reason, "{name}");
        assert_eq!(error.smtp_code, code, "{name}");
        if reason == "recipientsRejected" {
            assert_eq!(error.rejected.len(), 1);
            assert_eq!(error.rejected[0].address, "li@example.org");
            assert_eq!(data_count(&seen), 0);
        }
        assert_eq!(failed.next_attempt_at, None, "{name}");
        // The draft is back to editable and nothing is retried.
        fx.clock.advance(Duration::from_secs(48 * 3600));
        fx.settle().await;
        assert_eq!(connections(&seen), 1, "{name}");
        assert_eq!(fx.core.get_outbox(&item.id).unwrap().attempts, 1);
        assert!(fx.core.get_draft(&draft.id).is_ok(), "{name}");
        fx.states_become(
            &item.id,
            &[
                OutboxState::Queued,
                OutboxState::Sending,
                OutboxState::Failed,
            ],
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_item_can_be_edited_and_sent_again_with_a_new_message_id() {
    let fx = fixture();
    let (bad, _) = fx.server(|s| s.password = "other").await;
    let (good, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", bad);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    let failed = fx.reaches(&item.id, OutboxState::Failed).await;

    // The user fixes the connection and edits the draft.
    let mut value = Fx::account_config("lin@example.com", good, 1);
    value["imap"]["port"] = json!(account.imap.port);
    fx.core
        .update_account_connection(connection_update(&account.id, account.revision, value))
        .unwrap();
    let edited = fx
        .core
        .update_draft(
            serde_json::from_value(json!({
                "id": draft.id, "expectedRevision": draft.revision, "accountId": null,
                "to": [{"name": null, "address": "li@example.org"}], "cc": [], "bcc": [],
                "subject": "hi again", "bodyHtml": "hello",
            }))
            .unwrap(),
        )
        .unwrap();
    let second = fx.queue(&edited, 0);
    assert_ne!(second.id, item.id);
    assert_ne!(second.message_id, failed.message_id);
    fx.reaches(&second.id, OutboxState::Accepted).await;
    assert_eq!(connections(&seen), 1);
    assert_eq!(
        fx.core.get_outbox(&item.id).unwrap().state,
        OutboxState::Failed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_failures_back_off_on_schedule_and_then_succeed() {
    let fx = fixture();
    let port = dead_port().await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);

    let waits = [30u64, 60, 120, 300, 600, 1800, 1800];
    for (index, expected) in waits.iter().enumerate() {
        let attempts = index as u32 + 1;
        let queued = fx
            .until(&item.id, "requeue", |i| {
                i.state == OutboxState::Queued
                    && i.attempts == attempts
                    && i.next_attempt_at.is_some()
            })
            .await;
        let due =
            crate::send::clock::parse_stamp(queued.next_attempt_at.as_ref().unwrap()).unwrap();
        assert_eq!(
            (due - fx.clock.now()).num_seconds() as u64,
            *expected,
            "attempt {attempts}"
        );
        assert_eq!(queued.error.as_ref().unwrap().reason, "connect");
        // Just before it is due nothing happens.
        fx.clock.advance(Duration::from_secs(expected - 1));
        fx.settle().await;
        assert_eq!(fx.core.get_outbox(&item.id).unwrap().attempts, attempts);
        if index + 1 < waits.len() {
            fx.clock.advance(Duration::from_secs(1));
        }
    }
    // The server comes up where connections were refused.
    let mut script = Script::new(Mode::Implicit(fx.tls.server.clone()));
    script.password = PASSWORD;
    let (_, seen) = fake_server::start_at(port, script).await;
    fx.clock.advance(Duration::from_secs(1));
    let done = fx.reaches(&item.id, OutboxState::Accepted).await;
    assert_eq!(done.attempts, 8);
    assert_eq!(connections(&seen), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_stop_after_24_hours() {
    let fx = fixture();
    let account = fx.account("lin@example.com", dead_port().await);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    fx.until(&item.id, "first failure", |i| {
        i.state == OutboxState::Queued && i.attempts == 1 && i.next_attempt_at.is_some()
    })
    .await;
    fx.clock.advance(Duration::from_secs(25 * 3600));
    let failed = fx.reaches(&item.id, OutboxState::Failed).await;
    assert_eq!(failed.error.unwrap().reason, "retryExhausted");
    assert_eq!(failed.attempts, 2);
    assert!(fx.core.get_draft(&draft.id).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_now_skips_the_backoff() {
    let fx = fixture();
    let port = dead_port().await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    fx.until(&item.id, "first failure", |i| {
        i.state == OutboxState::Queued && i.attempts == 1
    })
    .await;
    let mut script = Script::new(Mode::Implicit(fx.tls.server.clone()));
    script.password = PASSWORD;
    let (_, seen) = fake_server::start_at(port, script).await;
    fx.settle().await;
    assert_eq!(connections(&seen), 0, "still waiting out the backoff");
    let waiting = fx.core.retry_outbox_now(&item.id).unwrap();
    assert_eq!(waiting.next_attempt_at, None);
    fx.reaches(&item.id, OutboxState::Accepted).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_final_reply_is_unknown_and_is_never_retried_automatically() {
    let fx = fixture();
    let (port, seen) = fx.server(|s| s.drop_after_body = true).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    let unknown = fx.reaches(&item.id, OutboxState::Unknown).await;
    assert!(unknown.data_started_at.is_some());
    assert_eq!(unknown.error.unwrap().reason, "unknown");
    fx.clock.advance(Duration::from_secs(1000 * 3600));
    fx.settle().await;
    assert_eq!(connections(&seen), 1);
    assert_eq!(data_count(&seen), 1);
    assert_eq!(
        fx.core.get_outbox(&item.id).unwrap().state,
        OutboxState::Unknown
    );
    // The draft stays locked while the outcome is unknown.
    let locked = serde_json::from_value(json!({
        "id": draft.id, "expectedRevision": draft.revision, "accountId": null,
        "to": [], "cc": [], "bcc": [], "subject": "", "bodyHtml": "",
    }))
    .unwrap();
    assert_eq!(
        fx.core.update_draft(locked).unwrap_err().code,
        ErrorCode::DraftLocked
    );
    assert_eq!(fx.core.list_outbox(None, false).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_item_can_be_marked_sent() {
    let fx = fixture();
    let (port, _) = fx.server(|s| s.drop_after_body = true).await;
    let account = fx.account("lin@example.com", port);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    fx.reaches(&item.id, OutboxState::Unknown).await;
    let request = ResolveUnknownRequest {
        id: item.id.clone(),
        resolution: UnknownResolution::MarkSent,
        client_mutation_id: id(),
    };
    for caller in [Caller::Model, Caller::Mcp] {
        assert_eq!(
            fx.core
                .resolve_unknown(caller, request.clone())
                .unwrap_err()
                .code,
            ErrorCode::Forbidden
        );
    }
    // The main window is interactive UI too.
    let done = fx
        .core
        .resolve_unknown(Caller::MainWindow, request.clone())
        .unwrap();
    assert_eq!(done.state, OutboxState::Accepted);
    assert_eq!(done.resolution, Some(OutboxResolution::MarkedSent));
    assert!(done.accepted_at.is_some());
    assert_eq!(
        fx.core.get_draft(&draft.id).unwrap_err().code,
        ErrorCode::NotFound
    );
    // A replay returns the same result; settling it again another way does not.
    assert_eq!(
        fx.core
            .resolve_unknown(Caller::Composer, request.clone())
            .unwrap()
            .id,
        item.id
    );
    let resend = ResolveUnknownRequest {
        resolution: UnknownResolution::Resend,
        client_mutation_id: id(),
        ..request
    };
    assert_eq!(
        fx.core
            .resolve_unknown(Caller::Composer, resend)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resending_an_unknown_item_keeps_the_message_id_and_the_bytes() {
    let fx = fixture();
    let (lost, first_seen) = fx.server(|s| s.drop_after_body = true).await;
    let (good, second_seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", lost);
    let draft = fx.draft(&account);
    let item = fx.queue(&draft, 0);
    let unknown = fx.reaches(&item.id, OutboxState::Unknown).await;

    // The user points the account at a working server before resending.
    let mut value = Fx::account_config("lin@example.com", good, 1);
    value["imap"]["port"] = json!(account.imap.port);
    fx.core
        .update_account_connection(connection_update(&account.id, account.revision, value))
        .unwrap();
    let request = ResolveUnknownRequest {
        id: item.id.clone(),
        resolution: UnknownResolution::Resend,
        client_mutation_id: id(),
    };
    let next = fx
        .core
        .resolve_unknown(Caller::Composer, request.clone())
        .unwrap();
    assert_ne!(next.id, item.id);
    assert_eq!(next.message_id, unknown.message_id);
    let again = fx.core.resolve_unknown(Caller::Composer, request).unwrap();
    assert_eq!(again.id, next.id, "a replay does not queue a third attempt");
    fx.reaches(&next.id, OutboxState::Accepted).await;

    assert_eq!(
        first_seen.lock().unwrap().body,
        second_seen.lock().unwrap().body
    );
    let old = fx.core.get_outbox(&item.id).unwrap();
    assert_eq!(old.state, OutboxState::Unknown);
    assert_eq!(old.resolution, Some(OutboxResolution::Resent));
    assert_eq!(old.superseded_by.as_deref(), Some(next.id.as_str()));
    assert_eq!(fx.core.list_outbox(None, false).unwrap().len(), 0);
    assert_eq!(connections(&first_seen), 1);
    assert_eq!(data_count(&second_seen), 1);
    // The draft went away with the accepted resend.
    assert_eq!(
        fx.core.get_draft(&draft.id).unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_the_account_address_fails_unsent_items_but_a_new_route_does_not() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let (a, b) = (fx.draft(&account), fx.draft(&account));
    let (first, second) = (fx.queue(&a, 30), fx.queue(&b, 30));

    // A connection change alone keeps the queue and uses it from then on.
    let mut value = Fx::account_config("lin@example.com", port, 1);
    value["imap"]["port"] = json!(account.imap.port);
    value["smtp"]["username"] = json!("lin2");
    let updated = fx
        .core
        .update_account_connection(connection_update(&account.id, account.revision, value))
        .unwrap();
    fx.settle().await;
    assert_eq!(
        fx.core.get_outbox(&first.id).unwrap().state,
        OutboxState::Queued
    );

    let mut value = Fx::account_config("other@example.com", port, 1);
    value["imap"]["port"] = json!(account.imap.port);
    fx.core
        .update_account_connection(connection_update(&account.id, updated.revision, value))
        .unwrap();
    for item in [&first, &second] {
        let failed = fx.reaches(&item.id, OutboxState::Failed).await;
        assert_eq!(failed.error.unwrap().reason, "accountChanged");
    }
    fx.clock.advance(Duration::from_secs(3600));
    fx.settle().await;
    assert_eq!(connections(&seen), 0);
    assert!(fx.core.get_draft(&a.id).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn one_accounts_stuck_server_does_not_block_another() {
    let fx = fixture_with(Timeouts::for_tests(Duration::from_secs(5)));
    let (stuck, stuck_seen) = fx.server(|s| s.stall = Some(Stage::Greeting)).await;
    let (fine, _) = fx.server(|_| {}).await;
    let slow = fx.account("slow@example.com", stuck);
    let quick = fx.account("quick@example.com", fine);
    let (a, b) = (fx.draft(&slow), fx.draft(&quick));
    let first = fx.queue(&a, 0);
    fx.until(&first.id, "connection", |_| connections(&stuck_seen) == 1)
        .await;
    let second = fx.queue(&b, 0);
    fx.reaches(&second.id, OutboxState::Accepted).await;
    assert_eq!(
        fx.core.get_outbox(&first.id).unwrap().state,
        OutboxState::Sending,
        "still waiting for its greeting"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_recovers_interrupted_and_waiting_items() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let (a, b, c) = (fx.draft(&account), fx.draft(&account), fx.draft(&account));
    let waiting = fx.queue(&a, 30);
    let (interrupted, started) = (fx.queue(&b, 30), fx.queue(&c, 30));
    let Fx {
        dir,
        core,
        clock,
        tls,
        secrets,
        ..
    } = fx;
    drop(core);
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let mut service =
            crate::accounts::AccountService::open(dir.path(), secrets.clone()).unwrap();
        service
            .force_sending_for_tests(&[(&interrupted.id, false), (&started.id, true)], t0())
            .unwrap();
    }
    let (core, events) = open_core(
        dir.path(),
        secrets,
        clock.clone(),
        &tls,
        Timeouts::for_tests(Duration::from_millis(500)),
    );
    assert_eq!(
        core.get_outbox(&interrupted.id).unwrap().state,
        OutboxState::Queued
    );
    assert_eq!(core.get_outbox(&interrupted.id).unwrap().attempts, 0);
    assert_eq!(
        core.get_outbox(&started.id).unwrap().state,
        OutboxState::Unknown
    );
    assert_eq!(
        core.get_outbox(&waiting.id).unwrap().state,
        OutboxState::Queued
    );
    assert!(events.lock().unwrap().iter().any(|event| matches!(
        event,
        CoreEvent::Outbox(changed) if changed.outbox_id == started.id && changed.state == OutboxState::Unknown
    )));
    // Queued items are picked up after the restart once their time comes.
    assert_eq!(connections(&seen), 0);
    clock.advance(Duration::from_secs(30));
    for id in [&waiting.id, &interrupted.id] {
        for _ in 0..500 {
            if core.get_outbox(id).unwrap().state == OutboxState::Accepted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(core.get_outbox(id).unwrap().state, OutboxState::Accepted);
    }
    assert_eq!(data_count(&seen), 2);
    assert_eq!(
        core.get_outbox(&started.id).unwrap().state,
        OutboxState::Unknown
    );
}

fn blank_request(account: &Account) -> crate::CreateDraftRequest {
    serde_json::from_value(json!({
        "accountId": account.id, "to": [], "cc": [], "bcc": [],
        "subject": " ", "bodyHtml": "\n ", "clientMutationId": id(),
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_operations_emit_events_and_blank_drafts_are_hidden() {
    let fx = fixture();
    let account = fx.account("lin@example.com", 1);
    let blank = fx.core.create_draft(blank_request(&account), "").unwrap();
    assert!(fx.core.list_drafts(None).unwrap().is_empty());
    assert!(blank.is_blank());

    let update = |draft: &Draft, subject: &str| crate::UpdateDraftRequest {
        id: draft.id.clone(),
        expected_revision: draft.revision,
        account_id: None,
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        body_html: String::new(),
    };
    let edited = fx.core.update_draft(update(&blank, "hi")).unwrap();
    assert_eq!(fx.core.list_drafts(None).unwrap().len(), 1);
    let cleared = fx.core.update_draft(update(&edited, "")).unwrap();
    assert!(fx.core.list_drafts(None).unwrap().is_empty());
    fx.core.delete_draft(&cleared.id).unwrap();

    let changes: Vec<DraftChange> = fx
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::DraftChanged(changed) if changed.draft_id == blank.id => {
                assert_eq!(changed.account_id, account.id);
                Some(changed.change)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        [
            DraftChange::Created,
            DraftChange::Updated,
            DraftChange::Updated,
            DraftChange::Deleted
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_the_core_purges_blank_drafts_only() {
    let fx = fixture();
    let account = fx.account("lin@example.com", 1);
    let blank = fx.core.create_draft(blank_request(&account), "").unwrap();
    let kept = fx.draft(&account);
    let Fx {
        dir,
        core,
        clock,
        tls,
        secrets,
        ..
    } = fx;
    drop(core);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (core, _) = open_core(
        dir.path(),
        secrets,
        clock,
        &tls,
        Timeouts::for_tests(Duration::from_millis(500)),
    );
    assert_eq!(
        core.get_draft(&blank.id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert!(core.get_draft(&kept.id).is_ok());
}

fn store_connection(fx: &Fx) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(fx.dir.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db
}

/// The blob file behind a snapshot hash, as the store lays it out.
fn snapshot_path(fx: &Fx, hash: &str) -> std::path::PathBuf {
    fx.dir.path().join("blobs").join(&hash[..2]).join(hash)
}

fn snapshot_of(fx: &Fx, item: &str) -> (Option<String>, Option<i64>) {
    store_connection(fx)
        .query_row(
            "SELECT snapshot_hash, snapshot_size FROM outbox WHERE id = ?1",
            [item],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn attached(fx: &Fx, draft: &Draft, name: &str, bytes: &[u8]) -> Draft {
    fx.core
        .add_attachment(
            crate::AddAttachmentRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                file_name: name.to_owned(),
            },
            bytes,
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_sent_from_the_frozen_snapshot_after_the_draft_is_gone() {
    let fx = fixture();
    let (port, seen) = fx.server(|_| {}).await;
    let account = fx.account("lin@example.com", port);
    let payload = b"%PDF-1.7 synthetic attachment bytes".to_vec();
    let draft = attached(&fx, &fx.draft(&account), "报告 终稿.pdf", &payload);
    assert_eq!(draft.attachments.len(), 1);
    let item = fx.queue(&draft, 30);
    assert_eq!(item.state, OutboxState::Queued);

    // Delete the draft out from under the queued item.
    store_connection(&fx)
        .execute("DELETE FROM drafts WHERE id = ?1", [&draft.id])
        .unwrap();
    let rows: i64 = store_connection(&fx)
        .query_row("SELECT COUNT(*) FROM draft_attachments", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0, "the attachment references go with the draft");

    fx.clock.advance(Duration::from_secs(31));
    let done = fx.reaches(&item.id, OutboxState::Accepted).await;
    assert_eq!(done.draft_id, None);
    let body = seen.lock().unwrap().body.clone();
    let parsed = mail_parser::MessageParser::default().parse(&body).unwrap();
    assert_eq!(parsed.attachment_count(), 1);
    let part = parsed.attachment(0).unwrap();
    assert_eq!(part.attachment_name(), Some("报告 终稿.pdf"));
    assert_eq!(part.contents(), payload.as_slice());
    assert_eq!(body, fx.core.outbox_mime_for_tests(&item.id));
}

fn inline_attached(fx: &Fx, draft: &Draft, name: &str) -> (Draft, String) {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 3, image::Rgb([9, 9, 9])))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (draft, images) = fx
        .core
        .add_inline_images(
            &draft.id,
            draft.revision,
            vec![(name.to_owned(), png.get_ref().clone())],
        )
        .unwrap();
    (draft, images[0].content_id.clone())
}

fn with_body(fx: &Fx, draft: &Draft, body_html: String) -> Draft {
    fx.core
        .update_draft(crate::UpdateDraftRequest {
            id: draft.id.clone(),
            expected_revision: draft.revision,
            account_id: None,
            to: draft.to.clone(),
            cc: vec![],
            bcc: vec![],
            subject: draft.subject.clone(),
            body_html,
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn freezing_sends_only_referenced_inline_images_and_drops_dangling_references() {
    let fx = fixture();
    let port = dead_port().await;
    let account = fx.account("lin@example.com", port);
    let draft = attached(&fx, &fx.draft(&account), "a.pdf", b"%PDF-1.7");
    let (draft, used) = inline_attached(&fx, &draft, "used.png");
    // Adding the next file would otherwise drop `used`, which the saved body
    // does not show yet.
    let draft = with_body(&fx, &draft, format!("<p><img src=\"cid:{used}\"></p>"));
    let (draft, unused) = inline_attached(&fx, &draft, "unused.png");
    let dangling = "00000000000000000000000000000000@example.com";
    let body = format!(
        "<p>hi<img src=\"cid:{used}\"><img src=\"cid:{dangling}\" alt=\"ghost\"><img src=\"cid:{used}\"></p>"
    );
    let draft = with_body(&fx, &draft, body);
    let item = fx.queue(&draft, 30);
    let raw = fx.core.outbox_mime_for_tests(&item.id);
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();

    let text = String::from_utf8_lossy(&raw).into_owned();
    assert!(text.contains("Content-Type: multipart/mixed"));
    assert!(text.contains("Content-Type: multipart/related"));
    let sent: Vec<_> = parsed
        .attachments()
        .map(|part| part.attachment_name().unwrap().to_owned())
        .collect();
    assert_eq!(sent, ["used.png", "a.pdf"], "unused.png is not sent");
    assert_eq!(text.matches(&format!("Content-ID: <{used}>")).count(), 1);
    assert!(!text.contains(&unused));
    assert!(!text.contains(dangling), "the dangling <img> is removed");
    let html = parsed.body_html(0).unwrap();
    assert!(html.contains(&format!("cid:{used}")));
    let plain = parsed.body_text(0).unwrap();
    assert!(!plain.contains("ghost"));
    assert!(plain.contains("[used.png]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_send_the_identical_snapshot_bytes() {
    let fx = fixture();
    let port = dead_port().await;
    let account = fx.account("lin@example.com", port);
    let draft = attached(&fx, &fx.draft(&account), "a.bin", b"\x00\x01binary");
    let item = fx.queue(&draft, 0);
    let queued = fx
        .until(&item.id, "first requeue", |i| {
            i.state == OutboxState::Queued && i.error.is_some()
        })
        .await;
    let (hash, size) = snapshot_of(&fx, &item.id);
    let hash = hash.expect("a queued item always has its snapshot");
    assert_eq!(
        size,
        Some(fx.core.outbox_mime_for_tests(&item.id).len() as i64)
    );

    // The server comes up where connections were refused.
    let mut script = Script::new(Mode::Implicit(fx.tls.server.clone()));
    script.password = PASSWORD;
    let (_, seen) = fake_server::start_at(port, script).await;
    fx.clock.advance(Duration::from_secs(31));
    fx.reaches(&item.id, OutboxState::Accepted).await;
    assert_eq!(queued.attempts, 1);
    assert_eq!(snapshot_of(&fx, &item.id).0.as_deref(), Some(hash.as_str()));
    assert_eq!(
        seen.lock().unwrap().body,
        fx.core.outbox_mime_for_tests(&item.id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_or_corrupt_snapshot_fails_before_any_data_is_sent() {
    for corrupt in [false, true] {
        let fx = fixture();
        let (port, seen) = fx.server(|_| {}).await;
        let account = fx.account("lin@example.com", port);
        let draft = attached(&fx, &fx.draft(&account), "a.bin", b"snapshot bytes");
        // The undo window keeps the worker away until the blob is damaged.
        let item = fx.queue(&draft, 30);
        let (hash, _) = snapshot_of(&fx, &item.id);
        let path = snapshot_path(&fx, hash.as_ref().unwrap());
        if corrupt {
            std::fs::write(&path, b"tampered").unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        fx.clock.advance(Duration::from_secs(31));

        let failed = fx.reaches(&item.id, OutboxState::Failed).await;
        assert_eq!(
            failed.error.as_ref().unwrap().reason,
            "snapshotMissing",
            "corrupt: {corrupt}"
        );
        assert_eq!(
            data_count(&seen),
            0,
            "no body may reach the server, corrupt: {corrupt}"
        );
        assert_eq!(
            fx.core.get_outbox(&item.id).unwrap().state,
            OutboxState::Failed,
            "a lost snapshot is never retried or rebuilt"
        );
        // The draft is editable again: its own attachment bytes are intact.
        assert_eq!(fx.core.get_draft(&draft.id).unwrap().attachments.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_draft_aborts_the_freeze_and_leaves_no_queue_row() {
    let fx = fixture();
    let account = fx.account("lin@example.com", 9);
    let draft = attached(&fx, &fx.draft(&account), "a.bin", b"bytes");
    let request = Fx::request(&draft);
    let service = fx.core.service_for_tests();

    let plan = {
        let service = service.lock().unwrap();
        service.prepare_send(&request, 0).unwrap()
    };
    let crate::accounts::outbox::SendPlan::Build(build) = plan else {
        panic!("a fresh draft is not a replay")
    };
    let frozen = build.freeze().unwrap();

    // A body edit between the two phases moves the revision on.
    fx.core
        .update_draft(
            serde_json::from_value(json!({
                "id": draft.id, "expectedRevision": draft.revision, "accountId": null,
                "to": [{"name": null, "address": "li@example.org"}], "cc": [], "bcc": [],
                "subject": "edited meanwhile", "bodyHtml": "x",
            }))
            .unwrap(),
        )
        .unwrap();
    let error = service
        .lock()
        .unwrap()
        .commit_send(&request, 0, &build, frozen, t0())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(fx.core.list_outbox(None, true).unwrap().is_empty());
    assert!(!fx.core.get_draft(&draft.id).unwrap().attachments.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_requeue_releases_the_superseded_items_snapshot() {
    let fx = fixture();
    let (port, seen) = fx
        .server(|script| {
            script.replies.insert(Stage::Mail, "554 5.7.1 nope");
        })
        .await;
    let account = fx.account("lin@example.com", port);
    let draft = attached(&fx, &fx.draft(&account), "a.bin", b"bytes");
    let first = fx.queue(&draft, 0);
    fx.reaches(&first.id, OutboxState::Failed).await;
    let (hash, _) = snapshot_of(&fx, &first.id);
    assert!(
        hash.is_some(),
        "a failed item keeps its snapshot until replaced"
    );

    let editable = fx.core.get_draft(&draft.id).unwrap();
    let second = fx.queue(&editable, 0);
    let released: (Option<String>, Option<String>) = store_connection(&fx)
        .query_row(
            "SELECT snapshot_hash, snapshot_released_at FROM outbox WHERE id = ?1",
            [&first.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(released.0, None);
    assert!(released.1.is_some());
    assert!(snapshot_of(&fx, &second.id).0.is_some());
    assert!(seen.lock().unwrap().connections >= 1);
}
