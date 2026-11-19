//! `contacts_suggest` (sending.md section 17.4) over the overview fixture:
//! accounts `work@` and `home@example.test`, each with INBOX, Junk, Trash
//! and Sent.

use super::*;
use crate::mail::ContactSuggestion;

fn suggest(f: &Fixture, query: &str) -> Vec<ContactSuggestion> {
    f.service.mail_contacts(query, None).unwrap()
}

fn addresses(items: &[ContactSuggestion]) -> Vec<&str> {
    items.iter().map(|item| item.address.as_str()).collect()
}

fn from(address: &'static str, name: Option<&'static str>, time: DateTime<Utc>) -> At {
    At {
        from: (address, name),
        ..at_time(time)
    }
}

fn sent_to(to: &'static [(&'static str, Option<&'static str>)], time: DateTime<Utc>) -> At {
    At {
        from: ("work@example.test", Some("Me")),
        to,
        ..at_time(time)
    }
}

fn hours(n: i64) -> DateTime<Utc> {
    day(0) + Duration::hours(n)
}

#[test]
fn matches_address_domain_name_words_and_cjk_names() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            from("alice@example.test", Some("Alice Liu"), hours(1)),
            from("bob.stone@corp.test", Some("Robert Stone"), hours(2)),
            from("zhang@example.test", Some("张三丰"), hours(3)),
            from("Mixed@Case.Test", Some("Ölaf"), hours(4)),
        ],
    );
    assert_eq!(addresses(&suggest(&f, "ali")), ["alice@example.test"]);
    assert_eq!(addresses(&suggest(&f, "  ALI ")), ["alice@example.test"]);
    // Domain start, not the middle of the address.
    assert_eq!(addresses(&suggest(&f, "corp")), ["bob.stone@corp.test"]);
    assert!(suggest(&f, "orp").is_empty());
    assert!(suggest(&f, "stone@").is_empty());
    // Any word of the name.
    assert_eq!(addresses(&suggest(&f, "liu")), ["alice@example.test"]);
    assert_eq!(addresses(&suggest(&f, "rob")), ["bob.stone@corp.test"]);
    assert_eq!(addresses(&suggest(&f, "stone")), ["bob.stone@corp.test"]);
    assert!(suggest(&f, "obert").is_empty());
    // CJK names match anywhere in the name.
    assert_eq!(addresses(&suggest(&f, "三丰")), ["zhang@example.test"]);
    // Stored addresses are lowercased; non-ASCII case folds too.
    assert_eq!(
        suggest(&f, "mix"),
        [ContactSuggestion {
            name: Some("Ölaf".into()),
            address: "mixed@case.test".into()
        }]
    );
    assert_eq!(addresses(&suggest(&f, "öla")), ["mixed@case.test"]);
    assert_eq!(addresses(&suggest(&f, "ÖLA")), ["mixed@case.test"]);
    assert!(suggest(&f, "nobody").is_empty());
}

#[test]
fn the_sql_prefilter_keeps_every_exact_match() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            // Whitespace other than a plain space separates words too.
            from("tab@example.test", Some("Alice\tLiu"), hours(1)),
            from("nbsp@example.test", Some("Kim\u{a0}Park"), hours(2)),
            from("under@example.test", Some("foo_bar"), hours(3)),
            // A name with CJK text matches anywhere, ASCII included.
            from("mixed@example.test", Some("张Bobby"), hours(4)),
            // LIKE wildcards in the query are literal.
            from("a_b@example.test", None, hours(5)),
            from("axb@example.test", None, hours(6)),
        ],
    );
    assert_eq!(addresses(&suggest(&f, "liu")), ["tab@example.test"]);
    assert_eq!(addresses(&suggest(&f, "park")), ["nbsp@example.test"]);
    assert_eq!(addresses(&suggest(&f, "bar")), ["under@example.test"]);
    assert_eq!(addresses(&suggest(&f, "obb")), ["mixed@example.test"]);
    assert_eq!(addresses(&suggest(&f, "a_")), ["a_b@example.test"]);
    assert!(suggest(&f, "a%").is_empty());
}

