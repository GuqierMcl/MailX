//! Scripted IMAP plus real temporary SQLite tests for the per-folder driver.

use super::*;
use crate::accounts::sync_plan::tests::{config, connection_update};
use crate::events::{CoreEvent, NewMail};
use crate::imap::{FolderStatus, ImapLogin};
use crate::mail::{EmailListQuery, FolderRole};
use crate::secrets::{SecretValue, test_support::MemorySecrets};
use crate::sync::cadence::{Activity, Cadence};
use crate::sync::folders::FolderRecord;
use crate::sync::messages::FolderMemory;
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

struct Fixture {
    _directory: TempDir,
    service: Arc<Mutex<AccountService>>,
    shared: Arc<Shared>,
    account_id: String,
    source: String,
    token: WorkerToken,
    events: Arc<Mutex<Vec<AccountSyncState>>>,
    new_mail: Arc<Mutex<Vec<NewMail>>>,
}

fn fixture() -> Fixture {
    fixture_with_config(config(json!({"kind":"direct"})))
}

fn fixture_with_config(account_config: serde_json::Value) -> Fixture {
    fixture_with(account_config, FolderRole::Inbox)
}

fn fixture_with(account_config: serde_json::Value, role: FolderRole) -> Fixture {
    let directory = TempDir::new().unwrap();
    let mut service =
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let account = service
        .create(
            serde_json::from_value(json!({
                "requestId": uuid::Uuid::new_v4().to_string(),
                "config": account_config
            }))
            .unwrap(),
        )
        .unwrap();
    service
        .store_folders(
            &account.id,
            &account.mailbox_source_id,
            &[FolderRecord {
                remote_name: "INBOX".into(),
                parent_remote_name: None,
                name: "INBOX".into(),
                delimiter: Some("/".into()),
                role,
                selectable: true,
                attributes: vec![],
                status: Some(FolderStatus {
                    messages: 2,
                    unseen: 2,
                    uid_next: Some(4),
                    uid_validity: Some(21),
                }),
            }],
        )
        .unwrap();
    let service = Arc::new(Mutex::new(service));
    let events = Arc::new(Mutex::new(Vec::new()));
    let new_mail = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let arrived = new_mail.clone();
    let shared = Arc::new(Shared {
        service: Arc::downgrade(&service),
        states: Mutex::new(HashMap::new()),
        global_sync_window: Mutex::new(crate::settings::SyncWindow::Days30),
        events: Arc::new(move |event: &CoreEvent| match event {
            CoreEvent::Sync(state) => observed.lock().unwrap().push(state.clone()),
            CoreEvent::NewMail(mail) => arrived.lock().unwrap().push(mail.clone()),
            CoreEvent::FlagWriteBack(_)
            | CoreEvent::Deletions(_)
            | CoreEvent::Outbox(_)
            | CoreEvent::DraftChanged(_)
            | CoreEvent::SentCopies(_)
            | CoreEvent::Assistant(_)
            | CoreEvent::AssistantConversationsChanged(_)
            | CoreEvent::Previews(_)
            | CoreEvent::AccountRemoved(_)
            | CoreEvent::MailStateChanged(_)
            | CoreEvent::Backfill(_) => {}
        }),
    });
    let token = WorkerToken {
        account_id: account.id.clone(),
        source: account.mailbox_source_id.clone(),
        is_ews: false,
        revision: account.revision,
        generation: 1,
    };
    shared.states.lock().unwrap().insert(
        account.id.clone(),
        AccountSyncState {
            account_id: account.id.clone(),
            mailbox_source_id: Some(account.mailbox_source_id.clone()),
            generation: 1,
            sequence: 0,
            folder_revision: 0,
            folders: vec![],
            phase: SyncPhase::Syncing,
            last_synced_at: None,
            failure: None,
            failure_since: None,
        },
    );
    Fixture {
        events,
        new_mail,
        token,
        _directory: directory,
        service,
        shared,
        account_id: account.id,
        source: account.mailbox_source_id,
    }
}

type Script = Vec<(String, String)>;

fn commands(validity: u32, uids: &[u32]) -> Script {
    let search = uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = vec![
        ("LOGIN ".into(), "{tag} OK authenticated\r\n".into()),
        (
            "CAPABILITY".into(),
            "* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n".into(),
        ),
        (
            "STATUS ".into(),
            format!(
                "* STATUS INBOX (MESSAGES {} UNSEEN 0 UIDNEXT 100 UIDVALIDITY {validity})\r\n{{tag}} OK status\r\n",
                uids.len()
            ),
        ),
        (
            "EXAMINE ".into(),
            format!(
                "* {} EXISTS\r\n* OK [UIDVALIDITY {validity}] generation\r\n* OK [UIDNEXT 100] next\r\n{{tag}} OK [READ-ONLY] examined\r\n",
                uids.len()
            ),
        ),
        (
            "UID SEARCH SINCE ".into(),
            format!("* SEARCH {search}\r\n{{tag}} OK searched\r\n"),
        ),
    ];
    if !uids.is_empty() {
        script.push((
            "UID SEARCH FLAGGED".into(),
            "* SEARCH\r\n{tag} OK searched\r\n".into(),
        ));
    }
    script
}

fn header(uid: u32, subject: &str) -> String {
    header_with(uid, subject, "", chrono::Utc::now())
}

fn header_with(
    uid: u32,
    subject: &str,
    flags: &str,
    received: chrono::DateTime<chrono::Utc>,
) -> String {
    let date = received.format("%d-%b-%Y %H:%M:%S +0000");
    let bytes = format!(
        "From: Sender <sender@example.com>\r\nSubject: {subject}\r\nTo: lin@example.com\r\n\r\n"
    );
    format!(
        "* 1 FETCH (UID {uid} FLAGS ({flags}) INTERNALDATE \"{date}\" RFC822.SIZE 200 BODY[HEADER.FIELDS (DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID CONTENT-TYPE CONTENT-DISPOSITION)] {{{}}}\r\n{bytes})\r\n",
        bytes.len()
    )
}

fn add_headers(script: &mut Script, headers: &[(u32, &str)]) {
    let response = headers
        .iter()
        .map(|(uid, subject)| header(*uid, subject))
        .collect::<String>();
    script.push(("UID FETCH ".into(), response + "{tag} OK fetched\r\n"));
}

async fn serve(stream: DuplexStream, script: Script) {
    let mut server = BufReader::new(stream);
    server
        .get_mut()
        .write_all(b"* OK synthetic IMAP\r\n")
        .await
        .unwrap();
    for (prefix, response) in script {
        let mut line = String::new();
        assert!(
            server.read_line(&mut line).await.unwrap() > 0,
            "missing command {prefix}"
        );
        let (tag, command) = line.trim_end().split_once(' ').unwrap();
        assert!(
            command.starts_with(&prefix),
            "expected {prefix}, got {command}"
        );
        assert!(!command.starts_with("SELECT") && !command.starts_with("STORE"));
        assert!(!command.contains(" BODY["), "header reads must use PEEK");
        server
            .get_mut()
            .write_all(response.replace("{tag}", tag).as_bytes())
            .await
            .unwrap();
    }
}

async fn run(fixture: &Fixture, script: Script, success: bool) -> Result<(), SyncError> {
    run_with(fixture, script, success, &mut FolderMemory::new())
        .await
        .0
}

/// Runs one folder round against `script`, returning the result and whether
/// visible data changed. `memory` carries STATUS snapshots between rounds.
async fn run_with(
    fixture: &Fixture,
    mut script: Script,
    success: bool,
    memory: &mut FolderMemory,
) -> (Result<(), SyncError>, bool) {
    if success {
        script.push(("LOGOUT".into(), "{tag} OK goodbye\r\n".into()));
    }
    let (client, server) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(server, script));
    let endpoint = fixture
        .service
        .lock()
        .unwrap()
        .get(&fixture.account_id)
        .unwrap()
        .connection
        .imap_smtp()
        .unwrap()
        .0
        .clone();
    let login = ImapLogin {
        host: endpoint.host,
        port: 993,
        tls: crate::accounts::types::TlsMode::Implicit,
        username: "lin".into(),
        credential: crate::imap::ProtocolCredential::Password(SecretValue::new(
            "synthetic-password".into(),
        )),
    };
    let mut session = imap::test_session(Box::new(client), &login).await.unwrap();
    let targets = fixture
        .service
        .lock()
        .unwrap()
        .sync_folders(&fixture.account_id, &fixture.source)
        .unwrap();
    fixture
        .shared
        .reconcile_targets(&fixture.token, &targets, false);
    let mut round = super::super::messages::FolderRound::default();
    let result = super::super::messages::sync_folder(
        &fixture.shared,
        &fixture.token,
        &targets[0].0,
        &targets[0].1,
        &mut session,
        memory,
        &mut round,
    )
    .await;
    let changed = round.changed;
    fixture
        .shared
        .announce(&fixture.token, round.arrivals)
        .await;
    if success {
        session.logout().await.unwrap();
    } else {
        drop(session);
    }
    server.await.unwrap();
    (result, changed)
}

#[tokio::test]
async fn initial_incremental_flags_deletion_and_generation_replacement() {
    let fixture = fixture();
    let mut first = commands(21, &[2, 3]);
    add_headers(&mut first, &[(2, "Old"), (3, "Kept")]);
    run(&fixture, first, true).await.unwrap();
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.coverage.headers_complete);
    assert!(page.items.iter().all(|mail| !mail.is_read));
    let kept_id = page
        .items
        .iter()
        .find(|mail| mail.subject == "Kept")
        .unwrap()
        .id
        .clone();

    let mut second = commands(21, &[3, 4]);
    add_headers(&mut second, &[(4, "New")]);
    second.push((
        "UID FETCH 3 (UID FLAGS)".into(),
        "* 1 FETCH (UID 3 FLAGS (\\Seen \\Flagged))\r\n{tag} OK flags\r\n".into(),
    ));
    run(&fixture, second, true).await.unwrap();
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(!page.items.iter().any(|mail| mail.subject == "Old"));
    let kept = page.items.iter().find(|mail| mail.id == kept_id).unwrap();
    assert!(kept.is_read && kept.is_starred);

    let mut third = commands(22, &[3]);
    add_headers(&mut third, &[(3, "New generation")]);
    run(&fixture, third, true).await.unwrap();
    let service = fixture.service.lock().unwrap();
    let page = service.list_emails(&EmailListQuery::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subject, "New generation");
    assert_ne!(page.items[0].id, kept_id);
    assert!(service.get_email(&kept_id).is_err());
}

