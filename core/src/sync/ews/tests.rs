use super::*;
use crate::{
    accounts::sync_plan::tests::config,
    mail::{EmailListQuery, MailAddress},
    secrets::test_support::MemorySecrets,
    storage::Flag,
};
use serde_json::json;
use tempfile::TempDir;

pub(crate) struct Fixture {
    pub(crate) _directory: TempDir,
    pub(crate) service: AccountService,
    pub(crate) target: SyncTarget,
    pub(crate) now: DateTime<Utc>,
}

fn fixture() -> Fixture {
    fixture_with_ews(None)
}

pub(crate) fn fixture_with_ews(url: Option<&str>) -> Fixture {
    let directory = TempDir::new().unwrap();
    let mut service =
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut value = config(json!({"kind":"direct"}));
    if let Some(url) = url {
        value.as_object_mut().unwrap().remove("imap");
        value.as_object_mut().unwrap().remove("smtp");
        value["backend"] = json!("ews");
        value["ews"] = json!({"url":url,"username":"DOMAIN\\synthetic"});
    }
    let account = service
        .create(
            serde_json::from_value(
                json!({"requestId":uuid::Uuid::new_v4().to_string(),"config":value}),
            )
            .unwrap(),
        )
        .unwrap();
    let folders = [EwsFolder {
        id: "remote-inbox".into(),
        parent_id: None,
        name: "Inbox".into(),
        role: Some(EwsFolderRole::Inbox),
        total: 1,
        unread: 1,
    }];
    service
        .store_folders(
            &account.id,
            &account.mailbox_source_id,
            &folder_records(&folders),
        )
        .unwrap();
    let local = service
        .sync_folders(&account.id, &account.mailbox_source_id)
        .unwrap()
        .remove(0)
        .0;
    let target = SyncTarget {
        account_id: account.id,
        mailbox_source_id: account.mailbox_source_id,
        folder_id: local,
        uid_validity: UID_VALIDITY,
    };
    Fixture {
        _directory: directory,
        service,
        target,
        now: DateTime::parse_from_rfc3339("2026-11-16T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    }
}

pub(crate) fn item(id: &str, now: DateTime<Utc>) -> EwsItem {
    EwsItem {
        id: id.into(),
        change_key: Some("key".into()),
        internet_message_id: Some(format!("<{id}@example.invalid>")),
        subject: format!("Synthetic {id}"),
        from: Some(MailAddress {
            name: None,
            address: "sender@example.invalid".into(),
        }),
        to: Vec::new(),
        cc: Vec::new(),
        reply_to: Vec::new(),
        received_at: Some(now),
        sent_at: Some(now),
        is_read: false,
        is_flagged: false,
        size: 100,
        has_attachments: false,
        preview: None,
        in_reply_to: None,
        references: None,
    }
}

pub(in crate::sync) fn store(f: &mut Fixture, items: &[EwsItem], capped: bool) -> FolderRound {
    let cursor = f
        .service
        .ensure_generation(&f.target, "2026-10-17")
        .unwrap()
        .unwrap();
    store_snapshot(
        &mut f.service,
        &f.target,
        &cursor,
        items,
        capped,
        None,
        f.now,
    )
    .unwrap()
}

pub(crate) fn emails(f: &Fixture) -> Vec<crate::mail::EmailSummary> {
    f.service
        .list_emails(
            &serde_json::from_value::<EmailListQuery>(json!({"folderId":f.target.folder_id}))
                .unwrap(),
        )
        .unwrap()
        .items
}

#[test]
fn ews_polling_preserves_local_pending_flags_and_never_downloads_a_body() {
    let mut f = fixture();
    let remote = item("one", f.now);
    assert!(store(&mut f, std::slice::from_ref(&remote), false).changed);
    let first = emails(&f).remove(0);
    f.service
        .set_message_flag(&first.id, Flag::Seen, true)
        .unwrap();
    f.service
        .set_message_flag(&first.id, Flag::Flagged, true)
        .unwrap();
    store(&mut f, &[remote], false);
    let row = emails(&f).remove(0);
    assert!(row.is_read && row.is_starred && row.pending_changes);
    assert_eq!(
        f.service.get_email(&row.id).unwrap().summary.body_state,
        crate::mail::BodyState::Missing
    );
    // A reverse toggle still sees the true local state and replaces the wish.
    f.service
        .set_message_flag(&row.id, Flag::Seen, false)
        .unwrap();
    let change = f
        .service
        .pending_flag_changes(&f.target.account_id, 10)
        .unwrap()
        .into_iter()
        .find(|change| change.flag == Flag::Seen)
        .unwrap();
    assert!(!change.wanted);
    use super::super::writeback::Queue;
    let service = std::sync::Mutex::new(f.service);
    service.finish(&change).unwrap();
    assert!(
        service
            .lock()
            .unwrap()
            .pending_flag_changes(&f.target.account_id, 10)
            .unwrap()
            .iter()
            .all(|change| change.flag != Flag::Seen)
    );
}

#[test]
fn only_complete_ews_enumerations_reconcile_external_deletions() {
    let mut f = fixture();
    let first = item("one", f.now);
    let second = item("two", f.now);
    store(&mut f, &[first.clone(), second.clone()], false);
    store(&mut f, std::slice::from_ref(&second), true);
    assert_eq!(
        emails(&f).len(),
        2,
        "a capped page cannot prove one disappeared"
    );
    store(&mut f, &[second], false);
    assert_eq!(emails(&f).len(), 1);
    assert_eq!(emails(&f)[0].subject, "Synthetic two");
}

#[test]
fn ews_arrivals_skip_initial_history_and_quiet_rounds_but_include_new_mail() {
    let mut f = fixture();
    let first = item("one", f.now);
    assert!(
        store(&mut f, std::slice::from_ref(&first), false)
            .arrivals
            .is_empty()
    );
    let quiet = store(&mut f, std::slice::from_ref(&first), false);
    assert!(!quiet.changed && quiet.arrivals.is_empty());
    let second = item("two", f.now);
    let next = store(&mut f, &[second, first], false);
    assert!(next.changed);
    assert_eq!(next.arrivals.len(), 1);
    assert_eq!(next.arrivals[0].subject, "Synthetic two");
}

#[test]
fn ews_folder_roles_and_parents_use_remote_identity_and_drop_unknown_parent() {
    let folders = [
        EwsFolder {
            id: "parent".into(),
            parent_id: Some("root-outside-list".into()),
            name: "Localized inbox".into(),
            role: Some(EwsFolderRole::Inbox),
            total: 0,
            unread: 0,
        },
        EwsFolder {
            id: "child".into(),
            parent_id: Some("parent".into()),
            name: "Trash".into(),
            role: None,
            total: 0,
            unread: 0,
        },
    ];
    let records = folder_records(&folders);
    assert_eq!(records[0].role, FolderRole::Inbox);
    assert!(records[0].parent_remote_name.is_none());
    assert_eq!(records[1].parent_remote_name.as_deref(), Some("parent"));
    assert_eq!(records[1].role, FolderRole::Other);
}

#[tokio::test]
async fn an_ews_body_uses_its_https_endpoint_and_never_the_imap_connector() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "https://127.0.0.1:{}/EWS/Exchange.asmx",
        listener.local_addr().unwrap().port()
    );
    let mut f = fixture_with_ews(Some(&endpoint));
    let remote = item("one", f.now);
    store(&mut f, &[remote], false);
    let id = emails(&f).remove(0).id;
    let service = Arc::new(std::sync::Mutex::new(f.service));
    let manager =
        crate::sync::bodies::BodyManager::new(&service, tokio::runtime::Handle::current());
    let (connector, mut imap) = tokio::sync::mpsc::unbounded_channel();
    manager.set_test_connector(connector);
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // A controlled EOF fails TLS without relying on a system trust store.
        drop(stream);
    });
    let detail = tokio::time::timeout(std::time::Duration::from_secs(3), manager.fetch(id))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), peer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.body_state, crate::mail::BodyState::Failed);
    assert!(
        imap.try_recv().is_err(),
        "EWS must never open the IMAP fallback"
    );
}

