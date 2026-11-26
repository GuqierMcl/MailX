//! Also hosts the recipient suggestion tests, which need the same accounts,
//! folders and local Sent copies.
mod contact_suggestions;
mod senders;

use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::{
    accounts::{AccountService, messages::SyncTarget, sync_plan::tests::config, types::Account},
    imap::FolderStatus,
    mail::FolderRole,
    secrets::test_support::MemorySecrets,
    sync::{folders::FolderRecord, headers::MessageRecord},
};

const ROLES: [(&str, FolderRole); 4] = [
    ("INBOX", FolderRole::Inbox),
    ("Junk", FolderRole::Junk),
    ("Trash", FolderRole::Trash),
    ("Sent", FolderRole::Sent),
];

struct Fixture {
    service: AccountService,
    db: Connection,
    _directory: TempDir,
    accounts: Vec<Account>,
    /// (account index, remote folder name) to its sync target.
    targets: HashMap<(usize, &'static str), SyncTarget>,
    now: DateTime<Utc>,
}

fn folder(name: &str, role: FolderRole) -> FolderRecord {
    FolderRecord {
        remote_name: name.into(),
        parent_remote_name: None,
        name: name.into(),
        delimiter: Some("/".into()),
        role,
        selectable: true,
        attributes: vec![],
        status: Some(FolderStatus {
            messages: 0,
            unseen: 0,
            uid_next: Some(1),
            uid_validity: Some(10),
        }),
    }
}

fn setup() -> Fixture {
    let directory = TempDir::new().unwrap();
    let mut service =
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut accounts = Vec::new();
    let mut targets = HashMap::new();
    for (index, address) in ["work@example.test", "home@example.test"]
        .iter()
        .enumerate()
    {
        let mut config = config(json!({"kind": "direct"}));
        config["emailAddress"] = (*address).into();
        let account = service
            .create(
                serde_json::from_value(
                    json!({"requestId": Uuid::new_v4().to_string(), "config": config}),
                )
                .unwrap(),
            )
            .unwrap();
        let records: Vec<_> = ROLES
            .iter()
            .map(|(name, role)| folder(name, *role))
            .collect();
        service
            .store_folders(&account.id, &account.mailbox_source_id, &records)
            .unwrap();
        for (folder_id, remote) in service
            .sync_folders(&account.id, &account.mailbox_source_id)
            .unwrap()
        {
            let name = ROLES.iter().find(|(name, _)| *name == remote).unwrap().0;
            let target = SyncTarget {
                account_id: account.id.clone(),
                mailbox_source_id: account.mailbox_source_id.clone(),
                folder_id,
                uid_validity: 10,
            };
            service
                .ensure_generation(&target, "2026-01-01")
                .unwrap()
                .unwrap();
            targets.insert((index, name), target);
        }
        accounts.push(account);
    }
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    Fixture {
        service,
        db,
        _directory: directory,
        accounts,
        targets,
        now: DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    }
}

fn at(now: DateTime<Utc>, minutes_ago: i64) -> String {
    (now - Duration::minutes(minutes_ago)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

struct Mail {
    subject: &'static str,
    minutes_ago: i64,
    read: bool,
    starred: bool,
    attachment: bool,
}

const fn mail(subject: &'static str, minutes_ago: i64) -> Mail {
    Mail {
        subject,
        minutes_ago,
        read: false,
        starred: false,
        attachment: false,
    }
}

impl Fixture {
    /// Stores `mails` in one folder (uids from 1) and returns their local ids.
    fn put(&mut self, account: usize, folder: &'static str, mails: &[Mail]) -> Vec<String> {
        let target = self.targets[&(account, folder)].clone();
        let records: Vec<MessageRecord> = mails
            .iter()
            .enumerate()
            .map(|(index, mail)| MessageRecord {
                uid: index as u32 + 1,
                message_id_header: Some(format!("{account}-{folder}-{index}@example.test")),
                original_message_id: None,
                sent_identity_policy: crate::providers::for_imap_host("").policy_key(),
                in_reply_to: None,
                references_header: None,
                subject: mail.subject.into(),
                from_name: Some("Sender".into()),
                from_address: "sender@example.test".into(),
                to_addresses: vec![],
                cc_addresses: vec![],
                reply_to_addresses: vec![],
                received_at: at(self.now, mail.minutes_ago),
                sent_at: None,
                size: Some(400),
                is_read: mail.read,
                is_flagged: mail.starred,
                flags: vec![],
                has_attachment: mail.attachment,
            })
            .collect();
        self.service
            .store_message_batch(&target, &records, records.len() as u32)
            .unwrap()
            .unwrap();
        records
            .iter()
            .map(|record| {
                self.db
                    .query_row(
                        "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                        params![target.folder_id, record.uid],
                        |row| row.get(0),
                    )
                    .unwrap()
            })
            .collect()
    }

    fn set_preview(&self, id: &str, preview: &str) {
        self.db
            .execute(
                "UPDATE messages SET preview = ?2, preview_state = 'ready' WHERE id = ?1",
                params![id, preview],
            )
            .unwrap();
    }

    fn cache_body(&self, id: &str, plain_text: &str, files: &[(&str, bool)]) {
        self.db
            .execute(
                "INSERT INTO message_bodies(message_id, raw_sha256, raw_size, plain_text,
                 sanitized_html, sanitizer_version, links, remote_images_blocked, truncated, fetched_at)
                 VALUES (?1, ?2, 20, ?3, '<p>x</p>', 1, '[]', 0, 0, '2026-11-14')",
                params![id, "a".repeat(64), plain_text],
            )
            .unwrap();
        for (index, (name, inline)) in files.iter().enumerate() {
            self.db
                .execute(
                    "INSERT INTO message_parts(message_id, part_index, content_id, filename, mime_type, size, is_inline)
                     VALUES (?1, ?2, NULL, ?3, 'application/pdf', 10, ?4)",
                    params![id, index as i64 + 1, name, inline],
                )
                .unwrap();
        }
        self.db
            .execute(
                "UPDATE messages SET body_state = 'ready' WHERE id = ?1",
                [id],
            )
            .unwrap();
    }

    fn overview(&self, since_minutes_ago: Option<i64>) -> MailOverview {
        let since = since_minutes_ago.map(|minutes| at(self.now, minutes));
        overview_at(&self.service, since.as_deref(), None, self.now).unwrap()
    }
}

#[test]
fn new_mail_counts_inbox_mail_after_since_per_account() {
    let mut f = setup();
    let work = f.put(
        0,
        "INBOX",
        &[
            mail("w1", 5),
            mail("w2", 10),
            mail("w3", 30),
            // Exactly at `since` is not new.
            mail("w-boundary", 60),
            mail("w-old", 120),
        ],
    );
    let home = f.put(1, "INBOX", &[mail("h1", 1), mail("h-old", 90)]);
    f.put(0, "Junk", &[mail("junk", 2)]);
    f.put(0, "Trash", &[mail("trash", 2)]);
    f.put(1, "Sent", &[mail("sent", 2)]);
    let archived = f.put(
        1,
        "INBOX",
        &[mail("h1", 1), mail("h-old", 90), mail("h-archived", 3)],
    );
    assert_eq!(archived[..2], home[..]);
    f.service.archive_messages(&[archived[2].clone()]).unwrap();

    let overview = f.overview(Some(60));
    let new_mail = &overview.new_mail;
    assert_eq!(new_mail.total, 4);
    let mut expected = vec![
        AccountMailCount {
            account_id: f.accounts[0].id.clone(),
            count: 3,
        },
        AccountMailCount {
            account_id: f.accounts[1].id.clone(),
            count: 1,
        },
    ];
    expected.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    assert_eq!(new_mail.accounts, expected);
    let latest: Vec<&str> = new_mail.latest.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(latest, [&home[0], &work[0], &work[1], &work[2]]);

    // `since` in the past beyond everything: limited to five, newest first.
    let all = f.overview(Some(24 * 60));
    assert_eq!(all.new_mail.total, 7);
    assert_eq!(all.new_mail.latest.len(), OVERVIEW_LATEST as usize);
    assert_eq!(all.new_mail.latest[0].id, home[0]);

    // No `since`: no new-mail section at all.
    assert_eq!(f.overview(None).new_mail, NewMailOverview::default());
}

#[test]
fn other_sources_and_generations_are_invisible() {
    let mut f = setup();
    f.put(0, "INBOX", &[mail("work", 5)]);
    let home = f.put(1, "INBOX", &[mail("home", 5)]);
    f.cache_body(&home[0], "Your verification code is 482913.", &[]);
    assert_eq!(f.overview(Some(60)).new_mail.total, 2);

    // The work inbox moved to a new UIDVALIDITY: its old rows are hidden.
    f.db.execute(
        "UPDATE folder_sync_cursors SET uid_validity = 11 WHERE folder_id = ?1",
        [&f.targets[&(0, "INBOX")].folder_id],
    )
    .unwrap();
    let overview = f.overview(Some(60));
    assert_eq!(overview.new_mail.total, 1);
    assert_eq!(overview.verification_codes.len(), 1);

    // The home account switched mailbox source: nothing of it remains.
    f.db.execute(
        "UPDATE accounts SET mailbox_source_id = 'new-source' WHERE id = ?1",
        [&f.accounts[1].id],
    )
    .unwrap();
    let overview = f.overview(Some(60));
    assert_eq!(overview.new_mail, NewMailOverview::default());
    assert!(overview.verification_codes.is_empty());
}

#[test]
fn starred_unread_follows_the_starred_view_scope() {
    let mut f = setup();
    let starred = |subject, read| Mail {
        read,
        starred: true,
        ..mail(subject, 600)
    };
    let inbox = f.put(
        0,
        "INBOX",
        &[
            starred("a", false),
            starred("read", true),
            mail("plain", 1),
            starred("archived", false),
        ],
    );
    f.service.archive_messages(&[inbox[3].clone()]).unwrap();
    f.put(1, "Sent", &[starred("sent", false)]);
    f.put(0, "Junk", &[starred("junk", false)]);
    f.put(1, "Trash", &[starred("trash", false)]);
    // Inbox "a", archived inbox mail and the Sent copy.
    assert_eq!(f.overview(None).starred_unread, 3);
}

#[test]
fn verification_codes_come_from_previews_and_cached_bodies_only() {
    let mut f = setup();
    let ids = f.put(
        0,
        "INBOX",
        &[
            mail("Sign-in", 1),
            mail("Your verification code", 2),
            mail("Two codes", 3),
            mail("Body not cached", 4),
            mail("Old code", 16),
            mail("Archived", 5),
        ],
    );
    // From the cached body, which wins over the preview.
    f.set_preview(&ids[0], "Nothing here");
    f.cache_body(&ids[0], "Your verification code is 482913.", &[]);
    // From the preview of a message whose body is not downloaded.
    f.set_preview(&ids[1], "Your verification code is 7391.");
    // Ambiguous: skipped.
    f.set_preview(&ids[2], "Your verification code is 123456 or 654321.");
    // No preview and no body: nothing to read, nothing downloaded.
    // Outside the fifteen-minute window.
    f.set_preview(&ids[4], "Your verification code is 555123.");
    f.set_preview(&ids[5], "Your verification code is 909090.");
    f.service.archive_messages(&[ids[5].clone()]).unwrap();
    let junk = f.put(0, "Junk", &[mail("Junk", 1)]);
    f.set_preview(&junk[0], "Your verification code is 111222.");

    let codes = f.overview(None).verification_codes;
    let found: Vec<(&str, &str)> = codes
        .iter()
        .map(|c| (c.email_id.as_str(), c.code.as_str()))
        .collect();
    assert_eq!(
        found,
        [(ids[0].as_str(), "482913"), (ids[1].as_str(), "7391")]
    );
    assert_eq!(codes[0].account_id, f.accounts[0].id);
    assert_eq!(codes[0].folder_id, f.targets[&(0, "INBOX")].folder_id);
    assert_eq!(codes[0].from.address, "sender@example.test");
    assert_eq!(codes[0].received_at, at(f.now, 1));
    assert_eq!(codes[0].body_state, BodyState::Ready);
    assert_eq!(codes[1].body_state, BodyState::Missing);
    assert!(!format!("{:?}", codes[0]).contains("482913"));
}

#[test]
fn verification_codes_skip_oversized_bodies_and_stop_at_three() {
    let mut f = setup();
    let ids = f.put(
        1,
        "INBOX",
        &[
            mail("a", 1),
            mail("b", 2),
            mail("c", 3),
            mail("d", 4),
            mail("e", 5),
        ],
    );
    let oversized = format!(
        "Your verification code is 482913.\n{}",
        "x".repeat(verification_code::MAX_TEXT_BYTES)
    );
    f.cache_body(&ids[0], &oversized, &[]);
    for (index, id) in ids.iter().enumerate().skip(1) {
        f.set_preview(id, &format!("Your verification code is 10000{index}."));
    }
    let codes: Vec<String> = f
        .overview(None)
        .verification_codes
        .into_iter()
        .map(|c| c.code)
        .collect();
    assert_eq!(codes, ["100001", "100002", "100003"]);
}

#[test]
fn code_candidates_are_recent_verification_subjects_without_a_code_yet() {
    let mut f = setup();
    let ids = f.put(
        0,
        "INBOX",
        &[
            // Has a code from its preview: a code, not a candidate.
            mail("Your verification code", 1),
            // Nothing stored yet: a candidate.
            mail("Your verification code", 2),
            // Not a verification subject.
            mail("Weekly newsletter", 3),
            mail("Sign in to Example", 4),
            // Outside the window.
            mail("Your verification code", 16),
            // Archived.
            mail("Your verification code", 5),
            // A reply is never a candidate.
            mail("Re: Your verification code", 6),
        ],
    );
    f.set_preview(&ids[0], "Your verification code is 482913.");
    f.service.archive_messages(&[ids[5].clone()]).unwrap();
    f.put(0, "Junk", &[mail("Your verification code", 1)]);
    let home = f.put(1, "INBOX", &[mail("Verify your email", 1)]);

    let overview = f.overview(None);
    assert_eq!(overview.verification_codes.len(), 1);
    let found: Vec<&str> = overview
        .code_candidates
        .iter()
        .map(|c| c.email_id.as_str())
        .collect();
    // Newest first, across accounts.
    assert_eq!(found, [home[0].as_str(), ids[1].as_str(), ids[3].as_str()]);
    let first = &overview.code_candidates[0];
    assert_eq!(first.account_id, f.accounts[1].id);
    assert_eq!(first.folder_id, f.targets[&(1, "INBOX")].folder_id);
    assert_eq!(first.from.address, "sender@example.test");
    assert_eq!(first.received_at, at(f.now, 1));
    assert_eq!(first.body_state, BodyState::Missing);
}

#[test]
fn code_candidates_stop_at_five() {
    let mut f = setup();
    let mails: Vec<Mail> = (1..=8).map(|n| mail("Your verification code", n)).collect();
    f.put(0, "INBOX", &mails);
    assert_eq!(
        f.overview(None).code_candidates.len(),
        OVERVIEW_CODE_CANDIDATES
    );
}

#[test]
fn recent_attachments_list_known_names_newest_first() {
    let mut f = setup();
    let with = |subject, minutes_ago| Mail {
        attachment: true,
        ..mail(subject, minutes_ago)
    };
    let days = |n: i64| n * 24 * 60;
    let ids = f.put(
        0,
        "INBOX",
        &[
            with("cached", 10),
            with("not cached", 20),
            mail("no attachment", 5),
            with("too old", days(7) + 1),
            with("archived", 30),
        ],
    );
    f.cache_body(
        &ids[0],
        "Report attached",
        &[("报告.pdf", false), ("logo.png", true), ("data.csv", false)],
    );
    f.service.archive_messages(&[ids[4].clone()]).unwrap();
    f.put(1, "Junk", &[with("junk", 1)]);
    let home = f.put(
        1,
        "INBOX",
        &[
            with("h1", days(1)),
            with("h2", days(2)),
            with("h3", days(3)),
            with("h4", days(4)),
        ],
    );

    let attachments = f.overview(None).recent_attachments;
    let listed: Vec<(&str, Vec<&str>)> = attachments
        .iter()
        .map(|a| {
            (
                a.email.id.as_str(),
                a.file_names.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (ids[0].as_str(), vec!["报告.pdf", "data.csv"]),
            (ids[1].as_str(), vec![]),
            (home[0].as_str(), vec![]),
            (home[1].as_str(), vec![]),
            (home[2].as_str(), vec![]),
        ]
    );
    assert!(attachments.iter().all(|a| a.email.has_attachment));
}

#[test]
fn query_rejects_unknown_fields_and_invalid_since() {
    assert!(
        serde_json::from_value::<MailOverviewQuery>(json!({"since": null, "now": "x"})).is_err()
    );
    assert_eq!(
        serde_json::from_value::<MailOverviewQuery>(json!({})).unwrap(),
        MailOverviewQuery::default()
    );
    let error = validate(
        &MailOverviewQuery {
            since: Some("yesterday".into()),
            ..Default::default()
        },
        Utc::now(),
    )
    .unwrap_err();
    assert_eq!(error, AppError::validation("since"));
    assert_eq!(
        validate(
            &MailOverviewQuery {
                since: Some("2026-11-14T08:00:00+08:00".into()),
                ..Default::default()
            },
            Utc::now(),
        )
        .unwrap()
        .0
        .as_deref(),
        Some("2026-11-14T00:00:00.000Z")
    );
    let f = setup();
    let empty = f.overview(Some(60));
    assert_eq!(empty.generated_at, "2026-11-14T12:00:00.000Z");
    assert_eq!(empty.starred_unread, 0);
    assert!(empty.verification_codes.is_empty() && empty.recent_attachments.is_empty());
    assert_eq!(empty.activity, None);
}

/// A message for the activity tests, at an exact time.
struct At {
    time: DateTime<Utc>,
    from: (&'static str, Option<&'static str>),
    to: &'static [(&'static str, Option<&'static str>)],
    cc: &'static [&'static str],
    read: bool,
    message_id: Option<&'static str>,
}

fn at_time(time: DateTime<Utc>) -> At {
    At {
        time,
        from: ("sender@example.test", Some("Sender")),
        to: &[],
        cc: &[],
        read: false,
        message_id: None,
    }
}

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Local midnights in UTC+8: day 13 starts at 2026-11-13T16:00Z, four hours
/// before the fixture's `now`.
fn day(index: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-31T16:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
        + Duration::days(index)
}

fn day_starts() -> Vec<String> {
    (0..14).map(|index| stamp(day(index))).collect()
}

impl Fixture {
    /// Stores messages after the folder's existing uids; returns their ids.
    fn put_at(&mut self, account: usize, folder: &'static str, mails: &[At]) -> Vec<String> {
        let target = self.targets[&(account, folder)].clone();
        let first: u32 = self
            .db
            .query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE folder_id = ?1",
                [&target.folder_id],
                |row| row.get(0),
            )
            .unwrap();
        let address = |(address, name): &(&str, Option<&str>)| crate::mail::MailAddress {
            name: name.map(Into::into),
            address: (*address).into(),
        };
        let records: Vec<MessageRecord> = mails
            .iter()
            .enumerate()
            .map(|(index, mail)| {
                let uid = first + index as u32;
                MessageRecord {
                    uid,
                    message_id_header: Some(mail.message_id.map_or_else(
                        || format!("{account}-{folder}-{uid}@example.test"),
                        Into::into,
                    )),
                    original_message_id: None,
                    sent_identity_policy: crate::providers::for_imap_host("").policy_key(),
                    in_reply_to: None,
                    references_header: None,
                    subject: "Activity".into(),
                    from_name: mail.from.1.map(Into::into),
                    from_address: mail.from.0.into(),
                    to_addresses: mail.to.iter().map(address).collect(),
                    cc_addresses: mail.cc.iter().map(|cc| address(&(*cc, None))).collect(),
                    reply_to_addresses: vec![],
                    received_at: stamp(mail.time),
                    sent_at: None,
                    size: Some(400),
                    is_read: mail.read,
                    is_flagged: false,
                    flags: vec![],
                    has_attachment: false,
                }
            })
            .collect();
        let highest = first + records.len() as u32;
        self.service
            .store_message_batch(&target, &records, highest)
            .unwrap()
            .unwrap();
        records
            .iter()
            .map(|record| {
                self.db
                    .query_row(
                        "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                        params![target.folder_id, record.uid],
                        |row| row.get(0),
                    )
                    .unwrap()
            })
            .collect()
    }

    /// An accepted send of `account` waiting in the Sent view as a local copy
    /// until a server copy with `message_id` arrives.
    fn local_copy(&self, account: usize, time: DateTime<Utc>, message_id: &str, to: &[&str]) {
        let envelope = json!({
            "from": self.accounts[account].email_address,
            "recipients": to,
        });
        self.db
            .execute(
                "INSERT INTO outbox(id, account_id, client_mutation_id, queue_canonical, message_id,
                 snapshot_hash, snapshot_size, envelope, subject, recipients_preview, recipient_count,
                 state, accepted_at, not_before, first_queued_at, created_at, updated_at, preview,
                 attachment_count)
                 VALUES (?1, ?2, ?3, '{}', ?4, ?5, 10, ?6, 'Sent copy', ?7, ?8, 'accepted',
                         ?9, ?9, ?9, ?9, ?9, 'body', 0)",
                params![
                    Uuid::new_v4().to_string(),
                    self.accounts[account].id,
                    Uuid::new_v4().to_string(),
                    format!("<{message_id}>"),
                    "b".repeat(64),
                    envelope.to_string(),
                    serde_json::to_string(to).unwrap(),
                    to.len() as i64,
                    stamp(time),
                ],
            )
            .unwrap();
    }