fn fetched(headers: &[String]) -> (String, String) {
    (
        "UID FETCH ".into(),
        headers.concat() + "{tag} OK fetched\r\n",
    )
}

fn flags_of(uids: &[u32]) -> (String, String) {
    let lines: String = uids
        .iter()
        .map(|uid| format!("* 1 FETCH (UID {uid} FLAGS ())\r\n"))
        .collect();
    ("UID FETCH ".into(), lines + "{tag} OK flags\r\n")
}

fn arrivals(fixture: &Fixture) -> Vec<NewMail> {
    std::mem::take(&mut *fixture.new_mail.lock().unwrap())
}

#[tokio::test]
async fn only_unread_recent_inbox_mail_that_arrived_after_the_first_round_is_new() {
    let fixture = fixture();
    let now = chrono::Utc::now();
    // The first round finds what was already there: never new.
    let mut first = commands(21, &[2, 3]);
    add_headers(&mut first, &[(2, "Old"), (3, "Older")]);
    run(&fixture, first, true).await.unwrap();
    assert!(arrivals(&fixture).is_empty());

    // Then 4 (unread, just now), 5 (already read elsewhere) and 6 (sent days
    // ago, e.g. delivered late or found after being offline) arrive.
    let mut second = commands(21, &[2, 3, 4, 5, 6]);
    second.push(fetched(&[
        header_with(4, "Fresh", "", now),
        header_with(5, "Read on phone", "\\Seen", now),
        header_with(6, "Stale", "", now - chrono::Duration::days(3)),
    ]));
    second.push(flags_of(&[2, 3]));
    run(&fixture, second, true).await.unwrap();
    let found = arrivals(&fixture);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].account_id, fixture.account_id);
    assert_eq!(found[0].count, 1);
    assert_eq!(found[0].messages[0].subject, "Fresh");
    assert_eq!(found[0].messages[0].from.address, "sender@example.com");

    // Nothing new: nothing announced, even on a full round.
    let mut third = commands(21, &[2, 3, 4, 5, 6]);
    third.push(flags_of(&[2, 3, 4, 5, 6]));
    run(&fixture, third, true).await.unwrap();
    assert!(arrivals(&fixture).is_empty());
}

#[tokio::test]
async fn many_arrivals_are_one_event_with_every_candidate_newest_first() {
    let fixture = fixture();
    let now = chrono::Utc::now();
    let mut first = commands(21, &[2]);
    add_headers(&mut first, &[(2, "Old")]);
    run(&fixture, first, true).await.unwrap();

    let mut second = commands(21, &[2, 3, 4, 5, 6, 7]);
    second.push(fetched(
        &(3..=7)
            .map(|uid| {
                header_with(
                    uid,
                    &format!("M{uid}"),
                    "",
                    now - chrono::Duration::minutes(10 - i64::from(uid)),
                )
            })
            .collect::<Vec<_>>(),
    ));
    second.push(flags_of(&[2]));
    run(&fixture, second, true).await.unwrap();
    let found = arrivals(&fixture);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].count, 5);
    let subjects: Vec<_> = found[0]
        .messages
        .iter()
        .map(|m| m.subject.as_str())
        .collect();
    assert_eq!(subjects, ["M7", "M6", "M5", "M4", "M3"]);
}

#[tokio::test]
async fn a_rebuilt_folder_is_not_new_mail_but_an_empty_first_inbox_can_receive_some() {
    let fixture = fixture();
    // An empty inbox completes its first round with nothing stored...
    run(&fixture, commands(21, &[]), true).await.unwrap();
    // ...so the first message after that is new.
    let mut second = commands(21, &[1]);
    add_headers(&mut second, &[(1, "Welcome")]);
    run(&fixture, second, true).await.unwrap();
    assert_eq!(arrivals(&fixture)[0].messages[0].subject, "Welcome");

    // The server rebuilt the folder (new UIDVALIDITY): everything in it is
    // re-downloaded, none of it is new mail.
    let mut rebuilt = commands(22, &[1, 2]);
    add_headers(&mut rebuilt, &[(1, "Welcome"), (2, "Other")]);
    run(&fixture, rebuilt, true).await.unwrap();
    assert!(arrivals(&fixture).is_empty());
}

#[tokio::test]
async fn new_mail_outside_the_inbox_is_not_announced() {
    let fixture = fixture_with(config(json!({"kind":"direct"})), FolderRole::Junk);
    let mut first = commands(21, &[2]);
    add_headers(&mut first, &[(2, "Old")]);
    run(&fixture, first, true).await.unwrap();
    let mut second = commands(21, &[2, 3]);
    add_headers(&mut second, &[(3, "Spam")]);
    second.push(flags_of(&[2]));
    run(&fixture, second, true).await.unwrap();
    assert!(arrivals(&fixture).is_empty());
}

#[tokio::test]
async fn search_failure_does_not_delete_cached_messages() {
    let fixture = fixture();
    let mut first = commands(21, &[2]);
    add_headers(&mut first, &[(2, "Preserved")]);
    run(&fixture, first, true).await.unwrap();
    let mut failed = commands(21, &[]);
    failed.last_mut().unwrap().1 = "{tag} NO search unavailable\r\n".into();
    assert!(run(&fixture, failed, false).await.is_err());
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subject, "Preserved");
}

#[tokio::test]
async fn search_without_enumeration_completes_for_an_explicitly_empty_folder() {
    let fixture = fixture();
    let mut script = commands(21, &[]);
    script.last_mut().unwrap().1 = "{tag} OK searched\r\n".into();
    run(&fixture, script, true).await.unwrap();

    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.coverage.headers_complete);
}

#[tokio::test]
async fn search_without_enumeration_preserves_cache_without_a_confirmed_empty_folder() {
    let fixture = fixture();
    let mut first = commands(21, &[2]);
    add_headers(&mut first, &[(2, "Preserved")]);
    run(&fixture, first, true).await.unwrap();

    for (examine_count, search_response) in [
        ("", "{tag} OK searched\r\n"),
        ("* 1 EXISTS\r\n", "{tag} OK searched\r\n"),
        ("* 0 EXISTS\r\n", "* 1 EXISTS\r\n{tag} OK searched\r\n"),
        (
            "* 0 EXISTS\r\n",
            "* OK [UIDVALIDITY 22] generation\r\n{tag} OK searched\r\n",
        ),
        ("* 0 EXISTS\r\n", "{tag} OK [UIDVALIDITY 22] searched\r\n"),
    ] {
        // A STATUS count of zero is insufficient without the current selection.
        let mut script = commands(21, &[]);
        script[3].1 = format!(
            "{examine_count}* OK [UIDVALIDITY 21] generation\r\n* OK [UIDNEXT 100] next\r\n{{tag}} OK [READ-ONLY] examined\r\n"
        );
        script.last_mut().unwrap().1 = search_response.into();
        assert!(run(&fixture, script, false).await.is_err());

        let page = fixture
            .service
            .lock()
            .unwrap()
            .list_emails(&EmailListQuery::default())
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].subject, "Preserved");
    }
}

#[tokio::test]
async fn rejected_partial_fetch_is_not_committed_and_retries_cleanly() {
    let fixture = fixture();
    let mut failed = commands(21, &[2, 3]);
    failed.push((
        "UID FETCH ".into(),
        header(2, "Not committed") + "{tag} NO partial fetch failed\r\n",
    ));
    assert!(run(&fixture, failed, false).await.is_err());
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert!(page.items.is_empty());
    assert!(!page.coverage.headers_complete);
    let mut retry = commands(21, &[2, 3]);
    add_headers(&mut retry, &[(2, "Recovered 2"), (3, "Recovered 3")]);
    run(&fixture, retry, true).await.unwrap();
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.coverage.headers_complete);
}

#[tokio::test]
async fn vanished_uid_between_search_and_fetch_keeps_coverage_incomplete() {
    let fixture = fixture();
    let mut first = commands(21, &[2, 3]);
    add_headers(&mut first, &[(3, "Survivor")]);
    assert!(run(&fixture, first, false).await.is_err());
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(!page.coverage.headers_complete);
    let mut retry = commands(21, &[3]);
    retry.push((
        "UID FETCH 3 (UID FLAGS)".into(),
        "* 1 FETCH (UID 3 FLAGS ())\r\n{tag} OK flags\r\n".into(),
    ));
    run(&fixture, retry, true).await.unwrap();
    assert!(
        fixture
            .service
            .lock()
            .unwrap()
            .list_emails(&EmailListQuery::default())
            .unwrap()
            .coverage
            .headers_complete
    );
}

#[tokio::test]
async fn short_flag_fetch_retries_live_uids_and_removes_confirmed_missing_uids() {
    let fixture = fixture();
    let mut first = commands(21, &[2, 3, 4]);
    add_headers(
        &mut first,
        &[(2, "Removed"), (3, "Recovered"), (4, "Returned")],
    );
    run(&fixture, first, true).await.unwrap();

    let mut second = commands(21, &[2, 3, 4]);
    second.extend([
        (
            "UID FETCH 2:4 (UID FLAGS)".into(),
            "* 1 FETCH (UID 4 FLAGS (\\Seen))\r\n{tag} OK flags\r\n".into(),
        ),
        (
            "UID SEARCH UID 2:3".into(),
            "* SEARCH 3\r\n{tag} OK searched\r\n".into(),
        ),
        (
            "UID FETCH 3 (UID FLAGS)".into(),
            "* 2 FETCH (UID 3 FLAGS (\\Seen))\r\n{tag} OK flags\r\n".into(),
        ),
    ]);
    run(&fixture, second, true).await.unwrap();

    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert!(page.coverage.headers_complete);
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().all(|mail| mail.is_read));
    assert!(page.items.iter().any(|mail| mail.subject == "Recovered"));
    assert!(page.items.iter().any(|mail| mail.subject == "Returned"));
}

