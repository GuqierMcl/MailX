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

#[test]
fn subject_fallback_groups_within_30_days() {
    let mut f = setup();
    put(
        &mut f,
        &[msg(1, "Invoice", "2026-09-01T00:00:00.000Z", None, None)],
    );
    put(
        &mut f,
        &[msg(
            2,
            "RE: invoice",
            "2026-09-06T00:00:00.000Z",
            None,
            None,
        )],
    );
    put(
        &mut f,
        &[msg(
            3,
            "回复: Invoice",
            "2026-10-07T00:00:00.000Z",
            None,
            None,
        )],
    );
    assert!(cid(&f, 1).is_some());
    assert_eq!(cid(&f, 1), cid(&f, 2));
    // 31 days after the first and 31 after the second: out of both windows.
    assert!(cid(&f, 3).is_none());
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