    /// Marks every inbox and Sent generation as fully synced.
    fn finish_all(&mut self) {
        for ((_, name), target) in self.targets.clone() {
            if matches!(name, "INBOX" | "Sent") {
                self.service
                    .finish_backfill(&target, false)
                    .unwrap()
                    .unwrap();
            }
        }
    }

    fn activity(&self) -> OverviewActivity {
        overview_at(&self.service, None, Some(&day_starts()), self.now)
            .unwrap()
            .activity
            .unwrap()
    }
}

#[test]
fn activity_counts_inbox_mail_per_local_day() {
    let mut f = setup();
    let ms = Duration::milliseconds(1);
    let ids = f.put_at(
        0,
        "INBOX",
        &[
            // Before the first day: not counted.
            at_time(day(0) - ms),
            // Exactly at a day start: that day.
            at_time(day(0)),
            at_time(day(7) - ms),
            at_time(day(7)),
            at_time(day(13)),
            // Read today.
            At {
                read: true,
                ..at_time(day(13) + Duration::hours(1))
            },
            // Exactly now: today.
            at_time(f.now),
            // Archived today: a day count, not today's inbox.
            at_time(day(13) + Duration::hours(2)),
            // Archived earlier: still received that day.
            at_time(day(3)),
            // In the future: not received yet.
            at_time(f.now + ms),
        ],
    );
    f.service
        .archive_messages(&[ids[7].clone(), ids[8].clone()])
        .unwrap();
    f.put_at(1, "INBOX", &[at_time(day(13) + Duration::minutes(5))]);
    for folder in ["Junk", "Trash", "Sent"] {
        f.put_at(1, folder, &[at_time(day(13)), at_time(day(10))]);
    }

    let activity = f.activity();
    let days: Vec<u32> = activity.days.iter().map(|d| d.received).collect();
    assert_eq!(days, [1, 0, 0, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 5]);
    assert_eq!(activity.days[13].start, "2026-11-13T16:00:00.000Z");
    assert_eq!(activity.received_previous_week, 3);
    assert_eq!(activity.received_last_week, 6);
    // Today: four unarchived inbox messages, one of them read.
    assert_eq!(
        activity.today,
        TodayActivity {
            received: 4,
            unread: 3
        }
    );
    // The sent messages above count only as sent.
    assert_eq!(
        (activity.sent_last_week, activity.sent_previous_week),
        (2, 0)
    );
}