#[tokio::test]
async fn unresolved_flags_keep_last_known_state_and_the_folder_still_completes() {
    let fixture = fixture();
    let uids: Vec<u32> = (1..=501).rev().collect();
    let mut first = commands(21, &uids);
    let now = chrono::Utc::now();
    for batch in uids.chunks(200) {
        first.push(fetched(
            &batch
                .iter()
                .map(|&uid| match uid {
                    1 => header_with(uid, "Later batch", "", now),
                    501 => header_with(uid, "Unresolved", "\\Seen", now),
                    _ => header_with(uid, "Other", "", now - chrono::Duration::minutes(1)),
                })
                .collect::<Vec<_>>(),
        ));
    }
    run(&fixture, first, true).await.unwrap();

    let mut second = commands(21, &uids);
    let live: Vec<_> = (476..=501).map(|uid| uid.to_string()).collect();
    second.extend([
        (
            "UID FETCH 2:501 (UID FLAGS)".into(),
            flags_of(&(2..=475).collect::<Vec<_>>()).1,
        ),
        (
            "UID SEARCH UID 476:501".into(),
            format!("* SEARCH {}\r\n{{tag}} OK searched\r\n", live.join(" ")),
        ),
        (
            "UID FETCH 476:500 (UID FLAGS)".into(),
            flags_of(&(476..=500).collect::<Vec<_>>()).1,
        ),
        (
            "UID FETCH 501 (UID FLAGS)".into(),
            "{tag} OK omitted\r\n".into(),
        ),
        (
            "UID FETCH 1 (UID FLAGS)".into(),
            "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n{tag} OK flags\r\n".into(),
        ),
    ]);
    // A folder the server will not fully answer does not fail the whole round
    // (2925 answers every flags fetch with nothing): it completes, and the mails
    // whose flags are unknown keep their last known state.
    run(&fixture, second, false).await.unwrap();
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert!(page.coverage.headers_complete);
    for subject in ["Unresolved", "Later batch"] {
        let mail = page
            .items
            .iter()
            .find(|mail| mail.subject == subject)
            .unwrap();
        assert!(mail.is_read, "{subject}");
    }
}

#[tokio::test]
async fn unsuccessful_targeted_search_preserves_cached_messages_and_flags() {
    for response in [
        "{tag} NO search unavailable\r\n",
        "{tag} OK no enumeration\r\n",
        "* SEARCH 5\r\n{tag} OK searched\r\n",
    ] {
        let fixture = fixture();
        let mut first = commands(21, &[2]);
        first.push(fetched(&[header_with(
            2,
            "Preserved",
            "\\Seen",
            chrono::Utc::now(),
        )]));
        run(&fixture, first, true).await.unwrap();

        let mut second = commands(21, &[2]);
        second.extend([
            (
                "UID FETCH 2 (UID FLAGS)".into(),
                "* 1 FETCH (FLAGS ())\r\n{tag} OK omitted\r\n".into(),
            ),
            ("UID SEARCH UID 2".into(), response.into()),
        ]);
        assert!(run(&fixture, second, false).await.is_err());

        let page = fixture
            .service
            .lock()
            .unwrap()
            .list_emails(&EmailListQuery::default())
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].subject, "Preserved");
        assert!(page.items[0].is_read);
        assert!(!page.coverage.headers_complete);
    }
}

fn replace_targets(fixture: &Fixture, names: &[(&str, bool)]) -> Vec<(String, String)> {
    let mut service = fixture.service.lock().unwrap();
    let records: Vec<_> = names
        .iter()
        .map(|(name, selectable)| FolderRecord {
            remote_name: (*name).into(),
            parent_remote_name: None,
            name: (*name).into(),
            delimiter: Some("/".into()),
            role: if *name == "INBOX" {
                FolderRole::Inbox
            } else {
                FolderRole::Other
            },
            selectable: *selectable,
            attributes: vec![],
            status: None,
        })
        .collect();
    service
        .store_folders(&fixture.account_id, &fixture.source, &records)
        .unwrap();
    let targets = service
        .sync_folders(&fixture.account_id, &fixture.source)
        .unwrap();
    fixture
        .shared
        .reconcile_targets(&fixture.token, &targets, true);
    targets
}

fn authenticate_commands() -> Script {
    commands(1, &[]).into_iter().take(2).collect()
}

fn folder_commands(name: &str, validity: u32, uids: &[u32]) -> Script {
    let quoted = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
    let mut script: Script = commands(validity, uids).into_iter().skip(2).collect();
    script[0].0 = format!("STATUS {quoted} ");
    script[0].1 = script[0]
        .1
        .replace("STATUS INBOX ", &format!("STATUS {quoted} "));
    script[1].0 = format!("EXAMINE {quoted}");
    script
}

async fn scripted_session(script: Script) -> (Session, JoinHandle<()>) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(server, script));
    let login = ImapLogin {
        host: "imap.example.com".into(),
        port: 993,
        tls: crate::accounts::types::TlsMode::Implicit,
        username: "lin".into(),
        credential: crate::imap::ProtocolCredential::Password(SecretValue::new(
            "synthetic-password".into(),
        )),
    };
    let session = imap::test_session(Box::new(client), &login).await.unwrap();
    (session, server)
}

#[tokio::test]
async fn list_only_discovery_reaches_all_selectable_roles_and_skips_containers() {
    let fixture = fixture();
    // Start with no directory cache: LIST alone must establish selectable
    // targets, while their counts remain unknown until the individual STATUS.
    fixture
        .service
        .lock()
        .unwrap()
        .store_folders(&fixture.account_id, &fixture.source, &[])
        .unwrap();
    let names = [
        "INBOX",
        "Archive",
        "Drafts",
        "Empty",
        "Junk",
        "Projects/&XfJT0ZAB-",
        "Sent Items",
        "Trash",
    ];
    let mut script = authenticate_commands();
    script.push((
        "LIST \"\" *".into(),
        concat!(
            "* LIST () \"/\" INBOX\r\n",
            "* LIST (\\Noselect \\Sent) \"/\" Legacy\r\n",
            "* LIST (\\Sent) \"/\" \"Sent Items\"\r\n",
            "* LIST (\\Drafts) \"/\" Drafts\r\n",
            "* LIST (\\Archive) \"/\" Archive\r\n",
            "* LIST (\\Junk) \"/\" Junk\r\n",
            "* LIST (\\Trash) \"/\" Trash\r\n",
            "* LIST (\\Noselect \\HasChildren) \"/\" Projects\r\n",
            "* LIST () \"/\" \"Projects/&XfJT0ZAB-\"\r\n",
            "* LIST () \"/\" Empty\r\n",
            "* LIST (\\NonExistent) \"/\" Missing\r\n",
            "{tag} OK complete listing\r\n",
        )
        .into(),
    ));
    for name in names {
        let uids: &[u32] = if name == "Empty" { &[] } else { &[7] };
        script.extend(folder_commands(name, 21, uids));
        if !uids.is_empty() {
            add_headers(&mut script, &[(7, name)]);
        }
    }
    script.push(("LOGOUT".into(), "{tag} OK\r\n".into()));
    let (mut session, server) = scripted_session(script).await;
    let remote = session.mail_folders().await.unwrap();
    assert_eq!(remote.len(), 11);
    assert!(remote.iter().all(|folder| folder.status.is_none()));
    let records = folders::interpret(&remote);
    let targets = {
        let mut service = fixture.service.lock().unwrap();
        assert!(
            service
                .store_folders(&fixture.account_id, &fixture.source, &records)
                .unwrap()
        );
        let stored = service.mail_folders(&fixture.account_id).unwrap();
        assert_eq!(stored.iter().filter(|folder| folder.selectable).count(), 8);
        assert!(stored.iter().all(|folder| !folder.counts_known));
        for role in [
            FolderRole::Inbox,
            FolderRole::Sent,
            FolderRole::Drafts,
            FolderRole::Archive,
            FolderRole::Junk,
            FolderRole::Trash,
        ] {
            assert_eq!(
                stored
                    .iter()
                    .filter(|folder| folder.role == role && folder.selectable)
                    .count(),
                1
            );
        }
        let parent = stored
            .iter()
            .find(|folder| folder.name == "Projects")
            .unwrap();
        let child = stored
            .iter()
            .find(|folder| folder.name == "已发送")
            .unwrap();
        assert!(!parent.selectable);
        assert!(child.selectable);
        assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(child.role, FolderRole::Other);
        assert!(
            !service
                .list_emails(&EmailListQuery::default())
                .unwrap()
                .coverage
                .headers_complete
        );
        service
            .sync_folders(&fixture.account_id, &fixture.source)
            .unwrap()
    };
    assert_eq!(
        targets
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>(),
        names
    );
    fixture
        .shared
        .reconcile_targets(&fixture.token, &targets, true);
    assert!(matches!(
        sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut FolderMemory::new(),
            None,
            || async { Err(ImapError::Timeout) }
        )
        .await,
        Outcome::Synced
    ));
    server.await.unwrap();

    let service = fixture.service.lock().unwrap();
    let stored = service.mail_folders(&fixture.account_id).unwrap();
    for folder in &stored {
        assert_eq!(folder.counts_known, folder.selectable);
    }
    let page = service.list_emails(&EmailListQuery::default()).unwrap();
    assert_eq!(page.items.len(), 7);
    assert!(page.coverage.headers_complete);
    for (folder_id, name) in &targets {
        let page = service
            .list_emails(&EmailListQuery {
                folder_id: Some(folder_id.clone()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), usize::from(name != "Empty"));
        assert!(page.coverage.headers_complete);
    }
    let inbox = service
        .list_emails(&EmailListQuery {
            folder_role: Some(FolderRole::Inbox),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].subject, "INBOX");
    let states = fixture.shared.states.lock().unwrap();
    let folders = &states[&fixture.account_id].folders;
    assert_eq!(folders.len(), 8);
    assert!(
        folders
            .iter()
            .all(|folder| folder.phase == FolderSyncPhase::Idle)
    );
}

