use super::*;

fn msg(
    uid: u32,
    subject: &str,
    received_at: &str,
    in_reply_to: Option<&str>,
    references: Option<&str>,
) -> MessageRecord {
    MessageRecord {
        subject: subject.into(),
        received_at: received_at.into(),
        in_reply_to: in_reply_to.map(str::to_owned),
        references_header: references.map(str::to_owned),
        ..record(uid)
    }
}

fn mid(uid: u32) -> String {
    format!("message-{uid}@example.test")
}

fn conversation(f: &Fixture, account: &str, uid: u32) -> Option<String> {
    f.db.query_row(
        "SELECT conversation_id FROM messages WHERE account_id = ?1 AND uid = ?2",
        params![account, uid],
        |row| row.get(0),
    )
    .unwrap()
}

fn local_id(f: &Fixture, uid: u32) -> String {
    f.db.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
        params![f.account.id, uid],
        |row| row.get(0),
    )
    .unwrap()
}

fn cid(f: &Fixture, uid: u32) -> Option<String> {
    conversation(f, &f.account.id, uid)
}

#[test]
fn references_in_reply_to_threads_messages_by_account() {
    let mut f = setup();
    put(
        &mut f,
        &[msg(1, "Hello", "2026-10-01T00:00:00.000Z", None, None)],
    );
    put(
        &mut f,
        &[msg(
            2,
            "Re: Hello",
            "2026-10-02T00:00:00.000Z",
            Some(&mid(1)),
            None,
        )],
    );
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
}

#[test]
fn references_header_uses_full_chain() {
    let mut f = setup();
    put(
        &mut f,
        &[
            msg(1, "Plan", "2026-10-01T00:00:00.000Z", None, None),
            msg(
                2,
                "Re: Plan",
                "2026-10-02T00:00:00.000Z",
                Some(&mid(1)),
                Some(&mid(1)),
            ),
            msg(
                3,
                "Re: Plan",
                "2026-10-03T00:00:00.000Z",
                Some(&mid(2)),
                Some(&format!("{} {}", mid(1), mid(2))),
            ),
        ],
    );
    let first = cid(&f, 1);
    assert!(first.is_some());
    assert_eq!(first, cid(&f, 2));
    assert_eq!(first, cid(&f, 3));
}

#[test]
fn a_parent_arriving_after_its_reply_joins_the_reply() {
    let mut f = setup();
    put(
        &mut f,
        &[msg(
            2,
            "Re: Late",
            "2026-10-02T00:00:00.000Z",
            Some(&mid(1)),
            None,
        )],
    );
    assert!(cid(&f, 2).is_some());
    put(
        &mut f,
        &[msg(1, "Late", "2026-10-01T00:00:00.000Z", None, None)],
    );
    assert_eq!(cid(&f, 1), cid(&f, 2));
}

const ME: &str = "lin@example.com";

fn addr(address: &str) -> MailAddress {
    MailAddress {
        name: None,
        address: address.into(),
    }
}

/// A header-less message from `from` to `to`.
fn mail(uid: u32, subject: &str, at: &str, from: &str, to: &[&str]) -> MessageRecord {
    MessageRecord {
        from_address: from.into(),
        to_addresses: to.iter().map(|a| addr(a)).collect(),
        cc_addresses: vec![],
        ..msg(uid, subject, at, None, None)
    }
}

#[test]
fn subject_fallback_groups_a_two_way_exchange_within_30_days() {
    let mut f = setup();
    put(
        &mut f,
        &[mail(
            1,
            "Invoice",
            "2026-09-01T00:00:00.000Z",
            ME,
            &["x@example.test"],
        )],
    );
    put(
        &mut f,
        &[mail(
            2,
            "RE: invoice",
            "2026-09-06T00:00:00.000Z",
            "X@example.test",
            &[ME],
        )],
    );
    put(
        &mut f,
        &[mail(
            3,
            "回复: Invoice",
            "2026-10-07T00:00:00.000Z",
            ME,
            &["x@example.test"],
        )],
    );
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
    // 31 days after the first and 31 after the second: out of both windows.
    assert!(cid(&f, 3).is_none());
}

#[test]
fn one_way_notifications_with_the_same_subject_are_not_a_conversation() {
    let mut f = setup();
    let records: Vec<_> = (1..=4)
        .map(|uid| {
            mail(
                uid,
                "Welcome",
                "2026-10-01T00:00:00.000Z",
                "no-reply@example.test",
                &[ME],
            )
        })
        .collect();
    put(&mut f, &records);
    for uid in 1..=4 {
        assert!(cid(&f, uid).is_none(), "{uid}");
        let page = f.service.get_conversation(&local_id(&f, uid)).unwrap();
        assert_eq!(page.messages.len(), 1);
    }
}