#[test]
fn sent_counts_follow_the_sent_view_and_count_matched_copies_once() {
    let mut f = setup();
    // Waiting for its server copy: counted as the local copy.
    f.local_copy(0, day(12), "pending@example.test", &["a@example.test"]);
    // Matched once its server copy is stored: counted as the server copy.
    f.local_copy(0, day(11), "matched@example.test", &["b@example.test"]);
    // Before the two weeks.
    f.local_copy(
        1,
        day(0) - Duration::hours(1),
        "old@example.test",
        &["c@example.test"],
    );
    f.put_at(
        0,
        "Sent",
        &[
            At {
                message_id: Some("matched@example.test"),
                ..at_time(day(11) + Duration::minutes(1))
            },
            at_time(day(2)),
            at_time(day(6) + Duration::hours(23)),
        ],
    );
    let matched: Option<String> =
        f.db.query_row(
            "SELECT server_copy_message_id FROM outbox WHERE message_id = '<matched@example.test>'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(matched.is_some());
    f.put_at(1, "Sent", &[at_time(day(13))]);
    // An inbox message from the same Message-ID is not a sent message.
    f.put_at(
        0,
        "INBOX",
        &[At {
            message_id: Some("matched@example.test"),
            ..at_time(day(11))
        }],
    );

    let activity = f.activity();
    assert_eq!(activity.sent_last_week, 3);
    assert_eq!(activity.sent_previous_week, 2);
}

#[test]
fn previous_week_is_complete_only_when_every_folder_is_synced_far_enough() {
    let mut f = setup();
    // Never finished a round.
    assert!(!f.activity().previous_week_complete);
    f.finish_all();
    assert!(f.activity().previous_week_complete);

    // The next round in progress keeps the history complete.
    let inbox = f.targets[&(1, "INBOX")].clone();
    f.service.begin_backfill(&inbox, false).unwrap().unwrap();
    assert!(f.activity().previous_week_complete);
    f.service.finish_backfill(&inbox, false).unwrap().unwrap();
    assert!(f.activity().previous_week_complete);

    // A capped round, whether just begun or finished, clears it until an
    // uncapped round finishes again.
    f.service.begin_backfill(&inbox, true).unwrap().unwrap();
    assert!(!f.activity().previous_week_complete);
    f.service.begin_backfill(&inbox, false).unwrap().unwrap();
    assert!(!f.activity().previous_week_complete);
    f.service.finish_backfill(&inbox, false).unwrap().unwrap();
    assert!(f.activity().previous_week_complete);
    f.service.finish_backfill(&inbox, true).unwrap().unwrap();
    assert!(!f.activity().previous_week_complete);
    f.service.finish_backfill(&inbox, false).unwrap().unwrap();
    assert!(f.activity().previous_week_complete);

    // A new generation is incomplete until its first round finishes, even
    // while that round runs.
    let regenerated = SyncTarget {
        uid_validity: 11,
        ..inbox.clone()
    };
    f.service
        .ensure_generation(&regenerated, "2026-01-01")
        .unwrap()
        .unwrap();
    assert!(!f.activity().previous_week_complete);
    f.service
        .begin_backfill(&regenerated, false)
        .unwrap()
        .unwrap();
    assert!(!f.activity().previous_week_complete);
    f.service
        .finish_backfill(&regenerated, false)
        .unwrap()
        .unwrap();
    assert!(f.activity().previous_week_complete);
    // A capped Junk folder does not matter.
    let junk = f.targets[&(1, "Junk")].clone();
    f.service.finish_backfill(&junk, true).unwrap().unwrap();
    assert!(f.activity().previous_week_complete);

    // A sync window starting too late: the first day starts 2026-10-31T16:00Z,
    // so the window must start by 2026-10-30.
    let sent = f.targets[&(0, "Sent")].folder_id.clone();
    let window = |date: &str| {
        f.db.execute(
            "UPDATE folder_sync_cursors SET window_start = ?2 WHERE folder_id = ?1",
            params![sent, date],
        )
        .unwrap();
    };
    window("2026-10-31");
    assert!(!f.activity().previous_week_complete);
    window("2026-10-30");
    assert!(f.activity().previous_week_complete);

    // A Sent folder that never synced (no generation at all).
    f.db.execute(
        "DELETE FROM folder_sync_cursors WHERE folder_id = ?1",
        [&sent],
    )
    .unwrap();
    assert!(!f.activity().previous_week_complete);
}

#[test]
fn top_contacts_are_people_you_write_to() {
    let mut f = setup();
    let last = |hours: i64| day(7) + Duration::hours(hours);
    f.put_at(
        0,
        "INBOX",
        &[
            // Alice: three received, one archived, mixed case.
            At {
                from: ("Alice@Example.test", Some("Alice")),
                ..at_time(last(1))
            },
            At {
                from: ("alice@example.test", Some("Alice Liu")),
                ..at_time(last(30))
            },
            At {
                from: ("ALICE@example.test", None),
                ..at_time(last(40))
            },
            // Before the last seven days: not counted.
            At {
                from: ("alice@example.test", Some("Old Alice")),
                ..at_time(day(6))
            },
            // A newsletter you never wrote to.
            At {
                from: ("news@example.test", Some("News")),
                ..at_time(last(2))
            },
            At {
                from: ("news@example.test", Some("News")),
                ..at_time(last(3))
            },
            At {
                from: ("news@example.test", Some("News")),
                ..at_time(last(4))
            },
            At {
                from: ("news@example.test", Some("News")),
                ..at_time(last(5))
            },
            // Your other account.
            At {
                from: ("home@example.test", Some("Me")),
                ..at_time(last(6))
            },
            At {
                from: ("home@example.test", Some("Me")),
                ..at_time(last(7))
            },
            // Carol: written to only long ago, so she qualifies.
            At {
                from: ("carol@example.test", Some("Carol")),
                ..at_time(last(8))
            },
            // Dave: one received, newer than Carol's.
            At {
                from: ("dave@example.test", Some("Dave")),
                ..at_time(last(9))
            },
        ],
    );
    let archived = f.put_at(
        0,
        "INBOX",
        &[At {
            from: ("alice@example.test", Some("Alice")),
            ..at_time(last(45))
        }],
    );
    f.service.archive_messages(&archived).unwrap();
    f.put_at(
        0,
        "Sent",
        &[
            // To Alice (named) and Bob, Cc Alice again: Alice counts once.
            At {
                from: ("work@example.test", None),
                to: &[
                    ("alice@example.test", Some("A. Liu")),
                    ("bob@example.test", None),
                ],
                cc: &["ALICE@example.test", "home@example.test"],
                ..at_time(last(20))
            },
            // Carol, two weeks ago.
            At {
                from: ("work@example.test", None),
                to: &[("carol@example.test", None)],
                ..at_time(day(1))
            },
            // Dave, long ago.
            At {
                from: ("work@example.test", None),
                to: &[("dave@example.test", None)],
                ..at_time(day(0) - Duration::days(10))
            },
        ],
    );
    // Bob twice more from a local copy (Bcc included in the envelope).
    f.local_copy(1, last(50), "bob-1@example.test", &["Bob@example.test"]);
    f.local_copy(1, last(51), "bob-2@example.test", &["bob@example.test"]);

    let contacts = f.activity().top_contacts;
    assert_eq!(
        contacts,
        [
            // 4 received (one archived) + 1 sent; the newest name is from
            // the archived message, newer than the sent one.
            OverviewContact {
                address: "alice@example.test".into(),
                name: Some("Alice".into()),
                count: 5
            },
            // 3 sent, no name known.
            OverviewContact {
                address: "bob@example.test".into(),
                name: None,
                count: 3
            },
            // Dave and Carol both have 1: Dave's message is newer.
            OverviewContact {
                address: "dave@example.test".into(),
                name: Some("Dave".into()),
                count: 1
            },
        ]
    );
}

#[test]
fn day_starts_are_validated() {
    let now = DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let check = |starts: Vec<String>| {
        validate(
            &MailOverviewQuery {
                since: None,
                day_starts: Some(starts),
            },
            now,
        )
    };
    let error = Err(AppError::validation("dayStarts"));
    let valid = day_starts();
    assert_eq!(check(valid.clone()).unwrap().1, Some(valid.clone()));
    // Any offset is accepted and normalized.
    let mut offset = valid.clone();
    offset[13] = "2026-11-14T00:00:00+08:00".into();
    assert_eq!(check(offset).unwrap().1, Some(valid.clone()));
    // Count.
    assert_eq!(check(valid[1..].to_vec()), error);
    let mut more = valid.clone();
    more.push(stamp(now));
    assert_eq!(check(more), error);
    assert_eq!(check(vec![]), error);
    // Order: equal or descending starts.
    let mut equal = valid.clone();
    equal[5] = equal[4].clone();
    assert_eq!(check(equal), error);
    let mut reversed = valid.clone();
    reversed.reverse();
    assert_eq!(check(reversed), error);
    // Invalid.
    let mut invalid = valid.clone();
    invalid[3] = "yesterday".into();
    assert_eq!(check(invalid), error);
    // The last start may be now but not later.
    let mut current = valid.clone();
    current[13] = stamp(now);
    assert!(check(current).is_ok());
    let mut future = valid;
    future[13] = stamp(now + Duration::milliseconds(1));
    assert_eq!(check(future), error);
    // Omitted: no activity, and the field is optional on the wire.
    assert_eq!(
        serde_json::from_value::<MailOverviewQuery>(json!({"since": null}))
            .unwrap()
            .day_starts,
        None
    );
}