#[tokio::test]
async fn every_completed_folder_rejection_continues_on_the_same_connection() {
    // STATUS, EXAMINE, SEARCH, header FETCH and flags FETCH each reject a
    // complete command; the second folder must still succeed without reconnect.
    for rejected_step in 0..5 {
        let fixture = fixture();
        if rejected_step == 4 {
            let mut existing = commands(21, &[2]);
            add_headers(&mut existing, &[(2, "Cached")]);
            run(&fixture, existing, true).await.unwrap();
        }
        let targets = replace_targets(
            &fixture,
            &[("Later", true), ("INBOX", true), ("Container", false)],
        );
        assert_eq!(targets[0].1, "INBOX");
        let mut script = authenticate_commands();
        let mut inbox = folder_commands("INBOX", 21, &[2]);
        if rejected_step >= 3 {
            inbox.push((
                "UID FETCH ".into(),
                header(2, "Must not commit") + "{tag} OK\r\n",
            ));
        }
        let index = if rejected_step >= 3 { 4 } else { rejected_step };
        inbox.truncate(index + 1);
        inbox[index].1 = if rejected_step >= 3 {
            header(2, "Must not commit") + "{tag} NO private rejected contents\r\n"
        } else {
            "{tag} NO private rejected contents\r\n".into()
        };
        script.extend(inbox);
        script.extend(folder_commands("Later", 21, &[2]));
        add_headers(&mut script, &[(2, "Later mail")]);
        script.push(("LOGOUT".into(), "{tag} OK\r\n".into()));
        let (session, server) = scripted_session(script).await;
        let result = sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut FolderMemory::new(),
            None,
            || async {
                panic!("completed rejection must not reconnect");
                #[allow(unreachable_code)]
                Err(ImapError::Timeout)
            },
        )
        .await;
        assert!(matches!(result, Outcome::Partial(_)));
        server.await.unwrap();
        let page = fixture
            .service
            .lock()
            .unwrap()
            .list_emails(&EmailListQuery::default())
            .unwrap();
        assert!(page.items.iter().any(|mail| mail.subject == "Later mail"));
        assert!(
            !page
                .items
                .iter()
                .any(|mail| mail.subject == "Must not commit")
        );
        let states = fixture.shared.states.lock().unwrap();
        let state = &states[&fixture.account_id];
        assert_eq!(state.folders.len(), 2);
        assert_eq!(state.folders[0].phase, FolderSyncPhase::Failed);
        assert_eq!(state.folders[1].phase, FolderSyncPhase::Idle);
        assert!(
            !state.folders[0]
                .failure
                .as_ref()
                .unwrap()
                .detail
                .as_ref()
                .unwrap()
                .contains("private")
        );
        assert!(state.folders.iter().all(|folder| folder.data_revision > 0));
    }
}

#[tokio::test(start_paused = true)]
async fn broken_first_folder_reconnects_with_backoff_and_resumes_after_it() {
    let fixture = fixture();
    let targets = replace_targets(&fixture, &[("INBOX", true), ("Later", true)]);
    // Repeat a complete round: INBOX always breaks, but Later is never starved.
    for _ in 0..2 {
        let mut first = authenticate_commands();
        first.push((
            "STATUS \"INBOX\" ".into(),
            "* BYE private disconnect\r\n".into(),
        ));
        let (session, server) = scripted_session(first).await;
        let mut resumed = authenticate_commands();
        resumed.extend(folder_commands("Later", 21, &[]));
        resumed.push(("LOGOUT".into(), "{tag} OK\r\n".into()));
        let mut remaining = Some(resumed);
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let recorded = attempts.clone();
        let start = Instant::now();
        let resume_server = Arc::new(Mutex::new(None));
        let result = sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut FolderMemory::new(),
            None,
            || {
                let mut attempts = recorded.lock().unwrap();
                attempts.push(Instant::now());
                let first_failure = attempts.len() == 1;
                let script = if first_failure {
                    None
                } else {
                    remaining.take()
                };
                let resume_server = resume_server.clone();
                async move {
                    if let Some(script) = script {
                        let (session, server) = scripted_session(script).await;
                        // The script must reach LOGOUT; await it once the round exits.
                        *resume_server.lock().unwrap() = Some(server);
                        Ok(session)
                    } else {
                        Err(ImapError::Net(NetError::Connect("refused".into())))
                    }
                }
            },
        )
        .await;
        assert!(matches!(result, Outcome::Partial(_)));
        server.await.unwrap();
        let server = resume_server.lock().unwrap().take().unwrap();
        server.await.unwrap();
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 2);
        assert!(attempts[0].duration_since(start) >= Duration::from_secs(1));
        assert!(attempts[1].duration_since(attempts[0]) >= Duration::from_secs(2));
        assert_eq!(
            fixture.shared.states.lock().unwrap()[&fixture.account_id].folders[1].phase,
            FolderSyncPhase::Idle
        );
    }
}

#[tokio::test]
async fn no_inbox_nested_and_escaped_names_sync_independently_and_notify_per_folder() {
    let fixture = fixture();
    let targets = replace_targets(
        &fixture,
        &[
            ("&XfJT0ZAB-/East \"team\"", true),
            ("Empty", true),
            ("Parent", false),
        ],
    );
    let mut script = authenticate_commands();
    for (_, name) in &targets {
        let uids = if name == "Empty" { vec![] } else { vec![2] };
        script.extend(folder_commands(name, 21, &uids));
        if !uids.is_empty() {
            add_headers(&mut script, &[(2, "Nested")]);
        }
    }
    script.push(("LOGOUT".into(), "{tag} OK\r\n".into()));
    let (session, server) = scripted_session(script).await;
    assert!(matches!(
        sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut FolderMemory::new(),
            None,
            || async { Err(ImapError::Timeout) }
        )
        .await,
        Outcome::Synced
    ));
    server.await.unwrap();
    let events = fixture.events.lock().unwrap();
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(
        events
            .iter()
            .any(|state| state.folders[0].phase == FolderSyncPhase::Idle
                && state.folders[1].phase == FolderSyncPhase::Waiting)
    );
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.coverage.headers_complete);
}

