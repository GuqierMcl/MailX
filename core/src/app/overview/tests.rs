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
        overview_at(&self.service, since.as_deref(), self.now).unwrap()
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
    let error = validate(&MailOverviewQuery {
        since: Some("yesterday".into()),
    })
    .unwrap_err();
    assert_eq!(error, AppError::validation("since"));
    assert_eq!(
        validate(&MailOverviewQuery {
            since: Some("2026-11-14T08:00:00+08:00".into()),
        })
        .unwrap()
        .as_deref(),
        Some("2026-11-14T00:00:00.000Z")
    );
    let f = setup();
    let empty = f.overview(Some(60));
    assert_eq!(empty.generated_at, "2026-11-14T12:00:00.000Z");
    assert_eq!(empty.starred_unread, 0);
    assert!(empty.verification_codes.is_empty() && empty.recent_attachments.is_empty());
}