#[test]
fn ews_folder_parent_cycles_cannot_reach_the_sidebar() {
    let folder = |id: &str, parent: &str| EwsFolder {
        id: id.into(),
        parent_id: Some(parent.into()),
        name: id.into(),
        role: None,
        total: 0,
        unread: 0,
    };
    let records = folder_records(&[
        folder("self", "self"),
        folder("a", "b"),
        folder("b", "a"),
        folder("leaf", "a"),
    ]);
    assert!(
        records
            .iter()
            .all(|folder| folder.parent_remote_name.is_none())
    );
}

async fn soap_request(peer: &mut tokio::io::BufReader<tokio::io::DuplexStream>) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut length = None;
    loop {
        let mut line = String::new();
        assert!(peer.read_line(&mut line).await.unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.unwrap()];
    peer.read_exact(&mut body).await.unwrap();
    String::from_utf8(body).unwrap()
}

async fn soap_reply(
    peer: &mut tokio::io::BufReader<tokio::io::DuplexStream>,
    operation: &str,
    payload: &str,
) {
    use tokio::io::AsyncWriteExt;
    let body = format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><s:Body><m:{operation}Response><m:ResponseMessages><m:{operation}ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{payload}</m:{operation}ResponseMessage></m:ResponseMessages></m:{operation}Response></s:Body></s:Envelope>"
    );
    peer.get_mut()
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn an_empty_nonterminal_ews_page_fails_instead_of_looping() {
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut peer = tokio::io::BufReader::new(peer);
        assert!(soap_request(&mut peer).await.contains("FindItem"));
        soap_reply(&mut peer,"FindItem","<m:RootFolder IncludesLastItemInRange=\"false\" IndexedPagingOffset=\"1\"><t:Items/></m:RootFolder>").await;
    });
    let mut client = EwsClient::test_stream(Box::new(stream)).await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        enumerate(&mut client, "inbox", Utc::now()),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn repeated_ews_items_with_advancing_cursors_stop_at_the_page_budget() {
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut peer = tokio::io::BufReader::new(peer);
        for next in 1..=MAX_PAGES {
            assert!(soap_request(&mut peer).await.contains("FindItem"));
            soap_reply(&mut peer,"FindItem",&format!("<m:RootFolder IncludesLastItemInRange=\"false\" IndexedPagingOffset=\"{next}\"><t:Items><t:Message><t:ItemId Id=\"duplicate\"/></t:Message></t:Items></m:RootFolder>")).await;
            assert!(soap_request(&mut peer).await.contains("GetItem"));
            soap_reply(
                &mut peer,
                "GetItem",
                "<m:Items><t:Message><t:ItemId Id=\"duplicate\"/></t:Message></m:Items>",
            )
            .await;
        }
    });
    let mut client = EwsClient::test_stream(Box::new(stream)).await;
    let (items, capped) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        enumerate(&mut client, "inbox", Utc::now()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        capped,
        "exhausting the request budget cannot claim complete coverage"
    );
    server.await.unwrap();
}