#[tokio::test]
async fn queued_old_worker_cannot_write_after_generation_changes() {
    let fixture = fixture();
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let written = writes.clone();
    // Poll once to enqueue spawn_blocking while the service lock is unavailable.
    let service_guard = fixture.service.lock().unwrap();
    let mut old_write = Box::pin(with_store(&fixture.shared, &fixture.token, move |_| {
        written.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }));
    assert!(
        old_write
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    fixture
        .shared
        .states
        .lock()
        .unwrap()
        .get_mut(&fixture.account_id)
        .unwrap()
        .generation += 1;
    drop(service_guard);
    assert!(matches!(old_write.await, Err(SyncError::Stale)));
    assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
    let before = fixture.events.lock().unwrap().len();
    fixture.shared.update(&fixture.token, |state| {
        state.failure = Some(failure(SyncFailureCode::Timeout, None, true))
    });
    assert_eq!(fixture.events.lock().unwrap().len(), before);
}

#[test]
fn successful_listing_rebuilds_runtime_states_without_resurrecting_removed_folders() {
    let fixture = fixture();
    let old = replace_targets(
        &fixture,
        &[
            ("INBOX", true),
            ("Removed", true),
            ("Parent", true),
            ("Parent/Child", true),
        ],
    );
    let removed_id = old
        .iter()
        .find(|(_, name)| name == "Removed")
        .unwrap()
        .0
        .clone();
    let parent_id = old
        .iter()
        .find(|(_, name)| name == "Parent")
        .unwrap()
        .0
        .clone();
    let child_id = old
        .iter()
        .find(|(_, name)| name == "Parent/Child")
        .unwrap()
        .0
        .clone();
    fixture
        .shared
        .folder_update(&fixture.token, &child_id, |folder| {
            folder.phase = FolderSyncPhase::Idle;
            folder.data_revision = 7;
        });
    let targets = replace_targets(
        &fixture,
        &[
            ("INBOX", true),
            ("Renamed", true),
            ("Parent", false),
            ("Parent/Child", true),
        ],
    );
    fixture
        .shared
        .folder_update(&fixture.token, &removed_id, |folder| {
            folder.phase = FolderSyncPhase::Idle
        });
    let states = fixture.shared.states.lock().unwrap();
    let state = &states[&fixture.account_id];
    assert_eq!(state.folder_revision, 2);
    assert!(
        !state
            .folders
            .iter()
            .any(|folder| folder.folder_id == removed_id || folder.folder_id == parent_id)
    );
    assert_eq!(
        state
            .folders
            .iter()
            .find(|folder| folder.folder_id == child_id)
            .unwrap()
            .data_revision,
        7
    );
    let renamed_id = &targets
        .iter()
        .find(|(_, name)| name == "Renamed")
        .unwrap()
        .0;
    assert_eq!(
        state
            .folders
            .iter()
            .find(|folder| &folder.folder_id == renamed_id)
            .unwrap()
            .phase,
        FolderSyncPhase::Waiting
    );
}

#[tokio::test]
async fn new_round_connection_and_credential_failures_do_not_keep_previous_idle_folders() {
    for blocked in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut account_config = config(json!({"kind":"direct"}));
        account_config["imap"]["host"] = json!("127.0.0.1");
        account_config["imap"]["port"] = json!(port);
        let mut fixture = fixture_with_config(account_config.clone());
        let targets = replace_targets(&fixture, &[("INBOX", true), ("Later", true)]);
        let synced_at = "2026-10-19T00:00:00.000Z";
        for (folder_id, _) in &targets {
            fixture
                .shared
                .folder_update(&fixture.token, folder_id, |folder| {
                    folder.phase = FolderSyncPhase::Idle;
                    folder.last_synced_at = Some(synced_at.into());
                    folder.data_revision = 7;
                });
        }
        if blocked {
            // TLS changes retain mailbox identity but revoke credential trust.
            account_config["imap"]["tls"] = json!("startTls");
            account_config["authentication"]["password"] = json!({"action":"keep"});
            let account = fixture
                .service
                .lock()
                .unwrap()
                .update_connection(connection_update(
                    &fixture.account_id,
                    fixture.token.revision,
                    account_config.clone(),
                ))
                .unwrap();
            assert_eq!(account.mailbox_source_id, fixture.source);
            fixture.token.revision = account.revision;
        }
        fixture.events.lock().unwrap().clear();
        // The sender stays alive for the whole test: without it the worker stops.
        let (_activity, activity) = tokio::sync::watch::channel(Activity::Active);
        let worker = tokio::spawn(super::run(
            fixture.shared.clone(),
            fixture.token.clone(),
            Default::default(),
            activity,
            crate::sync::idle::IdleHealth::watch_closed(),
        ));
        let failed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = fixture.shared.states.lock().unwrap()[&fixture.account_id].clone();
                if state.phase == SyncPhase::Failed {
                    break state;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        worker.abort();
        let _ = worker.await;
        assert_eq!(
            failed.failure.unwrap().code,
            if blocked {
                SyncFailureCode::CredentialUntrusted
            } else {
                SyncFailureCode::Network
            }
        );
        assert_eq!(failed.folders.len(), 2);
        for folder in &failed.folders {
            assert_eq!(folder.phase, FolderSyncPhase::Waiting);
            assert_eq!(folder.last_synced_at.as_deref(), Some(synced_at));
            assert_eq!(folder.data_revision, 7);
            assert!(folder.failure.is_none());
        }
        let events = fixture.events.lock().unwrap();
        assert!(events.iter().any(|state| {
            state.phase == SyncPhase::Syncing
                && state
                    .folders
                    .iter()
                    .all(|folder| folder.phase == FolderSyncPhase::Waiting)
        }));
        assert!(!events.iter().any(|state| {
            state
                .folders
                .iter()
                .any(|folder| folder.phase == FolderSyncPhase::Idle)
        }));
    }
}

#[tokio::test]
async fn lossy_status_name_still_syncs_headers_and_preserves_the_status_fast_path() {
    let fixture = fixture();
    let mut memory = FolderMemory::new();
    let mut first = commands(21, &[2, 3]);
    first[2].1 =
        "* STATUS \"???\" (MESSAGES 999 UNSEEN 888 UIDNEXT 999 UIDVALIDITY 999)\r\n{tag} OK\r\n"
            .into();
    first.insert(
        4,
        (
            "UID SEARCH UNSEEN".into(),
            "* SEARCH 2\r\n{tag} OK\r\n".into(),
        ),
    );
    add_headers(&mut first, &[(2, "Unread"), (3, "Other")]);
    let (result, changed) = run_with(&fixture, first.clone(), true, &mut memory).await;
    result.unwrap();
    assert!(changed);
    {
        let service = fixture.service.lock().unwrap();
        let folders = service.mail_folders(&fixture.account_id).unwrap();
        assert!(folders[0].counts_known);
        assert_eq!((folders[0].total_count, folders[0].unread_count), (2, 1));
        let page = service.list_emails(&EmailListQuery::default()).unwrap();
        assert_eq!(page.items.len(), 2);
        let folder = &service
            .sync_folders(&fixture.account_id, &fixture.source)
            .unwrap()[0]
            .0;
        let target = crate::accounts::messages::SyncTarget {
            account_id: fixture.account_id.clone(),
            mailbox_source_id: fixture.source.clone(),
            folder_id: folder.clone(),
            uid_validity: 21,
        };
        assert_eq!(service.cached_uids(&target).unwrap().len(), 2);
    }
    // The fallback already opened the folder. No second EXAMINE is needed,
    // and an unchanged round still skips the window search and flag fetches.
    first.truncate(5);
    let (result, changed) = run_with(&fixture, first, true, &mut memory).await;
    result.unwrap();
    assert!(!changed);
}

#[tokio::test]
async fn failed_status_fallback_preserves_counts_and_cached_headers() {
    let fixture = fixture();
    let mut first = commands(21, &[2, 3]);
    add_headers(&mut first, &[(2, "Old"), (3, "Kept")]);
    run(&fixture, first, true).await.unwrap();

    let mut failed = commands(21, &[]);
    failed[2].1 = "{tag} OK\r\n".into();
    failed.truncate(4);
    failed.push((
        "UID SEARCH UNSEEN".into(),
        "{tag} NO search rejected\r\n".into(),
    ));
    assert!(run(&fixture, failed, false).await.is_err());
    let service = fixture.service.lock().unwrap();
    let folders = service.mail_folders(&fixture.account_id).unwrap();
    assert_eq!(
        folders[0].total_count, 2,
        "failed fallback is not an empty folder"
    );
    assert_eq!(
        service
            .list_emails(&EmailListQuery::default())
            .unwrap()
            .items
            .len(),
        2
    );
}

#[tokio::test]
async fn an_unchanged_folder_skips_the_full_round_and_reports_no_change() {
    let fixture = fixture();
    let mut memory = FolderMemory::new();
    let mut first = commands(21, &[2, 3]);
    add_headers(&mut first, &[(2, "Old"), (3, "Kept")]);
    let (result, changed) = run_with(&fixture, first, true, &mut memory).await;
    result.unwrap();
    assert!(changed, "the first round stores messages");

    // Same STATUS: the server script ends after STATUS, so an EXAMINE, SEARCH
    // or FETCH from the client would fail the scripted server.
    let unchanged = commands(21, &[2, 3]).into_iter().take(3).collect();
    let (result, changed) = run_with(&fixture, unchanged, true, &mut memory).await;
    result.unwrap();
    assert!(!changed, "nothing changed, so readers need not refetch");
}

fn qq_sent_fixture() -> Fixture {
    let mut account = config(json!({"kind":"direct"}));
    account["imap"]["host"] = json!("imap.qq.com");
    account["emailAddress"] = json!("lin@vip.qq.com");
    fixture_with(account, FolderRole::Sent)
}

fn accepted_sent_copy(fixture: &Fixture) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let raw = b"From: lin@vip.qq.com\r\nTo: friend@example.com\r\nSubject: Sent copy\r\nMessage-ID: <original@vip.qq.com>\r\n\r\nbody";
    let snapshot = crate::storage::bodies::BlobStore::open(fixture._directory.path())
        .unwrap()
        .put(raw)
        .unwrap();
    let db = rusqlite::Connection::open(fixture._directory.path().join("mailx.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO outbox(id, account_id, client_mutation_id, queue_canonical, message_id,
         snapshot_hash, snapshot_size, envelope, subject, recipients_preview, recipient_count,
         state, accepted_at, not_before, first_queued_at, created_at, updated_at, preview,
         attachment_count)
         VALUES (?1, ?2, ?3, '{}', '<original@vip.qq.com>', ?4, ?5,
         '{\"from\":\"lin@vip.qq.com\",\"recipients\":[\"friend@example.com\"]}',
         'Sent copy', '[\"friend@example.com\"]', 1, 'accepted', ?6, ?6, ?6, ?6, ?6, 'body', 0)",
        rusqlite::params![
            id,
            fixture.account_id,
            uuid::Uuid::new_v4().to_string(),
            snapshot.sha256,
            snapshot.size as i64,
            timestamp
        ],
    )
    .unwrap();
    id
}

fn add_qq_sent_header(script: &mut Script, include_original: bool) {
    let fields = "DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID CONTENT-TYPE CONTENT-DISPOSITION IN-REPLY-TO REFERENCES X-OQ-MSGID";
    let original = if include_original {
        "X-OQ-MSGID: <original@vip.qq.com>\r\n"
    } else {
        ""
    };
    let header = format!(
        "From: lin@vip.qq.com\r\nTo: friend@example.com\r\nSubject: Sent copy\r\nMessage-ID: <rewritten@qq.com>\r\n{original}\r\n"
    );
    let date = chrono::Utc::now()
        .format("%d-%b-%Y %H:%M:%S +0000")
        .to_string();
    script.push((
        format!("UID FETCH 2 (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS ({fields})])"),
        format!("* 1 FETCH (UID 2 FLAGS (\\Seen) INTERNALDATE \"{date}\" RFC822.SIZE 200 BODY[HEADER.FIELDS ({fields})] {{{}}}\r\n{header})\r\n{{tag}} OK fetched\r\n", header.len()),
    ));
}

fn sent_page(fixture: &Fixture) -> crate::mail::EmailPage {
    fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery {
            account_ids: Some(vec![fixture.account_id.clone()]),
            folder_role: Some(FolderRole::Sent),
            ..Default::default()
        })
        .unwrap()
}

#[tokio::test]
async fn cached_sent_identity_refreshes_despite_unchanged_status_then_stops() {
    let fixture = qq_sent_fixture();
    let outbox = accepted_sent_copy(&fixture);
    let mut memory = FolderMemory::new();
    let mut first = commands(21, &[2]);
    add_qq_sent_header(&mut first, false);
    run_with(&fixture, first, true, &mut memory)
        .await
        .0
        .unwrap();
    assert_eq!(sent_page(&fixture).items.len(), 2);

    // A migrated cache has the standard Message-ID and an empty policy stamp.
    // Keep the round's STATUS memory unchanged to exercise the skip guard.
    let db = rusqlite::Connection::open(fixture._directory.path().join("mailx.sqlite3")).unwrap();
    db.execute("UPDATE messages SET sent_identity_policy = ''", [])
        .unwrap();
    let before: String = db
        .query_row("SELECT id FROM messages WHERE uid = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut repair = commands(21, &[2]);
    add_qq_sent_header(&mut repair, true);
    repair.push((
        "UID FETCH 2 (UID FLAGS)".into(),
        "* 1 FETCH (UID 2 FLAGS (\\Seen))\r\n{tag} OK flags\r\n".into(),
    ));
    let (result, changed) = run_with(&fixture, repair, true, &mut memory).await;
    result.unwrap();
    assert!(changed, "matching removes the duplicate local row");
    let page = sent_page(&fixture);
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].id, before,
        "repair preserves the cached server row"
    );
    assert!(!page.items[0].local_only);
    assert!(
        fixture
            .service
            .lock()
            .unwrap()
            .outbox_read(&outbox)
            .unwrap()
            .server_copy_message_id
            .is_some()
    );

    // No further EXAMINE, SEARCH, identity FETCH or flags FETCH is expected.
    let unchanged = commands(21, &[2]).into_iter().take(3).collect();
    let (result, changed) = run_with(&fixture, unchanged, true, &mut memory).await;
    result.unwrap();
    assert!(!changed);
}

