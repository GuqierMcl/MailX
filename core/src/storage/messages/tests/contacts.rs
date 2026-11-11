//! Sender filter, sender grouping and the aggregation rules.

use super::*;

const T: &str = "2026-10-18T09:00:00.000Z";

fn from(uid: u32, address: &str, received_at: &str, is_read: bool) -> MessageRecord {
    MessageRecord {
        from_name: Some(format!("Name {uid}")),
        from_address: address.into(),
        received_at: received_at.into(),
        is_read,
        ..record(uid)
    }
}

fn grouped() -> EmailListQuery {
    EmailListQuery {
        group_senders: true,
        ..inbox_query()
    }
}

fn rule(f: &mut Fixture, address: &str) {
    f.service.set_contact_aggregation(address, true).unwrap();
}

fn rows(f: &Fixture, query: &EmailListQuery) -> Vec<EmailSummary> {
    f.service.list_emails(query).unwrap().items
}

fn uids(f: &Fixture, items: &[EmailSummary]) -> Vec<u32> {
    items
        .iter()
        .map(|item| {
            f.db.query_row("SELECT uid FROM messages WHERE id = ?1", [&item.id], |r| {
                r.get(0)
            })
            .unwrap()
        })
        .collect()
}

#[test]
fn from_address_filters_case_insensitively_and_validates() {
    let mut f = setup();
    put(
        &mut f,
        &[
            from(1, "Alice@Example.Test", "2026-10-18T09:00:00.000Z", false),
            from(2, "alice@example.test", "2026-10-18T10:00:00.000Z", true),
            from(3, "bob@example.test", "2026-10-18T11:00:00.000Z", false),
        ],
    );
    for value in ["ALICE@example.test", "  alice@EXAMPLE.test "] {
        let items = rows(
            &f,
            &EmailListQuery {
                from_address: Some(value.into()),
                ..inbox_query()
            },
        );
        assert_eq!(uids(&f, &items), vec![2, 1], "{value}");
    }
    for bad in [
        "",
        "   ",
        "no-at-sign",
        "a@b\n.test",
        &format!("{}@x", "a".repeat(320)),
    ] {
        let error = f
            .service
            .list_emails(&EmailListQuery {
                from_address: Some(bad.into()),
                ..inbox_query()
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation, "{bad:?}");
        assert_eq!(error.field.as_deref(), Some("fromAddress"), "{bad:?}");
    }
}

#[test]
fn from_address_matches_local_sent_copies_by_account_address() {
    let f = setup();
    let local = local_copy(&f, 1, "2026-10-18T10:00:00.000Z", "mine", 0);
    let mut query = sent_query(&f);
    query.from_address = Some(f.account.email_address.to_uppercase());
    assert_eq!(listed(&f, &query), vec![local]);
    query.from_address = Some("someone@else.test".into());
    assert!(listed(&f, &query).is_empty());
}

#[test]
fn grouping_collapses_a_sender_into_its_newest_message_with_counts() {
    let mut f = setup();
    put(
        &mut f,
        &[
            from(1, "news@example.test", "2026-10-18T08:00:00.000Z", true),
            from(2, "friend@example.test", "2026-10-18T09:00:00.000Z", false),
            from(3, "News@Example.test", "2026-10-18T10:00:00.000Z", false),
            from(4, "news@example.test", "2026-10-18T11:00:00.000Z", true),
            from(5, "other@example.test", "2026-10-18T12:00:00.000Z", true),
        ],
    );
    // Without a rule, or without asking for grouping, nothing changes.
    assert_eq!(rows(&f, &grouped()).len(), 5);
    rule(&mut f, "NEWS@example.test");
    assert_eq!(rows(&f, &inbox_query()).len(), 5);
    let items = rows(&f, &grouped());
    assert_eq!(uids(&f, &items), vec![5, 4, 2]);
    assert_eq!(items[0].sender_group, None);
    assert_eq!(
        items[1].sender_group,
        Some(SenderGroup {
            address: "news@example.test".into(),
            total: 3,
            unread: 1
        })
    );
    assert_eq!(items[2].sender_group, None);
    assert!(items[1].is_read, "the row is the newest message itself");

    // Unread tab: only the group's newest unread message, counts over unread.
    let unread = rows(
        &f,
        &EmailListQuery {
            unread: Some(true),
            ..grouped()
        },
    );
    assert_eq!(uids(&f, &unread), vec![3, 2]);
    assert_eq!(
        unread[0].sender_group.as_ref().map(|g| (g.total, g.unread)),
        Some((1, 1))
    );

    // A group without unread mail does not show in the Unread tab at all.
    f.db.execute("UPDATE messages SET is_read = 1 WHERE uid = 3", [])
        .unwrap();
    let unread = rows(
        &f,
        &EmailListQuery {
            unread: Some(true),
            ..grouped()
        },
    );
    assert_eq!(uids(&f, &unread), vec![2]);

    // Time bounds narrow the counted set as well.
    let bounded = rows(
        &f,
        &EmailListQuery {
            received_before: Some("2026-10-18T10:30:00.000Z".into()),
            ..grouped()
        },
    );
    assert_eq!(uids(&f, &bounded), vec![3, 2]);
    assert_eq!(bounded[0].sender_group.as_ref().map(|g| g.total), Some(2));

    // Removing the rule brings the individual rows back.
    f.service
        .set_contact_aggregation("news@example.test", false)
        .unwrap();
    assert_eq!(rows(&f, &grouped()).len(), 5);
}

#[test]
fn text_search_and_from_address_ignore_grouping() {
    let mut f = setup();
    put(
        &mut f,
        &[
            from(1, "news@example.test", "2026-10-18T08:00:00.000Z", false),
            from(2, "news@example.test", "2026-10-18T09:00:00.000Z", false),
            from(3, "other@example.test", "2026-10-18T10:00:00.000Z", false),
        ],
    );
    rule(&mut f, "news@example.test");
    let text = rows(
        &f,
        &EmailListQuery {
            text: Some("Subject".into()),
            ..grouped()
        },
    );
    assert_eq!(text.len(), 3);
    assert!(text.iter().all(|item| item.sender_group.is_none()));
    let sender = rows(
        &f,
        &EmailListQuery {
            from_address: Some("news@example.test".into()),
            ..grouped()
        },
    );
    assert_eq!(uids(&f, &sender), vec![2, 1]);
    assert!(sender.iter().all(|item| item.sender_group.is_none()));
}

#[test]
fn grouped_pages_are_stable_across_equal_timestamps() {
    let mut f = setup();
    // Interleave two aggregated senders and plain mail, many on one timestamp.
    let mut records = vec![];
    for uid in 1..=60u32 {
        let (address, time) = match uid % 3 {
            0 => ("a@example.test", T),
            1 => ("b@example.test", T),
            _ => ("plain@example.test", T),
        };
        records.push(from(uid, address, time, uid % 2 == 0));
    }
    for uid in 61..=70u32 {
        records.push(from(
            uid,
            "a@example.test",
            "2026-10-19T09:00:00.000Z",
            false,
        ));
    }
    put(&mut f, &records);
    rule(&mut f, "a@example.test");
    rule(&mut f, "b@example.test");
    let full = rows(
        &f,
        &EmailListQuery {
            limit: Some(200),
            ..grouped()
        },
    );
    // 20 plain rows + 2 group rows.
    assert_eq!(full.len(), 22);
    for size in [1, 2, 3, 7] {
        let mut query = EmailListQuery {
            limit: Some(size),
            ..grouped()
        };
        let mut seen = vec![];
        loop {
            let page = f.service.list_emails(&query).unwrap();
            seen.extend(page.items);
            match page.next_cursor {
                Some(cursor) => query.cursor = Some(cursor),
                None => break,
            }
        }
        assert_eq!(seen, full, "page size {size}");
    }
    let group_a = full
        .iter()
        .find_map(|item| {
            item.sender_group
                .as_ref()
                .filter(|g| g.address == "a@example.test")
        })
        .unwrap();
    assert_eq!((group_a.total, group_a.unread), (30, 20));
    assert!(
        full.windows(2).all(|pair| {
            (&pair[0].received_at, &pair[0].id) > (&pair[1].received_at, &pair[1].id)
        })
    );
}

#[test]
fn local_sent_copies_are_never_grouped() {
    let mut f = setup();
    let local = local_copy(&f, 1, "2026-10-18T10:00:00.000Z", "mine", 0);
    let own = f.account.email_address.clone();
    rule(&mut f, &own);
    let items = rows(
        &f,
        &EmailListQuery {
            group_senders: true,
            ..sent_query(&f)
        },
    );
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, local);
    assert!(items[0].sender_group.is_none());
}

