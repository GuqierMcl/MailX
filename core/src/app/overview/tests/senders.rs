//! `newMail.senders` and `unreadSenders`: the overview's sender groups.
use super::*;

type From = (&'static str, Option<&'static str>);

fn mail_from(f: &Fixture, minutes_ago: i64, from: From, read: bool) -> At {
    At {
        time: f.now - Duration::minutes(minutes_ago),
        from,
        read,
        ..at_time(f.now)
    }
}

fn sender(address: &str, name: Option<&str>, count: u32) -> OverviewSender {
    OverviewSender {
        address: address.into(),
        name: name.map(Into::into),
        count,
    }
}

#[test]
fn new_mail_senders_group_by_address_in_the_scope_of_the_total() {
    let mut f = setup();
    let mails = [
        mail_from(&f, 5, ("News@Example.test", Some("News")), false),
        mail_from(&f, 10, ("news@example.test", Some("The News")), true),
        mail_from(&f, 20, ("NEWS@example.test", None), false),
        mail_from(&f, 7, ("ann@example.test", Some("Ann")), false),
        mail_from(&f, 8, ("ann@example.test", Some("Ann B")), false),
        mail_from(&f, 9, ("bob@example.test", None), false),
        // At `since` or older: not new.
        mail_from(&f, 60, ("old@example.test", Some("Old")), false),
        mail_from(&f, 90, ("old@example.test", Some("Old")), false),
    ];
    f.put_at(0, "INBOX", &mails);
    let bob = mail_from(&f, 1, ("bob@example.test", Some("Bob")), false);
    f.put_at(1, "INBOX", &[bob]);
    // Out of scope: other roles and locally archived mail.
    let junk = mail_from(&f, 2, ("junk@example.test", None), false);
    f.put_at(0, "Junk", &[junk]);
    let sent = mail_from(&f, 2, ("sent@example.test", None), false);
    f.put_at(0, "Sent", &[sent]);
    let archived = mail_from(&f, 3, ("ann@example.test", Some("Ann")), false);
    let archived = f.put_at(0, "INBOX", &[archived]);
    f.service.archive_messages(&archived).unwrap();

    let overview = f.overview(Some(60));
    let senders = &overview.new_mail.senders;
    // Read mail counts too: it is new mail. Ties go to the most recent sender.
    assert_eq!(
        senders,
        &[
            sender("news@example.test", Some("News"), 3),
            sender("bob@example.test", Some("Bob"), 2),
            sender("ann@example.test", Some("Ann"), 2),
        ]
    );
    // The listed counts add up to the total: same scope.
    let listed: u32 = senders.iter().map(|sender| sender.count).sum();
    assert_eq!(listed, overview.new_mail.total);

    // No `since`: no senders.
    assert!(f.overview(None).new_mail.senders.is_empty());
}

#[test]
fn new_mail_senders_stop_at_five_and_follow_visibility() {
    let mut f = setup();
    let addresses = ["a", "b", "c", "d", "e", "f", "g"]
        .map(|name| -> &'static str { Box::leak(format!("{name}@example.test").into_boxed_str()) });
    let mails: Vec<At> = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| mail_from(&f, index as i64 + 1, (address, None), false))
        .collect();
    f.put_at(0, "INBOX", &mails);
    let senders = f.overview(Some(60)).new_mail.senders;
    assert_eq!(senders.len(), OVERVIEW_SENDERS as usize);
    // One message each: the most recent first.
    assert_eq!(senders[0].address, "a@example.test");
    assert_eq!(senders[4].address, "e@example.test");

    // A new UIDVALIDITY hides the old rows here as it does for the total.
    f.db.execute(
        "UPDATE folder_sync_cursors SET uid_validity = 11 WHERE folder_id = ?1",
        [&f.targets[&(0, "INBOX")].folder_id],
    )
    .unwrap();
    let overview = f.overview(Some(60));
    assert_eq!(overview.new_mail.total, 0);
    assert!(overview.new_mail.senders.is_empty());
}

#[test]
fn unread_senders_need_three_unread_and_leave_out_archived_mail() {
    let mut f = setup();
    let unread = |f: &Fixture, minutes_ago, from| mail_from(f, minutes_ago, from, false);
    let read = |f: &Fixture, minutes_ago, from| mail_from(f, minutes_ago, from, true);
    let work = [
        unread(&f, 600, ("Shop@Example.test", Some("Shop"))),
        unread(&f, 500, ("shop@example.test", Some("The Shop"))),
        unread(&f, 400, ("SHOP@example.test", None)),
        unread(&f, 300, ("shop@example.test", Some("The Shop"))),
        // Three unread and one read: counts three.
        unread(&f, 200, ("tie@example.test", Some("Tie"))),
        unread(&f, 201, ("tie@example.test", Some("Tie"))),
        unread(&f, 202, ("tie@example.test", Some("Tie"))),
        read(&f, 1, ("tie@example.test", Some("Tie"))),
        // Two unread: below the threshold.
        unread(&f, 10, ("few@example.test", None)),
        unread(&f, 11, ("few@example.test", None)),
        // Three unread, one archived on this device: two count.
        unread(&f, 12, ("arch@example.test", None)),
        unread(&f, 13, ("arch@example.test", None)),
        unread(&f, 14, ("arch@example.test", None)),
        // All read.
        read(&f, 15, ("done@example.test", None)),
        read(&f, 16, ("done@example.test", None)),
        read(&f, 17, ("done@example.test", None)),
    ];
    let ids = f.put_at(0, "INBOX", &work);
    f.service.archive_messages(&[ids[12].clone()]).unwrap();
    // The same senders in another account, and unread mail in other folders.
    let home = [
        unread(&f, 1, ("shop@example.test", Some("Shop"))),
        unread(&f, 2, ("tie@example.test", Some("Tie"))),
    ];
    f.put_at(1, "INBOX", &home);
    let junk = [
        unread(&f, 1, ("junk@example.test", None)),
        unread(&f, 2, ("junk@example.test", None)),
        unread(&f, 3, ("junk@example.test", None)),
    ];
    f.put_at(0, "Junk", &junk);

    // Without `since` the new-mail section is empty; unread senders do not depend on it.
    let senders = f.overview(None).unread_senders;
    assert_eq!(
        senders,
        // shop: 4 + 1 across accounts, the name of the newest named mail.
        vec![
            sender("shop@example.test", Some("Shop"), 5),
            sender("tie@example.test", Some("Tie"), 4),
        ]
    );
}

#[test]
fn unread_senders_list_five_by_count_then_recency() {
    let mut f = setup();
    let mut mails = Vec::new();
    for (index, (address, count)) in [
        ("a@example.test", 3),
        ("b@example.test", 6),
        ("c@example.test", 3),
        ("d@example.test", 4),
        ("e@example.test", 3),
        ("f@example.test", 3),
        ("g@example.test", 3),
    ]
    .into_iter()
    .enumerate()
    {
        for n in 0..count {
            mails.push(mail_from(
                &f,
                100 + index as i64 * 3 + n,
                (address, None),
                false,
            ));
        }
    }
    f.put_at(0, "INBOX", &mails);
    let senders = f.overview(None).unread_senders;
    let listed: Vec<(&str, u32)> = senders
        .iter()
        .map(|sender| (sender.address.as_str(), sender.count))
        .collect();
    assert_eq!(
        listed,
        [
            ("b@example.test", 6),
            ("d@example.test", 4),
            ("a@example.test", 3),
            ("c@example.test", 3),
            ("e@example.test", 3),
        ]
    );
}
