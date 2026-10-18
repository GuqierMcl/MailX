//! Scripted IMAP plus real temporary SQLite tests for the inbox driver.

use super::*;
use crate::accounts::sync_plan::tests::config;
use crate::imap::{FolderStatus, ImapLogin};
use crate::mail::{EmailListQuery, FolderRole};
use crate::secrets::{SecretValue, test_support::MemorySecrets};
use crate::sync::folders::FolderRecord;
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

struct Fixture {
    _directory: TempDir,
    service: Arc<Mutex<AccountService>>,
    shared: Arc<Shared>,
    account_id: String,
    source: String,
}

fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let mut service =
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let account = service
        .create(
            serde_json::from_value(json!({
                "requestId": uuid::Uuid::new_v4().to_string(),
                "config": config(json!({"kind":"direct"}))
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
                role: FolderRole::Inbox,
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
    let shared = Arc::new(Shared {
        service: Arc::downgrade(&service),
        states: Mutex::new(HashMap::new()),
        listener: Arc::new(|_| {}),
    });
    Fixture {
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
    vec![
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
    ]
}

fn header(uid: u32, subject: &str) -> String {
    let date = chrono::Utc::now().format("%d-%b-%Y");
    let bytes = format!(
        "From: Sender <sender@example.com>\r\nSubject: {subject}\r\nTo: lin@example.com\r\n\r\n"
    );
    format!(
        "* 1 FETCH (UID {uid} FLAGS () INTERNALDATE \"{date} 12:00:00 +0000\" RFC822.SIZE 200 BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" 10 1) BODY[HEADER.FIELDS (DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID)] {{{}}}\r\n{bytes})\r\n",
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

async fn run(fixture: &Fixture, mut script: Script, success: bool) -> Result<(), SyncError> {
    if success {
        script.push(("LOGOUT".into(), "{tag} OK goodbye\r\n".into()));
    }
    let (client, server) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(server, script));
    let login = ImapLogin {
        host: "imap.example.com".into(),
        port: 993,
        tls: crate::accounts::types::TlsMode::Implicit,
        username: "lin".into(),
        password: SecretValue::new("synthetic-password".into()),
    };
    let mut session = imap::test_session(Box::new(client), &login).await.unwrap();
    let result = super::super::messages::sync_inboxes(
        &fixture.shared,
        &fixture.account_id,
        &fixture.source,
        &mut session,
    )
    .await;
    if success {
        session.logout().await.unwrap();
    } else {
        drop(session);
    }
    server.await.unwrap();
    result
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