#[test]
fn select_emails_leaves_out_grouped_senders() {
    let mut f = setup();
    put(
        &mut f,
        &[
            from(1, "news@example.test", "2026-10-18T08:00:00.000Z", false),
            from(2, "friend@example.test", "2026-10-18T09:00:00.000Z", false),
            from(3, "news@example.test", "2026-10-18T10:00:00.000Z", false),
        ],
    );
    rule(&mut f, "news@example.test");
    let selected = f.service.select_emails(&grouped()).unwrap();
    assert_eq!(
        selected
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        vec![message_id(&f, 2)]
    );
    // Without grouping every message is selectable; in a sender view too.
    assert_eq!(
        f.service.select_emails(&inbox_query()).unwrap().items.len(),
        3
    );
    let sender = f
        .service
        .select_emails(&EmailListQuery {
            from_address: Some("news@example.test".into()),
            ..grouped()
        })
        .unwrap();
    assert_eq!(sender.items.len(), 2);
}

#[test]
fn rules_are_idempotent_validated_and_listed_with_visible_counts() {
    let mut f = setup();
    put(
        &mut f,
        &[
            from(1, "zed@example.test", "2026-10-18T08:00:00.000Z", false),
            from(2, "Zed@example.test", "2026-10-18T09:00:00.000Z", true),
            from(3, "amy@example.test", "2026-10-18T10:00:00.000Z", true),
        ],
    );
    f.service
        .set_contact_aggregation(" Zed@Example.test ", true)
        .unwrap();
    f.service
        .set_contact_aggregation("zed@example.test", true)
        .unwrap();
    f.service
        .set_contact_aggregation("quiet@example.test", true)
        .unwrap();
    f.service
        .set_contact_aggregation("amy@example.test", true)
        .unwrap();
    assert_eq!(count(&f.db, "contact_aggregations"), 3);
    let list = f.service.list_contact_aggregations().unwrap();
    // Ordered by name (newest message's name, else the address), case-insensitively:
    // "Name 2" < "Name 3" < "quiet@...".
    assert_eq!(
        list.iter().map(|c| c.address.as_str()).collect::<Vec<_>>(),
        vec!["zed@example.test", "amy@example.test", "quiet@example.test"]
    );
    let zed = &list[0];
    assert_eq!(zed.name.as_deref(), Some("Name 2"));
    assert_eq!(zed.message_count, 2);
    assert_eq!(
        zed.latest_received_at.as_deref(),
        Some("2026-10-18T09:00:00.000Z")
    );
    let quiet = &list[2];
    assert_eq!(
        (
            quiet.name.clone(),
            quiet.message_count,
            quiet.latest_received_at.clone()
        ),
        (None, 0, None)
    );
    // Deleted messages no longer count.
    f.db.execute(
        "UPDATE messages SET remote_state = 'deleted' WHERE uid = 2",
        [],
    )
    .unwrap();
    assert_eq!(
        f.service.list_contact_aggregations().unwrap()[0].message_count,
        1
    );

    // Removal is idempotent too.
    f.service
        .set_contact_aggregation("ZED@example.test", false)
        .unwrap();
    f.service
        .set_contact_aggregation("zed@example.test", false)
        .unwrap();
    assert_eq!(f.service.list_contact_aggregations().unwrap().len(), 2);
    for bad in ["", "nope", "a@b\0", &format!("{}@x", "a".repeat(320))] {
        let error = f.service.set_contact_aggregation(bad, true).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation, "{bad:?}");
        assert_eq!(error.field.as_deref(), Some("address"), "{bad:?}");
    }
}