#[tokio::test]
async fn new_sent_header_uses_provider_identity_without_a_second_fetch() {
    let fixture = qq_sent_fixture();
    let outbox = accepted_sent_copy(&fixture);
    let mut script = commands(21, &[2]);
    add_qq_sent_header(&mut script, true);
    run(&fixture, script, true).await.unwrap();
    let page = sent_page(&fixture);
    assert_eq!(page.items.len(), 1);
    assert!(!page.items[0].local_only);
    assert!(
        fixture
            .service
            .lock()
            .unwrap()
            .outbox_read(&outbox)
            .unwrap()
            .server_copy_message_id
            .is_some()
    );
}

/// One full `sync_targets` round for the fixture's INBOX against `script`; the
/// cadence is told what the worker would tell it.
async fn inbox_round(
    fixture: &Fixture,
    mut script: Script,
    memory: &mut FolderMemory,
    cadence: &mut Cadence,
) {
    use super::super::manager::{record_round, revision_total};
    script.splice(0..0, authenticate_commands());
    script.push(("LOGOUT".into(), "{tag} OK goodbye\r\n".into()));
    let targets = fixture
        .service
        .lock()
        .unwrap()
        .sync_folders(&fixture.account_id, &fixture.source)
        .unwrap();
    fixture
        .shared
        .reconcile_targets(&fixture.token, &targets, false);
    let (session, server) = scripted_session(script).await;
    let before = revision_total(&fixture.shared, &fixture.token);
    let outcome = sync_targets(
        &fixture.shared,
        &fixture.token,
        targets,
        session,
        memory,
        None,
        || async { Err(ImapError::Timeout) },
    )
    .await;
    assert!(matches!(outcome, Outcome::Synced));
    server.await.unwrap();
    record_round(cadence, &fixture.shared, &fixture.token, before);
}

#[tokio::test]
async fn rounds_that_store_mail_keep_the_background_wait_short_and_quiet_ones_stretch_it() {
    use super::super::cadence::{BACKGROUND_BASE, BACKGROUND_CAP};
    let fixture = fixture();
    let mut memory = FolderMemory::new();
    let mut cadence = Cadence::default();
    let background = |cadence: &Cadence| cadence.wait(Activity::Background);

    // The first round stores mail: something was found.
    let mut first: Script = commands(21, &[2, 3]).into_iter().skip(2).collect();
    add_headers(&mut first, &[(2, "Old"), (3, "Kept")]);
    inbox_round(&fixture, first, &mut memory, &mut cadence).await;
    assert_eq!(background(&cadence), BACKGROUND_BASE);

    // Same STATUS afterwards: only STATUS runs and nothing is stored. Enough of
    // those in a row and the background wait grows, up to its cap.
    let mut waits = Vec::new();
    for _ in 0..6 {
        let quiet: Script = commands(21, &[2, 3]).into_iter().skip(2).take(1).collect();
        inbox_round(&fixture, quiet, &mut memory, &mut cadence).await;
        waits.push(background(&cadence));
    }
    assert_eq!(waits[0], BACKGROUND_BASE);
    assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(*waits.last().unwrap(), BACKGROUND_CAP);

    // A new message arrives: found something, so the wait starts over.
    let mut arrival: Script = commands(21, &[2, 3, 4]).into_iter().skip(2).collect();
    add_headers(&mut arrival, &[(4, "New")]);
    arrival.push((
        "UID FETCH ".into(),
        "* 1 FETCH (UID 2 FLAGS ())\r\n* 2 FETCH (UID 3 FLAGS ())\r\n{tag} OK flags\r\n".into(),
    ));
    inbox_round(&fixture, arrival, &mut memory, &mut cadence).await;
    assert_eq!(background(&cadence), BACKGROUND_BASE);
}