#[test]
fn the_same_subject_with_different_counterparts_is_not_merged() {
    let mut f = setup();
    put(
        &mut f,
        &[
            mail(1, "Hi", "2026-10-01T00:00:00.000Z", ME, &["x@example.test"]),
            mail(
                2,
                "Re: Hi",
                "2026-10-02T00:00:00.000Z",
                "x@example.test",
                &[ME],
            ),
        ],
    );
    put(
        &mut f,
        &[
            mail(3, "Hi", "2026-10-03T00:00:00.000Z", ME, &["y@example.test"]),
            mail(
                4,
                "Re: Hi",
                "2026-10-04T00:00:00.000Z",
                "y@example.test",
                &[ME],
            ),
        ],
    );
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
    assert!(cid(&f, 3).is_some());
    assert_eq!(cid(&f, 3), cid(&f, 4));
    assert_ne!(cid(&f, 1), cid(&f, 3));
}

#[test]
fn siblings_referencing_an_uncached_root_share_a_conversation() {
    let mut f = setup();
    let root = "root@example.test";
    put(
        &mut f,
        &[
            msg(
                1,
                "Re: [repo] PR",
                "2026-10-01T00:00:00.000Z",
                Some(root),
                Some(root),
            ),
            msg(
                2,
                "Re: [repo] PR",
                "2026-10-02T00:00:00.000Z",
                Some(root),
                Some(root),
            ),
            msg(
                3,
                "Re: [repo] PR",
                "2026-10-03T00:00:00.000Z",
                Some(root),
                Some(root),
            ),
        ],
    );
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
    assert_eq!(cid(&f, 1), cid(&f, 3));
}

#[test]
fn a_self_sent_message_in_sent_and_inbox_is_listed_once() {
    let mut f = setup();
    let mut sent = mail(11, "Note", "2026-10-01T00:00:00.000Z", ME, &[ME]);
    sent.message_id_header = Some("self@example.test".into());
    let mut inbox = sent.clone();
    inbox.uid = 12;
    inbox.received_at = "2026-10-01T00:00:05.000Z".into();
    put_sent(&mut f, &[sent]);
    put(&mut f, &[inbox]);
    put(
        &mut f,
        &[msg(
            13,
            "Re: Note",
            "2026-10-02T00:00:00.000Z",
            Some("self@example.test"),
            None,
        )],
    );
    for opened in [11, 12] {
        let page = f.service.get_conversation(&local_id(&f, 13)).unwrap();
        assert_eq!(page.messages.len(), 2);
        let page = f.service.get_conversation(&local_id(&f, opened)).unwrap();
        assert_eq!(page.messages.len(), 2, "{opened}");
        assert!(page.messages.iter().any(|m| m.id == local_id(&f, opened)));
    }
    // Not opened: the non-Sent copy wins.
    let page = f.service.get_conversation(&local_id(&f, 13)).unwrap();
    assert!(page.messages.iter().any(|m| m.id == local_id(&f, 12)));
    assert!(page.messages.iter().all(|m| m.id != local_id(&f, 11)));
}