#[test]
fn leaves_out_own_addresses_no_reply_junk_and_trash() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            from("home@example.test", Some("Me at home"), hours(1)),
            from("noreply@shop.test", Some("Shop"), hours(2)),
            from("Billing-No-Reply@shop.test", Some("Shop"), hours(3)),
            from("donotreply@shop.test", Some("Shop"), hours(4)),
            from("do-not-reply@shop.test", Some("Shop"), hours(5)),
            from("MAILER-DAEMON@shop.test", None, hours(6)),
            from("postmaster@shop.test", None, hours(7)),
            from("support@shop.test", Some("Shop"), hours(8)),
        ],
    );
    f.put_at(0, "Junk", &[from("spam@shop.test", Some("Shop"), hours(9))]);
    f.put_at(
        1,
        "Trash",
        &[from("gone@shop.test", Some("Shop"), hours(10))],
    );
    // Sent to your own other account and to a no-reply address.
    f.put_at(
        0,
        "Sent",
        &[sent_to(
            &[("home@example.test", None), ("noreply@shop.test", None)],
            hours(11),
        )],
    );
    assert_eq!(addresses(&suggest(&f, "shop")), ["support@shop.test"]);
    assert!(suggest(&f, "home").is_empty());
    assert!(suggest(&f, "work").is_empty());
    // Junk does not count, but the same address in the inbox does.
    f.put_at(0, "INBOX", &[from("spam@shop.test", None, hours(12))]);
    assert_eq!(
        addresses(&suggest(&f, "shop")),
        ["spam@shop.test", "support@shop.test"]
    );
}

#[test]
fn ranks_people_written_to_first_then_weighted_count_recency_and_address() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            // Received only: never before anyone written to.
            from("many@team.test", None, hours(30)),
            from("many@team.test", None, hours(31)),
            from("many@team.test", None, hours(32)),
            from("many@team.test", None, hours(33)),
            from("many@team.test", None, hours(34)),
            from("old@team.test", None, hours(1)),
            from("old@team.test", None, hours(2)),
            from("new@team.test", None, hours(3)),
            from("new@team.test", None, hours(4)),
            // One received on top of one sent: 3 + 1.
            from("both@team.test", None, hours(5)),
        ],
    );
    f.put_at(
        0,
        "Sent",
        &[
            sent_to(&[("both@team.test", None)], hours(6)),
            // Same weight 3: newer first, then by address.
            sent_to(&[("recent@team.test", None)], hours(20)),
            sent_to(&[("bea@team.test", None)], hours(10)),
            sent_to(&[("abe@team.test", None)], hours(10)),
        ],
    );
    // Listed in To and Cc of one message: still one message.
    f.put_at(
        1,
        "Sent",
        &[At {
            from: ("home@example.test", None),
            to: &[("zed@team.test", None)],
            cc: &["ZED@team.test"],
            ..at_time(hours(10))
        }],
    );
    // A local Sent copy (Bcc included) counts as written to.
    f.local_copy(1, hours(8), "copy-1@example.test", &["Copy@team.test"]);
    assert_eq!(
        addresses(&suggest(&f, "team")),
        [
            "both@team.test",
            "recent@team.test",
            "abe@team.test",
            "bea@team.test",
            "zed@team.test",
            "copy@team.test",
            "many@team.test",
            "new@team.test",
        ]
    );
    let all = f.service.mail_contacts("team", Some(20)).unwrap();
    assert_eq!(all.len(), 9);
    assert_eq!(all[8].address, "old@team.test");
    assert_eq!(
        addresses(&f.service.mail_contacts("team", Some(2)).unwrap()),
        ["both@team.test", "recent@team.test"]
    );
}