#[tokio::test]
async fn a_refresh_wakes_a_worker_waiting_out_its_backoff() {
    use crate::diagnostics::{DiagnosticEvent, DiagnosticSink};
    use crate::events::EventSink;

    #[derive(Default)]
    struct Capture(Arc<Mutex<Vec<DiagnosticEvent>>>);
    impl EventSink for Capture {
        fn emit(&self, _: &CoreEvent) {}
        fn diagnostics(&self) -> Option<DiagnosticSink> {
            let records = self.0.clone();
            Some(Arc::new(move |event| records.lock().unwrap().push(event)))
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut account_config = config(json!({"kind":"direct"}));
    account_config["imap"]["host"] = json!("127.0.0.1");
    account_config["imap"]["port"] = json!(port);
    let mut fixture = fixture_with_config(account_config);
    let diagnostics = Arc::new(Capture::default());
    Arc::get_mut(&mut fixture.shared).unwrap().events = diagnostics.clone();
    let wake = std::sync::Arc::new(super::SyncWake::default());
    let (_activity, activity) = tokio::sync::watch::channel(Activity::Active);
    let worker = tokio::spawn(super::run(
        fixture.shared.clone(),
        fixture.token.clone(),
        wake.clone(),
        activity,
        crate::sync::idle::IdleHealth::watch_closed(),
    ));
    let sequence =
        |fixture: &Fixture| fixture.shared.states.lock().unwrap()[&fixture.account_id].clone();
    let failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = sequence(&fixture);
            if state.phase == SyncPhase::Failed {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    // The first retry is due in at least one second; a wake must beat that.
    wake.notify();
    let woken = tokio::time::timeout(Duration::from_millis(600), async {
        loop {
            let state = sequence(&fixture);
            if state.sequence > failed.sequence {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await;
    worker.abort();
    let _ = worker.await;
    let records = diagnostics.0.lock().unwrap();
    let [
        DiagnosticEvent::SyncFailure {
            context: first,
            code: SyncFailureCode::Network,
            ..
        },
        DiagnosticEvent::SyncRetry { context: next, .. },
        ..,
    ] = records.as_slice()
    else {
        panic!("expected a network failure followed by a retry, got {records:?}")
    };
    assert_eq!(first.account, next.account);
    assert_ne!(first.round, next.round);
    assert!(woken.is_ok(), "the worker kept waiting out its backoff");
}

#[tokio::test]
async fn provider_policy_stops_before_reconnect_or_later_folders_and_keeps_cached_headers() {
    let fixture = fixture();
    let mut existing = commands(21, &[2]);
    add_headers(&mut existing, &[(2, "Cached")]);
    run(&fixture, existing, true).await.unwrap();
    let targets = replace_targets(&fixture, &[("INBOX", true), ("Later", true)]);
    let mut script = authenticate_commands();
    script.push((
        "STATUS ".into(),
        "{tag} NO Unsafe Login. Please contact kefu@188.com for help private-marker\r\n".into(),
    ));
    let (session, peer) = scripted_session(script).await;
    let result = sync_targets(
        &fixture.shared,
        &fixture.token,
        targets,
        session,
        &mut FolderMemory::new(),
        None,
        || async {
            panic!("policy refusal must not reconnect");
            #[allow(unreachable_code)]
            Err(ImapError::Timeout)
        },
    )
    .await;
    let Outcome::Blocked(failure) = result else {
        panic!("policy must stop automatic retries");
    };
    assert_eq!(failure.code, SyncFailureCode::ProviderPolicy);
    assert!(!failure.will_retry);
    assert_eq!(failure.detail, None);
    assert!(
        !serde_json::to_string(&failure)
            .unwrap()
            .contains("private-marker")
    );
    let page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    assert!(page.items.iter().any(|mail| mail.subject == "Cached"));
    peer.await.unwrap();
}

#[tokio::test]
async fn idle_and_manual_refresh_bypass_unchanged_status_skip_for_flag_changes() {
    let fixture = fixture();
    let targets = replace_targets(&fixture, &[("INBOX", true), ("Sent", true)]);
    let mut memory = FolderMemory::new();

    // Round 1: initial full sync caches both INBOX and Sent in FolderMemory.
    let mut initial = authenticate_commands();
    initial.extend([
        (
            "STATUS \"INBOX\"".into(),
            "* STATUS \"INBOX\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 21)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE \"INBOX\"".into(),
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 2\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH\r\n{tag} OK\r\n".into(),
        ),
    ]);
    add_headers(&mut initial, &[(2, "Inbox Mail")]);
    initial.extend([
        (
            "STATUS \"Sent\"".into(),
            "* STATUS \"Sent\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 22)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE \"Sent\"".into(),
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 22] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 3\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH\r\n{tag} OK\r\n".into(),
        ),
    ]);
    add_headers(&mut initial, &[(3, "Sent Mail")]);
    let (session, peer) = scripted_session(initial).await;
    assert!(matches!(
        sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut memory,
            None,
            || async { Err(ImapError::Timeout) },
        )
        .await,
        Outcome::Synced
    ));
    peer.await.unwrap();

    // Round 2 (IDLE wake): STATUS is unchanged for both folders, so only INBOX
    // reconciles its flags (picking up \Flagged) while Sent still skips.
    apply_reconcile(
        &mut memory,
        &targets,
        Reconcile {
            all: false,
            inbox: true,
        },
    );
    let mut idle_round = authenticate_commands();
    idle_round.extend([
        (
            "STATUS \"INBOX\"".into(),
            "* STATUS \"INBOX\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 21)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE \"INBOX\"".into(),
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 2\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH 2\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID FETCH 2 (UID FLAGS)".into(),
            "* 1 FETCH (UID 2 FLAGS (\\Seen \\Flagged))\r\n{tag} OK\r\n".into(),
        ),
        (
            "STATUS \"Sent\"".into(),
            "* STATUS \"Sent\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 22)\r\n{tag} OK\r\n"
                .into(),
        ),
    ]);
    let (session, peer) = scripted_session(idle_round).await;
    assert!(matches!(
        sync_targets(
            &fixture.shared,
            &fixture.token,
            targets.clone(),
            session,
            &mut memory,
            None,
            || async { Err(ImapError::Timeout) },
        )
        .await,
        Outcome::Synced
    ));
    peer.await.unwrap();

    let starred = |subject: &str| {
        fixture
            .service
            .lock()
            .unwrap()
            .list_emails(&EmailListQuery::default())
            .unwrap()
            .items
            .into_iter()
            .find(|mail| mail.subject == subject)
            .unwrap()
            .is_starred
    };
    assert!(starred("Inbox Mail"));
    assert!(!starred("Sent Mail"));

    // Round 3 (manual refresh): STATUS is still unchanged, yet every folder
    // reconciles its flags (clearing INBOX star and setting Sent star).
    apply_reconcile(
        &mut memory,
        &targets,
        Reconcile {
            all: true,
            inbox: false,
        },
    );
    let mut refresh_round = authenticate_commands();
    refresh_round.extend([
        (
            "STATUS \"INBOX\"".into(),
            "* STATUS \"INBOX\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 21)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE \"INBOX\"".into(),
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 2\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID FETCH 2 (UID FLAGS)".into(),
            "* 1 FETCH (UID 2 FLAGS (\\Seen))\r\n{tag} OK\r\n".into(),
        ),
        (
            "STATUS \"Sent\"".into(),
            "* STATUS \"Sent\" (MESSAGES 1 UNSEEN 0 UIDNEXT 10 UIDVALIDITY 22)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE \"Sent\"".into(),
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 22] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 3\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH 3\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID FETCH 3 (UID FLAGS)".into(),
            "* 1 FETCH (UID 3 FLAGS (\\Seen \\Flagged))\r\n{tag} OK\r\n".into(),
        ),
    ]);
    let (session, peer) = scripted_session(refresh_round).await;
    assert!(matches!(
        sync_targets(
            &fixture.shared,
            &fixture.token,
            targets,
            session,
            &mut memory,
            None,
            || async { Err(ImapError::Timeout) },
        )
        .await,
        Outcome::Synced
    ));
    peer.await.unwrap();

    assert!(!starred("Inbox Mail"));
    assert!(starred("Sent Mail"));
}

#[tokio::test]
async fn starred_messages_outside_the_date_window_are_synced_and_unstarred_cleanly() {
    let fixture = fixture();
    let old_time = chrono::Utc::now() - chrono::Duration::days(90);
    let recent_time = chrono::Utc::now() - chrono::Duration::hours(2);

    // Round 1: UID 200 is inside the 30-day window; UID 50 is 90 days old and starred.
    let mut first = authenticate_commands();
    first.extend([
        (
            "STATUS ".into(),
            "* STATUS INBOX (MESSAGES 2 UNSEEN 0 UIDNEXT 201 UIDVALIDITY 21)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE ".into(),
            "* 2 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 200\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH 50\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID FETCH 50,200 (UID FLAGS INTERNALDATE".into(),
            format!(
                "{}{}{}",
                header_with(200, "Recent", "\\Seen", recent_time),
                header_with(50, "Old Star", "\\Seen \\Flagged", old_time),
                "{tag} OK headers\r\n"
            ),
        ),
    ]);
    run(&fixture, first, true).await.unwrap();

    let starred_page = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery {
            starred: Some(true),
            ..EmailListQuery::default()
        })
        .unwrap();
    assert_eq!(starred_page.items.len(), 1);
    assert_eq!(starred_page.items[0].subject, "Old Star");

    // Round 2: UID 50 is unstarred remotely (no longer returned by UID SEARCH FLAGGED),
    // yet refresh_flags still checks it and clears is_starred without deleting it.
    let mut second = authenticate_commands();
    second.extend([
        (
            "STATUS ".into(),
            "* STATUS INBOX (MESSAGES 2 UNSEEN 0 UIDNEXT 201 UIDVALIDITY 21)\r\n{tag} OK\r\n"
                .into(),
        ),
        (
            "EXAMINE ".into(),
            "* 2 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH SINCE ".into(),
            "* SEARCH 200\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH FLAGGED".into(),
            "* SEARCH\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID FETCH 50,200 (UID FLAGS)".into(),
            "* 1 FETCH (UID 200 FLAGS (\\Seen))\r\n* 2 FETCH (UID 50 FLAGS (\\Seen))\r\n{tag} OK\r\n"
                .into(),
        ),
    ]);
    run(&fixture, second, true).await.unwrap();

    let all = fixture
        .service
        .lock()
        .unwrap()
        .list_emails(&EmailListQuery::default())
        .unwrap();
    let old = all
        .items
        .iter()
        .find(|mail| mail.subject == "Old Star")
        .expect("unstarred older message remains visible instead of being marked deleted");
    assert!(!old.is_starred);
}

#[tokio::test]
async fn imap_backfill_fetches_older_messages_marks_history_end_and_preserves_on_shrink() {
    let fixture = fixture();
    let recent_uid = 200u32;
    let old_uid = 50u32;
    let old_date = chrono::Utc::now() - chrono::Duration::days(400);

    // Hot round syncs the 30-day window (UID 200).
    let mut hot = commands(21, &[recent_uid]);
    add_headers(&mut hot, &[(recent_uid, "Recent")]);
    run(&fixture, hot, true).await.unwrap();

    // Expand sync window to All and confirm task starts in Running without waking hot sync.
    let today = chrono::Utc::now().date_naive();
    {
        let mut service = fixture.service.lock().unwrap();
        assert!(
            service
                .schedule_account_backfill(
                    &fixture.account_id,
                    crate::settings::SyncWindow::All,
                    today
                )
                .unwrap()
        );
        let tasks = service.backfill_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].phase,
            crate::sync::backfill::BackfillPhase::Running
        );
    }

    let targets = fixture
        .service
        .lock()
        .unwrap()
        .sync_folders(&fixture.account_id, &fixture.source)
        .unwrap();
    let folder_id = targets[0].0.clone();

    let job = fixture
        .service
        .lock()
        .unwrap()
        .next_backfill_folder(&fixture.account_id, &fixture.source)
        .unwrap()
        .expect("backfill job queued");

    let mut script: Script = vec![
        ("LOGIN ".into(), "{tag} OK authenticated\r\n".into()),
        (
            "CAPABILITY".into(),
            "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n".into(),
        ),
        (
            "EXAMINE ".into(),
            "* 2 EXISTS\r\n* OK [UIDVALIDITY 21] ok\r\n{tag} OK\r\n".into(),
        ),
        (
            "UID SEARCH BEFORE ".into(),
            format!("* SEARCH {old_uid}\r\n{{tag}} OK\r\n"),
        ),
        (
            format!("UID FETCH {old_uid} "),
            header_with(old_uid, "Two Years Ago", "\\Seen", old_date) + "{tag} OK fetched\r\n",
        ),
        ("LOGOUT".into(), "{tag} OK goodbye\r\n".into()),
    ];
    let _ = &mut script;
    let (client, server) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(server, script));
    let endpoint = fixture
        .service
        .lock()
        .unwrap()
        .get(&fixture.account_id)
        .unwrap()
        .connection
        .imap_smtp()
        .unwrap()
        .0
        .clone();
    let login = ImapLogin {
        host: endpoint.host,
        port: 993,
        tls: crate::accounts::types::TlsMode::Implicit,
        username: "lin".into(),
        credential: crate::imap::ProtocolCredential::Password(SecretValue::new(
            "synthetic-password".into(),
        )),
    };
    let mut session = imap::test_session(Box::new(client), &login).await.unwrap();
    if let Ok(mut states) = fixture.shared.states.lock()
        && let Some(state) = states.get_mut(&fixture.account_id)
    {
        state.phase = SyncPhase::Idle;
        for f in &mut state.folders {
            f.phase = FolderSyncPhase::Idle;
        }
    }
    fixture.events.lock().unwrap().clear();
    fixture.new_mail.lock().unwrap().clear();
    let wake = SyncWake::default();
    let mut did_work = false;
    let outcome = super::super::backfill::backfill_imap_folder(
        &fixture.shared,
        &fixture.token,
        &wake,
        &mut session,
        &job,
        std::time::Instant::now(),
        &mut did_work,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        super::super::backfill::FolderTurn::Completed
    ));
    session.logout().await.unwrap();
    server.await.unwrap();

    // No NewMail event and no Syncing phase emitted by backfill.
    assert!(fixture.new_mail.lock().unwrap().is_empty());
    assert!(!fixture.events.lock().unwrap().iter().any(|state| {
        state.phase == SyncPhase::Syncing
            || state
                .folders
                .iter()
                .any(|f| f.phase == FolderSyncPhase::Syncing)
    }));

    // Coverage shows complete history and both emails are visible.
    {
        let service = fixture.service.lock().unwrap();
        assert!(service.backfill_tasks().unwrap().is_empty());
        let page = service
            .list_emails(&EmailListQuery {
                folder_id: Some(folder_id.clone()),
                ..EmailListQuery::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.coverage.history_reached_end);
        assert_eq!(page.coverage.synced_after, None);
    }

    // Shrinking back to 30 days keeps already-synced historical mail intact.
    {
        let mut service = fixture.service.lock().unwrap();
        assert!(
            !service
                .schedule_account_backfill(
                    &fixture.account_id,
                    crate::settings::SyncWindow::Days30,
                    today
                )
                .unwrap()
        );
        let page = service
            .list_emails(&EmailListQuery {
                folder_id: Some(folder_id),
                ..EmailListQuery::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().any(|m| m.subject == "Two Years Ago"));
    }
}

#[tokio::test]
async fn backfill_yields_immediately_to_pending_hot_wake() {
    let fixture = fixture();
    let hot = commands(21, &[]);
    run(&fixture, hot, true).await.unwrap();
    let today = chrono::Utc::now().date_naive();
    fixture
        .service
        .lock()
        .unwrap()
        .schedule_account_backfill(
            &fixture.account_id,
            crate::settings::SyncWindow::Days90,
            today,
        )
        .unwrap();

    let wake = SyncWake::default();
    wake.notify();
    let turn = super::super::backfill::run_budgeted(&fixture.shared, &fixture.token, &wake).await;
    assert_eq!(turn, super::super::backfill::BackfillTurn::Interrupted);
    let tasks = fixture.service.lock().unwrap().backfill_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].phase,
        crate::sync::backfill::BackfillPhase::Running
    );
}