#[test]
fn rethread_step_rebuilds_old_rows_and_finishes() {
    let mut f = setup();
    put(
        &mut f,
        &[
            msg(1, "Ask", "2026-10-01T00:00:00.000Z", None, None),
            msg(
                2,
                "Re: Ask",
                "2026-10-02T00:00:00.000Z",
                Some(&mid(1)),
                None,
            ),
            msg(3, "Alone", "2026-10-03T00:00:00.000Z", None, None),
        ],
    );
    f.db.execute(
        "UPDATE messages SET conversation_id = NULL, threading_version = 0",
        [],
    )
    .unwrap();
    assert!(cid(&f, 1).is_none());
    assert_eq!(f.service.rethread_step(2).unwrap(), 2);
    assert_eq!(f.service.rethread_step(2).unwrap(), 1);
    assert_eq!(f.service.rethread_step(2).unwrap(), 0);
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
    assert!(cid(&f, 3).is_none());
    let pending: i64 =
        f.db.query_row(
            "SELECT COUNT(*) FROM messages WHERE threading_version < 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn processed_rows_are_marked_even_when_they_stay_solo() {
    let mut f = setup();
    put(
        &mut f,
        &[msg(1, "Alone", "2026-10-01T00:00:00.000Z", None, None)],
    );
    assert!(cid(&f, 1).is_none());
    let version: i64 =
        f.db.query_row(
            "SELECT threading_version FROM messages WHERE uid = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, crate::storage::conversations::THREADING_VERSION);
}

#[test]
fn subject_fallback_skipped_when_header_exists() {
    let mut f = setup();
    put(
        &mut f,
        &[
            msg(2, "Other", "2026-10-01T00:00:00.000Z", None, None),
            msg(1, "Topic", "2026-10-02T00:00:00.000Z", None, Some(&mid(2))),
        ],
    );
    put(
        &mut f,
        &[msg(3, "Topic", "2026-10-03T00:00:00.000Z", None, None)],
    );
    assert!(cid(&f, 1).is_some());
    assert_ne!(cid(&f, 1), cid(&f, 3));
    assert!(cid(&f, 3).is_none());
}

#[test]
fn cross_account_messages_not_merged() {
    let mut f = setup();
    let (other, other_target) = add_account(&mut f.service);
    f.service
        .ensure_generation(&other_target, "2026-09-18")
        .unwrap()
        .unwrap();
    let shared = msg(1, "Same", "2026-10-01T00:00:00.000Z", None, None);
    let reply = msg(
        2,
        "Re: Same",
        "2026-10-02T00:00:00.000Z",
        Some(&mid(1)),
        None,
    );
    put(&mut f, &[shared.clone(), reply.clone()]);
    f.service
        .store_message_batch(&other_target, &[reply], 2)
        .unwrap()
        .unwrap();
    // The other account only has the reply; its parent lives in the first account.
    let here = cid(&f, 2);
    let there = conversation(&f, &other.id, 2);
    assert!(here.is_some() && there.is_some());
    assert_ne!(here, there);
    assert_eq!(cid(&f, 1), here);
}

#[test]
fn get_conversation_returns_all_messages() {
    let mut f = setup();
    put(
        &mut f,
        &[
            msg(
                3,
                "Re: Ask",
                "2026-10-03T00:00:00.000Z",
                Some(&mid(2)),
                Some(&format!("{} {}", mid(1), mid(2))),
            ),
            msg(1, "Ask", "2026-10-01T00:00:00.000Z", None, None),
            msg(
                2,
                "Re: Ask",
                "2026-10-02T00:00:00.000Z",
                Some(&mid(1)),
                Some(&mid(1)),
            ),
            msg(4, "Unrelated", "2026-10-04T00:00:00.000Z", None, None),
        ],
    );
    let page = f.service.get_conversation(&local_id(&f, 2)).unwrap();
    let ids: Vec<_> = page.messages.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids, [local_id(&f, 1), local_id(&f, 2), local_id(&f, 3)]);
    assert!(page.possibly_incomplete, "backfill has not finished");
}

#[test]
fn get_conversation_degrades_gracefully_for_solo_message() {
    let mut f = setup();
    put(
        &mut f,
        &[msg(1, "Alone", "2026-10-01T00:00:00.000Z", None, None)],
    );
    assert!(cid(&f, 1).is_none());
    let id = local_id(&f, 1);
    let page = f.service.get_conversation(&id).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, id);
    assert!(!page.possibly_incomplete);
    assert_eq!(
        f.service
            .get_conversation(&Uuid::new_v4().to_string())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn subject_fallback_skipped_for_large_groups() {
    let mut f = setup();
    let records: Vec<_> = (1..=21)
        .map(|uid| msg(uid, "Weekly digest", "2026-10-01T00:00:00.000Z", None, None))
        .collect();
    put(&mut f, &records);
    for uid in 1..=21 {
        assert!(cid(&f, uid).is_none(), "{uid}");
    }
}

#[test]
fn junk_and_trash_copies_are_left_out_of_a_conversation() {
    let mut f = setup();
    put(
        &mut f,
        &[
            msg(1, "Ask", "2026-10-01T00:00:00.000Z", None, None),
            msg(
                2,
                "Re: Ask",
                "2026-10-02T00:00:00.000Z",
                Some(&mid(1)),
                None,
            ),
        ],
    );
    f.db.execute(
        "UPDATE folders SET role = 'trash' WHERE id = (SELECT folder_id FROM messages WHERE uid = 2)",
        [],
    )
    .unwrap();
    // Both copies now sit in Trash: only the opened one remains.
    let page = f.service.get_conversation(&local_id(&f, 2)).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, local_id(&f, 2));
}