#[test]
fn the_name_is_the_newest_non_empty_display_name_from_either_side() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            from("alice@example.test", Some("Alice"), hours(1)),
            from("alice@example.test", Some("Alice Liu"), hours(3)),
            // Newer but empty or blank: ignored.
            from("alice@example.test", None, hours(5)),
            from("alice@example.test", Some("   "), hours(6)),
            from("bob@example.test", Some("Bob"), hours(1)),
            from("nameless@example.test", None, hours(1)),
        ],
    );
    f.put_at(
        0,
        "Sent",
        &[
            // Older than Alice's newest name: loses.
            sent_to(&[("alice@example.test", Some("A. Liu"))], hours(2)),
            // Newer than Bob's received name: wins, trimmed.
            sent_to(&[("bob@example.test", Some("  Bobby Tables "))], hours(4)),
        ],
    );
    let names: Vec<_> = suggest(&f, "example")
        .into_iter()
        .map(|item| (item.address, item.name))
        .collect();
    assert_eq!(
        names,
        [
            (
                "alice@example.test".to_owned(),
                Some("Alice Liu".to_owned())
            ),
            (
                "bob@example.test".to_owned(),
                Some("Bobby Tables".to_owned())
            ),
            ("nameless@example.test".to_owned(), None),
        ]
    );
    // The name that matches is the one shown: the older name does not.
    assert_eq!(addresses(&suggest(&f, "bobby")), ["bob@example.test"]);
    assert!(suggest(&f, "a. l").is_empty());
}

#[test]
fn validates_query_and_limit() {
    let mut f = setup();
    let many: Vec<At> = (0..10)
        .map(|index| {
            let address: &'static str =
                Box::leak(format!("person{index:02}@many.test").into_boxed_str());
            from(address, None, hours(index))
        })
        .collect();
    f.put_at(0, "INBOX", &many);
    assert_eq!(suggest(&f, "many").len(), 8);
    assert_eq!(f.service.mail_contacts("many", Some(20)).unwrap().len(), 10);
    assert_eq!(f.service.mail_contacts("many", Some(1)).unwrap().len(), 1);
    for limit in [0, 21, u32::MAX] {
        let error = f.service.mail_contacts("many", Some(limit)).unwrap_err();
        assert_eq!(error.code, crate::ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some("limit"));
    }
    for empty in ["", "   ", "\t\n"] {
        assert!(suggest(&f, empty).is_empty());
    }
    assert!(suggest(&f, &"a".repeat(320)).is_empty());
    assert!(suggest(&f, &format!("  {}  ", "a".repeat(320))).is_empty());
    let error = f.service.mail_contacts(&"a".repeat(321), None).unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::Validation);
    assert_eq!(error.field.as_deref(), Some("query"));
    // LIKE wildcards and quotes are plain text.
    assert!(suggest(&f, "%").is_empty());
    assert!(suggest(&f, "_").is_empty());
    assert!(suggest(&f, "'").is_empty());
}