#[tokio::test]
async fn backfill_pause_resume_cancel_and_load_older_preserve_cached_mail() {
    let fixture = fixture();
    let mut hot = commands(21, &[10]);
    add_headers(&mut hot, &[(10, "Recent")]);
    run(&fixture, hot, true).await.unwrap();

    let folder_id = fixture
        .service
        .lock()
        .unwrap()
        .sync_folders(&fixture.account_id, &fixture.source)
        .unwrap()[0]
        .0
        .clone();

    let mut service = fixture.service.lock().unwrap();
    let (account_id, queued) = service.load_older_folder(&folder_id, 90).unwrap();
    assert_eq!(account_id, fixture.account_id);
    assert!(queued);
    assert_eq!(
        service.backfill_tasks().unwrap()[0].phase,
        crate::sync::backfill::BackfillPhase::Running
    );

    assert!(
        service
            .control_account_backfill(
                &fixture.account_id,
                crate::sync::backfill::BackfillControlAction::Pause
            )
            .unwrap()
    );
    assert_eq!(
        service.backfill_tasks().unwrap()[0].phase,
        crate::sync::backfill::BackfillPhase::Paused
    );

    assert!(
        service
            .control_account_backfill(
                &fixture.account_id,
                crate::sync::backfill::BackfillControlAction::Resume
            )
            .unwrap()
    );
    assert_eq!(
        service.backfill_tasks().unwrap()[0].phase,
        crate::sync::backfill::BackfillPhase::Running
    );

    assert!(
        service
            .control_account_backfill(
                &fixture.account_id,
                crate::sync::backfill::BackfillControlAction::Cancel
            )
            .unwrap()
    );
    assert!(service.backfill_tasks().unwrap().is_empty());
    let page = service
        .list_emails(&EmailListQuery {
            folder_id: Some(folder_id),
            ..EmailListQuery::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn ews_backfill_paginates_cursor_before_and_completes() {
    let fixture = {
        let mut f = super::super::ews::tests::fixture_with_ews(Some(
            "https://controlled.example/EWS/Exchange.asmx",
        ));
        let target = f.target.clone();
        let initial_since = (f.now.date_naive() - chrono::Duration::days(30)).to_string();
        let cursor = f
            .service
            .ensure_generation(&target, &initial_since)
            .unwrap()
            .unwrap();
        let recent = super::super::ews::tests::item("recent-item", f.now);
        let assigned = f
            .service
            .ews_assign(&target, &[(recent.id.clone(), recent.change_key.clone())])
            .unwrap()
            .unwrap();
        let rec = super::super::ews::message_record(assigned[0].uid, &recent, f.now);
        f.service
            .store_message_batch(&target, &[rec], assigned[0].uid)
            .unwrap();
        f.service.finish_backfill(&target, false).unwrap();
        f.service
            .schedule_account_backfill(
                &target.account_id,
                crate::settings::SyncWindow::Days90,
                f.now.date_naive(),
            )
            .unwrap();
        let expected_target = (f.now.date_naive() - chrono::Duration::days(90)).to_string();
        let _ = (cursor, expected_target);
        f
    };

    let account_id = fixture.target.account_id.clone();
    let source = fixture.target.mailbox_source_id.clone();
    let folder_id = fixture.target.folder_id.clone();
    let target_date = (fixture.now.date_naive() - chrono::Duration::days(90)).to_string();
    let prev_highest = fixture
        .service
        .next_backfill_folder(&account_id, &source)
        .unwrap()
        .unwrap()
        .highest_uid;
    let account_revision = fixture.service.sync_identity(&account_id).unwrap().revision;
    let service = Arc::new(Mutex::new(fixture.service));
    let events: Events = Arc::new(|_: &CoreEvent| {});
    let shared = Arc::new(Shared {
        service: Arc::downgrade(&service),
        states: Mutex::new(HashMap::new()),
        global_sync_window: Mutex::new(crate::settings::SyncWindow::Days90),
        events,
    });
    let token = WorkerToken {
        account_id: account_id.clone(),
        source: source.clone(),
        is_ews: true,
        revision: account_revision,
        generation: 1,
    };
    shared.states.lock().unwrap().insert(
        account_id.clone(),
        AccountSyncState {
            account_id: account_id.clone(),
            mailbox_source_id: Some(source.clone()),
            generation: 1,
            sequence: 0,
            folder_revision: 0,
            folders: vec![FolderSyncState {
                folder_id: folder_id.clone(),
                phase: FolderSyncPhase::Idle,
                last_synced_at: None,
                failure: None,
                failure_since: None,
                data_revision: 0,
            }],
            phase: SyncPhase::Idle,
            last_synced_at: None,
            failure: None,
            failure_since: None,
        },
    );
    let job = service
        .lock()
        .unwrap()
        .next_backfill_folder(&account_id, &source)
        .unwrap()
        .unwrap();

    async fn read_http(stream: &mut DuplexStream) -> String {
        use tokio::io::AsyncReadExt as _;
        let mut bytes = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]);
                let len = head
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + len {
                    return String::from_utf8(bytes[end + 4..end + 4 + len].to_vec()).unwrap();
                }
            }
        }
    }

    async fn write_soap(stream: &mut DuplexStream, op: &str, body: &str) {
        let envelope = format!(
            "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><s:Body><m:{op}Response><m:ResponseMessages><m:{op}ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{body}</m:{op}ResponseMessage></m:ResponseMessages></m:{op}Response></s:Body></s:Envelope>"
        );
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n{envelope}",
            envelope.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
    }

    let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        // Page 1 FindItem
        let req1 = read_http(&mut server_stream).await;
        assert!(req1.contains("FindItem"));
        write_soap(
            &mut server_stream,
            "FindItem",
            "<m:RootFolder IncludesLastItemInRange=\"false\" IndexedPagingOffset=\"1\" TotalItemsInView=\"2\"><t:Items><t:Message><t:ItemId Id=\"old-2\" ChangeKey=\"k2\"/><t:Subject>Old Two</t:Subject><t:DateTimeReceived>2026-09-10T10:00:00Z</t:DateTimeReceived></t:Message></t:Items></m:RootFolder>",
        )
        .await;
        // Page 1 GetItem recipients
        let get1 = read_http(&mut server_stream).await;
        assert!(get1.contains("GetItem"));
        write_soap(
            &mut server_stream,
            "GetItem",
            "<m:Items><t:Message><t:ItemId Id=\"old-2\" ChangeKey=\"k2\"/></t:Message></m:Items>",
        )
        .await;
        // Page 2 FindItem
        let req2 = read_http(&mut server_stream).await;
        assert!(req2.contains("FindItem"));
        write_soap(
            &mut server_stream,
            "FindItem",
            "<m:RootFolder IncludesLastItemInRange=\"true\" TotalItemsInView=\"1\"><t:Items><t:Message><t:ItemId Id=\"old-1\" ChangeKey=\"k1\"/><t:Subject>Old One</t:Subject><t:DateTimeReceived>2026-08-25T10:00:00Z</t:DateTimeReceived></t:Message></t:Items></m:RootFolder>",
        )
        .await;
        // Page 2 GetItem recipients
        let get2 = read_http(&mut server_stream).await;
        assert!(get2.contains("GetItem"));
        write_soap(
            &mut server_stream,
            "GetItem",
            "<m:Items><t:Message><t:ItemId Id=\"old-1\" ChangeKey=\"k1\"/></t:Message></m:Items>",
        )
        .await;
    });

    let mut client = crate::ews::EwsClient::test_stream(Box::new(client_stream)).await;
    let wake = SyncWake::default();
    let mut did_work = false;
    let turn = super::super::backfill::backfill_ews_folder(
        &shared,
        &token,
        &wake,
        &mut client,
        &job,
        std::time::Instant::now(),
        &mut did_work,
    )
    .await
    .unwrap();
    assert!(matches!(
        turn,
        super::super::backfill::FolderTurn::Completed
    ));
    server.await.unwrap();

    let service = service.lock().unwrap();
    assert!(service.backfill_tasks().unwrap().is_empty());
    let page = service
        .list_emails(&EmailListQuery {
            folder_id: Some(folder_id),
            ..EmailListQuery::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 3);
    assert!(!page.coverage.history_reached_end);
    assert_eq!(
        page.coverage.synced_after.as_deref(),
        Some(target_date.as_str())
    );
    assert!(page.items.iter().any(|m| m.subject == "Old One"));
    assert!(page.items.iter().any(|m| m.subject == "Old Two"));
    let _ = prev_highest;
}

#[test]
fn backfill_tasks_reports_known_total_while_other_folders_are_still_queued() {
    let f = fixture();
    let account_id = f.account_id.clone();
    let source = f.source.clone();
    let mut service = f.service.lock().unwrap();
    service
        .store_folders(
            &account_id,
            &source,
            &[
                FolderRecord {
                    remote_name: "INBOX".into(),
                    parent_remote_name: None,
                    name: "INBOX".into(),
                    delimiter: Some("/".into()),
                    role: FolderRole::Inbox,
                    selectable: true,
                    attributes: vec![],
                    status: None,
                },
                FolderRecord {
                    remote_name: "Sent".into(),
                    parent_remote_name: None,
                    name: "Sent".into(),
                    delimiter: Some("/".into()),
                    role: FolderRole::Sent,
                    selectable: true,
                    attributes: vec!["\\Sent".into()],
                    status: None,
                },
            ],
        )
        .unwrap();
    let folders = service.sync_folders(&account_id, &source).unwrap();
    let inbox_id = folders
        .iter()
        .find(|(_, remote)| remote == "INBOX")
        .unwrap()
        .0
        .clone();
    let sent_id = folders
        .iter()
        .find(|(_, remote)| remote == "Sent")
        .unwrap()
        .0
        .clone();

    for folder_id in [&inbox_id, &sent_id] {
        let target = crate::storage::messages::SyncTarget {
            account_id: account_id.clone(),
            mailbox_source_id: source.clone(),
            folder_id: folder_id.clone(),
            uid_validity: 1,
        };
        service.ensure_generation(&target, "2026-10-01").unwrap();
        service.finish_backfill(&target, false).unwrap();
    }

    service
        .schedule_account_backfill(
            &account_id,
            crate::settings::SyncWindow::Days90,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        )
        .unwrap();
    let tasks = service.backfill_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].total, None);

    let inbox_target = crate::storage::messages::SyncTarget {
        account_id: account_id.clone(),
        mailbox_source_id: source.clone(),
        folder_id: inbox_id,
        uid_validity: 1,
    };
    service
        .update_backfill_progress(&inbox_target, 200, Some(585), None, None)
        .unwrap();

    // Even though Sent still has history_total = NULL, the account task reports
    // the known progress from INBOX rather than staying indeterminate.
    let tasks = service.backfill_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].fetched, 200);
    assert_eq!(tasks[0].total, Some(585));
}
