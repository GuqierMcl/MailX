use super::*;
use crate::sync::ews::tests::{emails, fixture_with_ews, item, store};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};

async fn request(peer: &mut BufReader<DuplexStream>) -> String {
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

async fn success(peer: &mut BufReader<DuplexStream>, operation: &str) {
    let body = format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\"><s:Body><m:{operation}Response><m:ResponseMessages><m:{operation}ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode></m:{operation}ResponseMessage></m:ResponseMessages></m:{operation}Response></s:Body></s:Envelope>"
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
async fn ews_false_flag_values_are_sent_together_and_acknowledged() {
    let mut f = fixture_with_ews(Some("https://controlled.example/EWS/Exchange.asmx"));
    let mut remote = item("one", f.now);
    remote.is_read = true;
    remote.is_flagged = true;
    store(&mut f, &[remote], false);
    let id = emails(&f).remove(0).id;
    f.service.set_message_flag(&id, Flag::Seen, false).unwrap();
    f.service
        .set_message_flag(&id, Flag::Flagged, false)
        .unwrap();
    let changes = f
        .service
        .pending_flag_changes(&f.target.account_id, 10)
        .unwrap();
    let service = Arc::new(Mutex::new(f.service));
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        let body = request(&mut peer).await;
        assert!(body.contains("<t:IsRead>false</t:IsRead>"));
        assert!(body.contains("<t:FlagStatus>NotFlagged</t:FlagStatus>"));
        assert!(body.contains("Id=\"one\""));
        assert!(
            !body.contains("ChangeKey="),
            "a stale metadata key must not block a queued field update"
        );
        success(&mut peer, "UpdateItem").await;
    });
    let mut client = EwsClient::test_stream(Box::new(stream)).await;
    let result = push(
        &service,
        &f.target.account_id,
        &f.target.mailbox_source_id,
        &mut client,
        changes,
        Vec::new(),
        WriteBackReport::default(),
    )
    .await;
    assert!(matches!(
        result,
        RoundEnd::Done(WriteBackReport {
            sent: 2,
            waiting: 0,
            ..
        })
    ));
    assert!(
        service
            .lock()
            .unwrap()
            .pending_flag_changes(&f.target.account_id, 10)
            .unwrap()
            .is_empty()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn an_ews_update_cannot_acknowledge_a_newer_reverse_toggle() {
    let mut f = fixture_with_ews(Some("https://controlled.example/EWS/Exchange.asmx"));
    let mut remote = item("one", f.now);
    remote.is_read = true;
    store(&mut f, &[remote], false);
    let id = emails(&f).remove(0).id;
    f.service.set_message_flag(&id, Flag::Seen, false).unwrap();
    let changes = f
        .service
        .pending_flag_changes(&f.target.account_id, 10)
        .unwrap();
    let service = Arc::new(Mutex::new(f.service));
    let racing = service.clone();
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        assert!(
            request(&mut peer)
                .await
                .contains("<t:IsRead>false</t:IsRead>")
        );
        racing
            .lock()
            .unwrap()
            .set_message_flag(&id, Flag::Seen, true)
            .unwrap();
        success(&mut peer, "UpdateItem").await;
    });
    let mut client = EwsClient::test_stream(Box::new(stream)).await;
    push(
        &service,
        &f.target.account_id,
        &f.target.mailbox_source_id,
        &mut client,
        changes,
        Vec::new(),
        WriteBackReport::default(),
    )
    .await;
    let pending = service
        .lock()
        .unwrap()
        .pending_flag_changes(&f.target.account_id, 10)
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0].wanted,
        "the newer true wish must remain queued after writing false"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn ews_soft_and_permanent_deletions_complete_the_shared_queue() {
    for permanent in [false, true] {
        let mut f = fixture_with_ews(Some("https://controlled.example/EWS/Exchange.asmx"));
        let remote = item("one", f.now);
        store(&mut f, &[remote], false);
        let id = emails(&f).remove(0).id;
        if !permanent {
            use crate::{mail::FolderRole, sync::folders::FolderRecord};
            let record = |name: &str, role| FolderRecord {
                remote_name: name.into(),
                parent_remote_name: None,
                name: name.into(),
                delimiter: None,
                role,
                selectable: true,
                attributes: Vec::new(),
                status: None,
            };
            f.service
                .store_folders(
                    &f.target.account_id,
                    &f.target.mailbox_source_id,
                    &[
                        record("remote-inbox", FolderRole::Inbox),
                        record("remote-trash", FolderRole::Trash),
                    ],
                )
                .unwrap();
        }
        let scheduled = f
            .service
            .schedule_deletion(
                std::slice::from_ref(&id),
                permanent,
                f.now,
                chrono::Duration::zero(),
            )
            .unwrap();
        assert_eq!(scheduled.permanent, permanent);
        let (deletions, _) = f
            .service
            .claim_due_deletions(&f.target.account_id, f.now)
            .unwrap();
        assert_eq!(deletions.len(), 1);
        assert_eq!(
            f.service
                .cancel_deletion(&scheduled.batch_id)
                .unwrap_err()
                .code,
            ErrorCode::AlreadyDeleting
        );
        let service = Arc::new(Mutex::new(f.service));
        let (stream, peer) = tokio::io::duplex(16 * 1024);
        let server = tokio::spawn(async move {
            let mut peer = BufReader::new(peer);
            let body = request(&mut peer).await;
            assert!(body.contains(if permanent {
                "DeleteType=\"HardDelete\""
            } else {
                "DeleteType=\"MoveToDeletedItems\""
            }));
            assert!(body.contains("Id=\"one\""));
            success(&mut peer, "DeleteItem").await;
        });
        let mut client = EwsClient::test_stream(Box::new(stream)).await;
        let result = push(
            &service,
            &f.target.account_id,
            &f.target.mailbox_source_id,
            &mut client,
            Vec::new(),
            deletions,
            WriteBackReport::default(),
        )
        .await;
        assert!(matches!(
            result,
            RoundEnd::Done(WriteBackReport {
                deleted: 1,
                waiting: 0,
                ..
            })
        ));
        {
            let mut service = service.lock().unwrap();
            assert_eq!(
                service.get_email(&id).unwrap_err().code,
                ErrorCode::NotFound
            );
            assert!(
                service
                    .claim_due_deletions(&f.target.account_id, f.now)
                    .unwrap()
                    .0
                    .is_empty()
            );
            assert_eq!(
                service
                    .cancel_deletion(&scheduled.batch_id)
                    .unwrap_err()
                    .code,
                ErrorCode::NotFound
            );
        }
        server.await.unwrap();
    }
}