/// Seeds 90,000 inbox messages from 5,000 senders and 10,000 sent messages
/// to 2,000 recipients in account 0 and times a few typical queries. The
/// rows are copied in SQL from one stored template per folder, since storing
/// them one batch at a time spends most of the time threading. Run with
/// `cargo test -p mailx-core --release --lib -- --ignored contact_suggestions_timing --nocapture`.
#[test]
#[ignore = "timing run, not a correctness test"]
fn contact_suggestions_timing() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[from("template@example.test", Some("T"), hours(0))],
    );
    f.put_at(
        0,
        "Sent",
        &[sent_to(&[("template@example.test", None)], hours(0))],
    );
    let columns: Vec<String> =
        f.db.prepare("SELECT name FROM pragma_table_info('messages')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
    for (folder, count, from, name, to) in [
        (
            "INBOX",
            90_000,
            "char(97 + (n % 5000) * 7 % 26, 97 + (n % 5000) * 11 % 23, 97 + (n % 5000) * 3 % 19)
               || (n % 5000) || '@' || char(97 + (n % 200) % 26, 97 + (n % 200) * 5 % 26) || 'host.test'",
            "char(65 + (n % 5000) * 3 % 26) || 'ane ' || char(65 + (n % 5000) * 17 % 26) || 'mith' || (n % 5000)",
            "m.to_addresses",
        ),
        (
            "Sent",
            10_000,
            "m.from_address",
            "m.from_name",
            "json_array(json_object('name', 'Recipient ' || (n % 2000), 'address',
               'recipient' || (n % 2000) || '@domain' || (n % 50) || '.test'))",
        ),
    ] {
        let target = f.targets[&(0, folder)].clone();
        let values: Vec<String> = columns
            .iter()
            .map(|column| match column.as_str() {
                "id" => "lower(hex(randomblob(16)))".into(),
                "uid" => "n + 1".into(),
                "message_id_header" => "n || '@bench.test'".into(),
                "from_address" => from.into(),
                "from_name" => name.into(),
                "to_addresses" => to.into(),
                "received_at" => {
                    "strftime('%Y-%m-%dT%H:%M:%fZ', '2026-01-01', '+' || n || ' seconds')".into()
                }
                other => format!("m.{other}"),
            })
            .collect();
        f.db.execute(
            &format!(
                "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < ?2)
                 INSERT INTO messages ({}) SELECT {} FROM s, messages m WHERE m.folder_id = ?1 AND m.uid = 1",
                columns.join(", "),
                values.join(", "),
            ),
            params![target.folder_id, count],
        )
        .unwrap();
    }
    let total: u32 =
        f.db.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
    println!("{total} messages");
    if let Some(path) = std::env::var_os("MAILX_BENCH_COPY") {
        f.db.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
    }
    for query in [
        "s",
        "sm",
        "smith12",
        "a",
        "ab",
        "jane",
        "kh",
        "recipient1",
        "domain4",
        "zzz",
    ] {
        suggest(&f, query);
        let runs = 5;
        let started = std::time::Instant::now();
        let mut found = 0;
        for _ in 0..runs {
            found = suggest(&f, query).len();
        }
        println!(
            "{query:>12}: {:?} per query, {found} results",
            started.elapsed() / runs
        );
    }
}

#[test]
fn scoped_contacts_do_not_use_other_account_names_sent_rank_or_own_address_exclusions() {
    let mut f = setup();
    f.put_at(
        0,
        "INBOX",
        &[
            from("alice@team.test", Some("Alice local"), hours(1)),
            from("shared@team.test", Some("Local name"), hours(2)),
            from(
                "home@example.test",
                Some("Other account as a correspondent"),
                hours(3),
            ),
        ],
    );
    f.put_at(
        1,
        "INBOX",
        &[from("shared@team.test", Some("Private name"), hours(10))],
    );
    f.put_at(
        1,
        "Sent",
        &[sent_to(
            &[("hidden@team.test", Some("Private recipient"))],
            hours(11),
        )],
    );
    let scope = [f.accounts[0].id.clone()];
    let results = f
        .service
        .suggest_contacts_scoped("team", Some(1), Some(&scope))
        .unwrap();
    assert_eq!(addresses(&results), ["shared@team.test"]);
    assert_eq!(results[0].name.as_deref(), Some("Local name"));
    assert_eq!(
        addresses(
            &f.service
                .suggest_contacts_scoped("home", None, Some(&scope))
                .unwrap()
        ),
        ["home@example.test"]
    );
    assert!(
        f.service
            .suggest_contacts_scoped("team", None, Some(&[]))
            .unwrap()
            .is_empty()
    );
    assert!(
        f.service
            .suggest_contacts_scoped("private", None, Some(&scope))
            .unwrap()
            .is_empty()
    );
}
