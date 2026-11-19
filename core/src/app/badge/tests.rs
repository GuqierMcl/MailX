use rusqlite::Connection;
use tempfile::TempDir;

use super::*;
use crate::MailChangeSource;

fn folder(account: &str, role: FolderRole, unread: u32, known: bool) -> Folder {
    Folder {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account.into(),
        parent_id: None,
        name: "INBOX".into(),
        role,
        selectable: true,
        counts_known: known,
        unread_count: unread,
        total_count: unread,
        archived_count: 0,
        archived_unread_count: 0,
    }
}

#[test]
fn sums_known_inboxes_of_included_accounts_only() {
    let folders = [
        folder("a", FolderRole::Inbox, 3, true),
        folder("b", FolderRole::Inbox, 4, true),
        // Unknown counts are left out rather than guessed.
        folder("c", FolderRole::Inbox, 9, false),
        folder("a", FolderRole::Junk, 5, true),
        folder("a", FolderRole::Other, 6, true),
        Folder {
            selectable: false,
            ..folder("b", FolderRole::Inbox, 7, true)
        },
    ];
    let mut settings = BadgeSettings::default();
    assert_eq!(count_unread(&folders, &settings), 7);
    settings.excluded_accounts = vec!["b".into(), "gone".into()];
    assert_eq!(count_unread(&folders, &settings), 3);
    settings.enabled = false;
    assert_eq!(count_unread(&folders, &settings), 0);
    let huge = [
        folder("a", FolderRole::Inbox, u32::MAX, true),
        folder("b", FolderRole::Inbox, 1, true),
    ];
    assert_eq!(count_unread(&huge, &BadgeSettings::default()), u32::MAX);
}

#[tokio::test]
async fn follows_the_sidebar_number_through_reading_archiving_and_deleting() {
    let directory = TempDir::new().unwrap();
    let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nbody";
    let (core, ids) = MailxCore::body_test_fixture(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[raw, raw, raw, raw],
    )
    .unwrap();
    let on = BadgeSettings::default();
    let account = core.account_identities().unwrap()[0].id.clone();
    // What the sidebar reads for the unified inbox.
    let sidebar = || {
        core.accounts()
            .unwrap()
            .all_folders()
            .unwrap()
            .iter()
            .find(|folder| folder.role == FolderRole::Inbox)
            .unwrap()
            .unread_count
    };
    assert_eq!(core.badge_count(&on).unwrap(), 4);

    core.mail_flags(&ids[..1], Some(true), None, MailChangeSource::User)
        .unwrap();
    assert_eq!(core.badge_count(&on).unwrap(), 3);

    core.mail_archive(&ids[1..2]).unwrap();
    assert_eq!(core.badge_count(&on).unwrap(), 2);

    let deletion = core.delete_emails(&ids[2..3], true).unwrap();
    assert_eq!(core.badge_count(&on).unwrap(), 1);
    assert_eq!(core.badge_count(&on).unwrap(), sidebar());
    core.undo_delete(&deletion.batch_id).unwrap();
    assert_eq!(core.badge_count(&on).unwrap(), 2);
    assert_eq!(core.badge_count(&on).unwrap(), sidebar());

    let excluded = BadgeSettings {
        excluded_accounts: vec![account.clone()],
        ..BadgeSettings::default()
    };
    assert_eq!(core.badge_count(&excluded).unwrap(), 0);
    let off = BadgeSettings {
        enabled: false,
        ..BadgeSettings::default()
    };
    assert_eq!(core.badge_count(&off).unwrap(), 0);

    // An inbox whose counts were never confirmed by the server is skipped.
    Connection::open(directory.path().join("mailx.sqlite3"))
        .unwrap()
        .execute("UPDATE folders SET synced_at = ''", [])
        .unwrap();
    assert_eq!(core.badge_count(&on).unwrap(), 0);
}
