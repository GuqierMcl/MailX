//! Embedded, ordered database migrations.
//!
//! Migration SQL is part of the binary. Once a migration has shipped, its SQL
//! and checksum are immutable; schema changes are added as a new numbered entry.

use crate::{AppError, ErrorCode};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Migration {
    pub version: u32,
    pub description: &'static str,
    /// Objects that must exist once this migration is recorded. This keeps
    /// the history check from treating an unrelated table as a valid schema.
    pub required_tables: &'static [&'static str],
    pub sql: &'static str,
}

/// The production migration manifest. Keep entries ordered and append-only.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "account configuration baseline",
        required_tables: &[
            "schema_migrations",
            "accounts",
            "auth_profiles",
            "mail_connections",
            "network_routes",
            "create_requests",
            "credential_journal",
        ],
        sql: include_str!("migrations/0001_account_configuration.sql"),
    },
    Migration {
        version: 2,
        description: "folder list cache",
        required_tables: &["folders"],
        sql: include_str!("migrations/0002_folders.sql"),
    },
    Migration {
        version: 3,
        description: "message header cache and folder synchronization cursors",
        required_tables: &["folder_sync_cursors", "messages"],
        sql: include_str!("migrations/0003_messages.sql"),
    },
    Migration {
        version: 4,
        description: "message body and attachment metadata cache",
        required_tables: &["message_bodies", "message_parts"],
        sql: include_str!("migrations/0004_message_bodies.sql"),
    },
    Migration {
        version: 5,
        description: "account display color",
        required_tables: &["accounts"],
        sql: include_str!("migrations/0005_account_color.sql"),
    },
    Migration {
        version: 6,
        description: "pending read and starred changes",
        required_tables: &["flag_changes"],
        sql: include_str!("migrations/0006_flag_changes.sql"),
    },
    Migration {
        version: 7,
        description: "local drafts and account sender name",
        required_tables: &["drafts"],
        sql: include_str!("migrations/0007_drafts.sql"),
    },
    Migration {
        version: 8,
        description: "send queue and local sent copies",
        required_tables: &["outbox"],
        sql: include_str!("migrations/0008_outbox.sql"),
    },
    Migration {
        version: 9,
        description: "authoritative rich-text draft body",
        required_tables: &["drafts"],
        sql: include_str!("migrations/0009_draft_body_html.sql"),
    },
    Migration {
        version: 10,
        description: "explicit image proxy DNS policy",
        required_tables: &["network_routes"],
        sql: include_str!("migrations/0010_image_proxy_dns.sql"),
    },
    Migration {
        version: 11,
        description: "recoverable local account removal",
        required_tables: &["account_removals", "removed_create_requests", "secret_gc"],
        sql: include_str!("migrations/0011_account_removal.sql"),
    },
    Migration {
        version: 12,
        description: "account provider icon preference",
        required_tables: &["accounts"],
        sql: include_str!("migrations/0012_account_provider_icon.sql"),
    },
    Migration {
        version: 13,
        description: "compose draft attachments",
        required_tables: &["draft_attachments"],
        sql: include_str!("migrations/0013_draft_attachments.sql"),
    },
    Migration {
        version: 14,
        description: "send queue snapshots stored as blobs",
        required_tables: &["outbox"],
        sql: include_str!("migrations/0014_outbox_snapshot_blob.sql"),
    },
    Migration {
        version: 15,
        description: "account SMTP size limit",
        required_tables: &["accounts"],
        sql: include_str!("migrations/0015_account_smtp_size_limit.sql"),
    },
    Migration {
        version: 16,
        description: "message list preview state",
        required_tables: &["messages"],
        sql: include_str!("migrations/0016_preview_state.sql"),
    },
    Migration {
        version: 17,
        description: "inline image content ids",
        required_tables: &["draft_attachments"],
        sql: include_str!("migrations/0017_inline_image_content_id.sql"),
    },
    Migration {
        version: 18,
        description: "outbox sent copy list fields",
        required_tables: &["outbox"],
        sql: include_str!("migrations/0018_outbox_sent_copy_list.sql"),
    },
    Migration {
        version: 19,
        description: "local full-text search index",
        required_tables: &["search_fts", "search_docs", "search_index_state"],
        sql: include_str!("migrations/0019_search_index.sql"),
    },
    Migration {
        version: 20,
        description: "scheduled message deletions",
        required_tables: &["message_deletions"],
        sql: include_str!("migrations/0020_message_deletions.sql"),
    },
    Migration {
        version: 21,
        description: "local message archive",
        required_tables: &["message_archives"],
        sql: include_str!("migrations/0021_message_archives.sql"),
    },
    Migration {
        version: 22,
        description: "draft origin",
        required_tables: &[],
        sql: include_str!("migrations/0022_draft_origin.sql"),
    },
    Migration {
        version: 23,
        description: "attachment origin for agents",
        required_tables: &["draft_attachments"],
        sql: include_str!("migrations/0023_attachment_origin.sql"),
    },
    Migration {
        version: 24,
        description: "provider-neutral sent copy identity",
        required_tables: &["messages"],
        sql: include_str!("migrations/0024_sent_copy_identity.sql"),
    },
    Migration {
        version: 25,
        description: "OAuth grant bindings and authorization-aware send admission",
        required_tables: &["auth_profiles", "oauth_client_registrations"],
        sql: include_str!("migrations/0025_oauth_bindings.sql"),
    },
    Migration {
        version: 26,
        description: "contact aggregation rules and sender lookup index",
        required_tables: &["contact_aggregations"],
        sql: include_str!("migrations/0026_contact_aggregations.sql"),
    },
    Migration {
        version: 27,
        description: "conversation threading headers and conversation identity",
        required_tables: &["messages"],
        sql: include_str!("migrations/0027_conversation_threading.sql"),
    },
    Migration {
        version: 28,
        description: "rebuild conversation identity with threading version",
        required_tables: &["messages"],
        sql: include_str!("migrations/0028_conversation_rethread.sql"),
    },
    Migration {
        version: 29,
        description: "cached message translations",
        required_tables: &["message_translations"],
        sql: include_str!("migrations/0029_message_translations.sql"),
    },
];

pub(crate) fn checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

/// Prepare a database using an explicit migration manifest.
///
/// The manifest is intentionally supplied as an argument so the storage tests
/// can exercise multi-step and failing migrations without changing the
/// production list. Production callers use [`MIGRATIONS`].
pub(crate) fn apply(db: &mut Connection, migrations: &[Migration]) -> Result<(), AppError> {
    validate_manifest(migrations)?;

    // user_version is an SQLite integer. Reading it as u32 rejects negative
    // values and keeps all comparisons in the same domain as the manifest.
    let user_version: u32 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    let latest = migrations
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0);
    if user_version > latest {
        return Err(AppError::new(ErrorCode::StorageIncompatible));
    }

    let has_user_tables = has_user_tables(db)?;
    if user_version == 0 {
        // Only a genuinely empty SQLite file may be initialized. In
        // particular, never overwrite an unversioned user database.
        if has_user_tables {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        }
    } else {
        // A versioned database must expose a complete, verified history before
        // any new migration is attempted.
        if !has_user_tables || !history_table_exists(db)? {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        }
        verify_history(db, user_version, migrations)?;
        verify_required_tables(db, user_version, migrations)?;
    }

    // These connection settings are part of the storage contract and are set
    // before migration work starts. The directory lock held by Storage::open
    // prevents another MailX process from racing this change.
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "FULL")?;

    let applied = user_version;
    for migration in migrations
        .iter()
        .filter(|migration| migration.version > applied)
    {
        apply_one(db, *migration)?;
    }
    Ok(())
}

fn validate_manifest(migrations: &[Migration]) -> Result<(), AppError> {
    let valid = migrations.first().is_some_and(|first| first.version == 1)
        && migrations
            .iter()
            .all(|migration| !migration.description.trim().is_empty())
        && migrations
            .windows(2)
            .all(|pair| pair[1].version == pair[0].version.saturating_add(1));
    if valid {
        Ok(())
    } else {
        Err(AppError::new(ErrorCode::StorageIncompatible))
    }
}

fn has_user_tables(db: &Connection) -> Result<bool, AppError> {
    db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master
            WHERE type IN ('table', 'index', 'view', 'trigger')
              AND name NOT LIKE 'sqlite_%'
        )",
        [],
        |row| row.get(0),
    )
    .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))
}

fn history_table_exists(db: &Connection) -> Result<bool, AppError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
        [],
        |row| row.get(0),
    )
    .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))
}

fn verify_required_tables(
    db: &Connection,
    user_version: u32,
    migrations: &[Migration],
) -> Result<(), AppError> {
    for migration in migrations.iter().take(user_version as usize) {
        for table in migration.required_tables {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master
                        WHERE type = 'table' AND name = ?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
            if !exists {
                return Err(AppError::new(ErrorCode::StorageIncompatible));
            }
        }
    }
    Ok(())
}

fn verify_history(
    db: &Connection,
    user_version: u32,
    migrations: &[Migration],
) -> Result<(), AppError> {
    let mut statement = db
        .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;

    let mut history = Vec::new();
    for row in rows {
        history.push(row.map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?);
    }
    if history.len() != user_version as usize {
        return Err(AppError::new(ErrorCode::StorageIncompatible));
    }

    for (index, (version, saved_checksum)) in history.iter().enumerate() {
        let expected_version = (index as u32) + 1;
        let Some(migration) = migrations.get(index) else {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        };
        if *version != i64::from(expected_version)
            || migration.version != expected_version
            || saved_checksum != &checksum(migration.sql)
        {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        }
    }
    Ok(())
}

fn apply_one(db: &mut Connection, migration: Migration) -> Result<(), AppError> {
    // IMMEDIATE obtains the write reservation at transaction start, so the SQL
    // body, history row, and user_version commit or roll back together.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    execute_migration(&tx, migration)?;
    tx.commit()
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))
}

fn execute_migration(tx: &Transaction<'_>, migration: Migration) -> Result<(), AppError> {
    tx.execute_batch(migration.sql)
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    tx.execute(
        "INSERT INTO schema_migrations(version, checksum) VALUES (?1, ?2)",
        (migration.version, checksum(migration.sql)),
    )
    .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    tx.pragma_update(None, "user_version", migration.version)
        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    use tempfile::TempDir;

    fn open_db() -> (TempDir, Connection) {
        let directory = TempDir::new().unwrap();
        let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
        (directory, db)
    }

    const V1: &str = "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL) STRICT; CREATE TABLE one(value TEXT NOT NULL) STRICT;";
    const V2: &str = "ALTER TABLE one ADD COLUMN second TEXT;";
    const V3: &str = "CREATE TABLE three(value INTEGER NOT NULL) STRICT;";

    fn test_manifest() -> [Migration; 3] {
        [
            Migration {
                version: 1,
                description: "test baseline",
                required_tables: &["schema_migrations", "one"],
                sql: V1,
            },
            Migration {
                version: 2,
                description: "test second migration",
                required_tables: &[],
                sql: V2,
            },
            Migration {
                version: 3,
                description: "test third migration",
                required_tables: &[],
                sql: V3,
            },
        ]
    }

    #[test]
    fn new_database_runs_all_migrations_in_order() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &test_manifest()).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                .get::<_, u32>(0))
                .unwrap(),
            3
        );
        assert!(
            db.query_row(
                "SELECT 1 FROM pragma_table_info('one') WHERE name='second'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .is_ok()
        );
        apply(&mut db, &test_manifest()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                .get::<_, u32>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn failing_step_rolls_back_only_that_step() {
        let (_directory, mut db) = open_db();
        let manifest = [
            Migration {
                version: 1,
                description: "test baseline",
                required_tables: &["schema_migrations", "one"],
                sql: V1,
            },
            Migration {
                version: 2,
                description: "test failing migration",
                required_tables: &[],
                sql: "CREATE TABLE two(value TEXT); THIS IS INVALID;",
            },
            Migration {
                version: 3,
                description: "test third migration",
                required_tables: &[],
                sql: V3,
            },
        ];
        assert_eq!(
            apply(&mut db, &manifest).unwrap_err().code,
            ErrorCode::StorageUnavailable
        );
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                .get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert!(
            db.query_row(
                "SELECT name FROM sqlite_master WHERE name='two'",
                [],
                |row| row.get::<_, String>(0)
            )
            .optional()
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn history_must_be_a_verified_complete_prefix() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &test_manifest()).unwrap();
        db.execute("DELETE FROM schema_migrations WHERE version = 2", [])
            .unwrap();
        assert_eq!(
            apply(&mut db, &test_manifest()).unwrap_err().code,
            ErrorCode::StorageIncompatible
        );
    }

    #[test]
    fn original_v1_schema_bytes_remain_compatible() {
        let (_directory, mut db) = open_db();
        let original = include_str!("schema.sql");
        db.execute_batch(original).unwrap();
        db.execute(
            "INSERT INTO schema_migrations(version, checksum) VALUES (1, ?1)",
            [checksum(original)],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        // A v1 database is accepted and upgraded through every later migration.
        assert_eq!(
            db.query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            MIGRATIONS.len() as i64
        );
    }

    #[test]
    fn unversioned_schema_objects_are_not_overwritten() {
        let (_directory, mut db) = open_db();
        db.execute_batch("CREATE VIEW existing AS SELECT 1;")
            .unwrap();
        assert_eq!(
            apply(&mut db, MIGRATIONS).unwrap_err().code,
            ErrorCode::StorageIncompatible
        );
        assert!(
            db.query_row(
                "SELECT name FROM sqlite_master WHERE name='existing'",
                [],
                |row| row.get::<_, String>(0)
            )
            .is_ok()
        );
    }

    #[test]
    fn versioned_history_requires_production_schema_tables() {
        let (_directory, mut db) = open_db();
        db.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL) STRICT;
             CREATE TABLE unrelated(value TEXT) STRICT;
             INSERT INTO unrelated VALUES ('keep');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO schema_migrations(version, checksum) VALUES (1, ?1)",
            [checksum(MIGRATIONS[0].sql)],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();

        assert_eq!(
            apply(&mut db, MIGRATIONS).unwrap_err().code,
            ErrorCode::StorageIncompatible
        );
        assert_eq!(
            db.query_row("SELECT value FROM unrelated", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "keep"
        );
        assert!(
            db.query_row(
                "SELECT name FROM sqlite_master WHERE name='unrelated'",
                [],
                |row| row.get::<_, String>(0)
            )
            .is_ok()
        );
    }

    fn manifest_up_to(version: u32) -> Vec<Migration> {
        MIGRATIONS
            .iter()
            .copied()
            .filter(|migration| migration.version <= version)
            .collect()
    }

    #[test]
    fn drafts_migration_applies_to_a_fresh_database() {
        let (_directory, mut db) = open_db();
        apply(&mut db, MIGRATIONS).unwrap();
        assert_eq!(MIGRATIONS.last().unwrap().version, 29);
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('drafts')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for column in [
            "id",
            "account_id",
            "kind",
            "revision",
            "client_mutation_id",
            "origin",
        ] {
            assert!(columns.iter().any(|name| name == column), "{column}");
        }
        assert!(
            db.query_row(
                "SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'sender_name'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .is_ok()
        );
    }

    #[test]
    fn contact_aggregations_migration_adds_rules_table_and_sender_index() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(25)).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        db.execute(
            "INSERT INTO contact_aggregations(address, created_at) VALUES ('a@example.test', 't')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO contact_aggregations(address, created_at) VALUES ('a@example.test', 't')",
                [],
            )
            .is_err(),
            "one rule per address"
        );
        let index: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'messages_by_sender'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn drafts_migration_upgrades_a_v6_database_and_keeps_its_accounts() {
        let (_directory, mut db) = open_db();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        apply(&mut db, &manifest_up_to(6)).unwrap();
        db.execute(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.com', 's', 't', 't')",
            [],
        )
        .unwrap();

        apply(&mut db, MIGRATIONS).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT sender_name FROM accounts WHERE id = 'a'",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        let insert = |id: &str, mutation: &str, kind: &str| {
            db.execute(
                "INSERT INTO drafts(id, account_id, kind, to_addresses, cc_addresses, bcc_addresses,
                 subject, body_text, created_at, updated_at, client_mutation_id, create_canonical)
                 VALUES (?1, 'a', ?3, '[]', '[]', '[]', '', '', 't', 't', ?2, '{}')",
                [id, mutation, kind],
            )
        };
        let id = |n: char| n.to_string().repeat(36);
        insert(&id('1'), &id('a'), "new").unwrap();
        // Mutation ids are unique per account; kinds and revisions are checked.
        assert!(insert(&id('2'), &id('a'), "new").is_err());
        assert!(insert(&id('3'), &id('b'), "other").is_err());
        assert!(
            db.execute("UPDATE drafts SET revision = 0", []).is_err(),
            "revision starts at 1"
        );
        assert!(
            db.execute("UPDATE accounts SET sender_name = ''", [])
                .is_err(),
            "an empty name is stored as NULL"
        );
        db.execute("DELETE FROM accounts WHERE id = 'a'", [])
            .unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM drafts", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn outbox_migration_upgrades_a_v7_database_and_enforces_its_constraints() {
        let (_directory, mut db) = open_db();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        apply(&mut db, &manifest_up_to(7)).unwrap();
        db.execute(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.com', 's', 't', 't')",
            [],
        )
        .unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        let id = |n: char| n.to_string().repeat(36);
        let insert = |row: &str, mutation: &str, state: &str| {
            db.execute(
                "INSERT INTO outbox(id, account_id, client_mutation_id, queue_canonical, message_id,
                 mime, envelope, subject, recipients_preview, recipient_count, state, not_before,
                 first_queued_at, created_at, updated_at)
                 VALUES (?1, 'a', ?2, '{}', '<m@x>', x'00', '{}', '', '[]', 1, ?3, 't', 't', 't', 't')",
                [row, mutation, state],
            )
        };
        insert(&id('1'), &id('a'), "queued").unwrap();
        assert!(
            insert(&id('2'), &id('a'), "queued").is_err(),
            "unique per account"
        );
        assert!(
            insert(&id('3'), &id('b'), "sent").is_err(),
            "state is checked"
        );
        assert!(
            insert(&id('4'), &id('c'), "accepted").is_err(),
            "accepted needs accepted_at"
        );
        assert!(
            db.execute("UPDATE outbox SET data_started_at = 't'", [])
                .is_err(),
            "a queued item has not started its body"
        );
        db.execute("DELETE FROM accounts WHERE id = 'a'", [])
            .unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM outbox", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "the queue goes with its account"
        );
    }

    #[test]
    fn sent_copy_list_fields_are_nullable_and_the_migration_reruns() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(17)).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        for column in ["preview", "attachment_count"] {
            let notnull: i64 = db
                .query_row(
                    "SELECT \"notnull\" FROM pragma_table_info('outbox') WHERE name = ?1",
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(notnull, 0, "{column}");
        }
    }

    #[test]
    fn rich_drafts_migration_preserves_legacy_text_and_queued_mime() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(8)).unwrap();
        db.execute(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.com', 's', 't', 't')", [],
        ).unwrap();
        let draft = "d".repeat(36);
        db.execute(
            "INSERT INTO drafts(id, account_id, kind, to_addresses, cc_addresses, bcc_addresses,
             subject, body_text, created_at, updated_at, client_mutation_id, create_canonical)
             VALUES (?1, 'a', 'new', '[]', '[]', '[]', 'old', ?2, 't', 't', ?3, '{}')",
            [&draft, "<literal>\n\n第二行", &"m".repeat(36)],
        )
        .unwrap();
        let mime = b"Content-Type: text/plain; charset=utf-8\r\n\r\nlegacy snapshot\r\n";
        db.execute(
            "INSERT INTO outbox(id, draft_id, account_id, client_mutation_id, queue_canonical, message_id,
             mime, envelope, subject, recipients_preview, recipient_count, state, not_before,
             first_queued_at, created_at, updated_at)
             VALUES (?1, ?2, 'a', ?3, '{}', '<old@example.com>', ?4, '{}', 'old', '[]', 1, 'queued', 't', 't', 't', 't')",
            rusqlite::params!["q".repeat(36), draft, "n".repeat(36), mime.as_slice()],
        ).unwrap();

        apply(&mut db, MIGRATIONS).unwrap();
        let (text, html, revision): (String, Option<String>, u32) = db
            .query_row(
                "SELECT body_text, body_html, revision FROM drafts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(text, "<literal>\n\n第二行");
        assert_eq!(html, None);
        assert_eq!(revision, 1);
        assert_eq!(
            db.query_row("SELECT mime FROM outbox", [], |row| row
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            mime
        );
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            MIGRATIONS.last().unwrap().version
        );
    }

    #[test]
    fn image_proxy_dns_migration_keeps_existing_routes_restricted() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(9)).unwrap();
        db.execute(
            "INSERT INTO accounts(id,revision,display_name,email_address,mailbox_source_id,created_at,updated_at) VALUES ('a',1,'Work','work@example.com','s','t','t')",
            [],
        ).unwrap();
        db.execute(
            "INSERT INTO network_routes(account_id,revision,kind,proxy_kind,host,port,dns) VALUES ('a',1,'custom','socks5','127.0.0.1',1080,'proxy')",
            [],
        ).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        let (host, allowed): (String, bool) = db
            .query_row(
                "SELECT host,image_proxy_dns FROM network_routes WHERE account_id='a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(host, "127.0.0.1");
        assert!(!allowed);
        assert!(
            db.execute("UPDATE network_routes SET image_proxy_dns=2", [])
                .is_err()
        );
    }

    #[test]
    fn account_removal_migration_preserves_v10_accounts_and_creation_requests() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(10)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id,
             created_at, updated_at) VALUES ('a', 3, 'Keep', 'local@example.test', 'source', 't', 't');
             INSERT INTO create_requests(request_id, account_id, canonical_config)
             VALUES ('create', 'a', 'existing');",
        )
        .unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        assert_eq!(
            db.query_row("SELECT revision FROM accounts WHERE id = 'a'", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap(),
            3
        );
        assert_eq!(
            db.query_row(
                "SELECT canonical_config FROM create_requests WHERE request_id = 'create'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "existing"
        );
        for table in ["account_removals", "removed_create_requests", "secret_gc"] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, u32>(0))
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn preview_state_migration_backfills_downloaded_bodies_and_constrains_values() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(15)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id,
             created_at, updated_at) VALUES ('a', 1, 'Keep', 'local@example.test', 'source', 't', 't');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
             attributes, unread_count, total_count, synced_at)
             VALUES ('f', 'a', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 0, 2, 't');",
        )
        .unwrap();
        for (id, uid, body_state) in [("downloaded", 1, "ready"), ("pending", 2, "missing")] {
            db.execute(
                "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
                 subject, from_address, to_addresses, cc_addresses, reply_to_addresses, received_at,
                 is_read, is_flagged, flags, has_attachment, remote_state, updated_at, body_state)
                 VALUES (?1, 'a', 'source', 'f', 1, ?2, 's', 'x@example.test', '[]', '[]', '[]', 't',
                 0, 0, '[]', 0, 'present', 't', ?3)",
                rusqlite::params![id, uid, body_state],
            )
            .unwrap();
        }
        apply(&mut db, MIGRATIONS).unwrap();
        let state = |id: &str| -> String {
            db.query_row(
                "SELECT preview_state FROM messages WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(state("downloaded"), "ready");
        assert_eq!(state("pending"), "missing");
    }

    #[test]
    fn provider_icon_migration_enables_existing_accounts_and_constrains_the_flag() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(11)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id,
             created_at, updated_at) VALUES ('a', 3, 'Keep', 'local@example.test', 'source', 't', 't');",
        )
        .unwrap();

        apply(&mut db, MIGRATIONS).unwrap();
        let (revision, enabled): (u32, bool) = db
            .query_row(
                "SELECT revision, use_provider_icon FROM accounts WHERE id='a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(revision, 3);
        assert!(enabled);
        db.execute("UPDATE accounts SET use_provider_icon=0", [])
            .unwrap();
        assert!(
            db.execute("UPDATE accounts SET use_provider_icon=2", [])
                .is_err()
        );
        assert!(
            db.execute("UPDATE accounts SET use_provider_icon=NULL", [])
                .is_err()
        );
    }

    #[test]
    fn attachment_origin_upgrade_preserves_rows_order_indexes_and_cascades() {
        let (_directory, mut db) = open_db();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        apply(&mut db, &manifest_up_to(22)).unwrap();
        let draft = "d".repeat(36);
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.invalid', 'source', 't', 't');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO drafts(id, account_id, kind, to_addresses, cc_addresses, bcc_addresses,
             subject, body_text, body_html, created_at, updated_at, client_mutation_id, create_canonical)
             VALUES (?1, 'a', 'new', '[]', '[]', '[]', 'Test', '', '', 't', 't', ?2, '{}')",
            [&draft, &"m".repeat(36)],
        )
        .unwrap();
        let old_rows = [
            (5_i64, "user", 0, None),
            (8, "forwarded", 1, Some("inline@test")),
        ];
        for (rowid, origin, inline, cid) in old_rows {
            db.execute(
                "INSERT INTO draft_attachments
                 (rowid, draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'text/plain', 9, ?5, ?6, ?7, ?8, 'created')",
                rusqlite::params![rowid, draft, rowid.to_string().repeat(36), format!("{rowid}.txt"), "b".repeat(64), inline, cid, origin],
            )
            .unwrap();
        }
        let snapshot = |db: &Connection| {
            db.prepare("SELECT json_array(rowid, draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at)
                        FROM draft_attachments ORDER BY rowid")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let before = snapshot(&db);
        apply(&mut db, MIGRATIONS).unwrap();
        assert_eq!(snapshot(&db), before);
        for (index, origin) in ["mcp", "ai"].into_iter().enumerate() {
            db.execute(
                "INSERT INTO draft_attachments(draft_id, id, file_name, media_type, size, blob_hash, origin, created_at)
                 VALUES (?1, ?2, 'new.txt', 'text/plain', 1, ?3, ?4, 't')",
                rusqlite::params![draft, char::from(b'x' + index as u8).to_string().repeat(36), "c".repeat(64), origin],
            ).unwrap();
        }
        assert!(
            db.execute("UPDATE draft_attachments SET origin = 'untrusted'", [])
                .is_err()
        );
        assert!(
            db.execute("UPDATE draft_attachments SET draft_id = 'missing'", [])
                .is_err()
        );
        assert!(db.execute("UPDATE draft_attachments SET inline = 1, content_id = 'inline@test' WHERE origin = 'mcp'", []).is_err());
        for name in [
            "draft_attachments_by_blob",
            "draft_attachments_inline_content_id",
        ] {
            assert!(
                db.query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                    [name],
                    |row| row.get::<_, i64>(0)
                )
                .is_ok()
            );
        }
        db.execute("DELETE FROM drafts WHERE id = ?1", [&draft])
            .unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM draft_attachments", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn sent_identity_upgrade_preserves_v23_mail_and_marks_headers_for_refresh() {
        let (_directory, mut db) = open_db();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        apply(&mut db, &manifest_up_to(23)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.invalid', 'source', 't', 't');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
               attributes, unread_count, total_count, synced_at)
             VALUES ('f', 'a', 'source', 'Sent', 'Sent', 'sent', 1, '[]', 0, 1, 't');
             INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
               message_id_header, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
               received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at, body_state, preview)
             VALUES ('m', 'a', 'source', 'f', 7, 42, 'server@example.invalid', 'Saved', 'work@example.invalid',
               '[]', '[]', '[]', 't', 1, 1, '[]', 0, 'present', 't', 'ready', 'preview');
             INSERT INTO message_bodies(message_id, raw_sha256, raw_size, plain_text, sanitizer_version,
               links, remote_images_blocked, truncated, fetched_at)
             VALUES ('m', 'hash', 12, 'cached body', 1, '[]', 0, 0, 't');",
        ).unwrap();
        let snapshot = |db: &Connection| -> String {
            db.query_row("SELECT json_array(m.id, m.uid, m.uid_validity, m.message_id_header,
                m.subject, m.is_read, m.is_flagged, m.body_state, m.preview, b.raw_sha256, b.plain_text)
                FROM messages m JOIN message_bodies b ON b.message_id = m.id", [], |row| row.get(0)).unwrap()
        };
        let before = snapshot(&db);
        apply(&mut db, MIGRATIONS).unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        assert_eq!(snapshot(&db), before);
        let identity: (Option<String>, String) = db
            .query_row(
                "SELECT original_message_id, sent_identity_policy FROM messages WHERE id = 'm'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(identity, (None, String::new()));
        assert!(
            db.execute("UPDATE messages SET sent_identity_policy = NULL", [])
                .is_err()
        );
        db.execute("DELETE FROM folders WHERE id = 'f'", [])
            .unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM message_bodies", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn conversation_threading_migration_adds_columns_and_keeps_messages() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(26)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.invalid', 'source', 't', 't');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
               attributes, unread_count, total_count, synced_at)
             VALUES ('f', 'a', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 0, 1, 't');
             INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
               message_id_header, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
               received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
             VALUES ('m', 'a', 'source', 'f', 7, 42, 'x@example.invalid', 'Hi', 'a@example.invalid',
               '[]', '[]', '[]', 't', 1, 1, '[]', 0, 'present', 't');",
        )
        .unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        let row: (Option<String>, Option<String>, Option<String>, String) = db
            .query_row(
                "SELECT in_reply_to, references_header, conversation_id, subject FROM messages WHERE id = 'm'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (None, None, None, "Hi".into()));
        assert!(
            db.query_row(
                "SELECT 1 FROM sqlite_master WHERE name = 'messages_by_conversation'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .is_ok()
        );
    }

    #[test]
    fn conversation_rethread_migration_adds_version_and_clears_conversations() {
        let (_directory, mut db) = open_db();
        apply(&mut db, &manifest_up_to(27)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.invalid', 'source', 't', 't');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
               attributes, unread_count, total_count, synced_at)
             VALUES ('f', 'a', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 0, 1, 't');
             INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
               message_id_header, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
               received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at, conversation_id)
             VALUES ('m', 'a', 'source', 'f', 7, 42, 'x@example.invalid', 'Hi', 'a@example.invalid',
               '[]', '[]', '[]', 't', 1, 1, '[]', 0, 'present', 't', 'old');",
        )
        .unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        let row: (Option<String>, i64, String) = db
            .query_row(
                "SELECT conversation_id, threading_version, subject FROM messages WHERE id = 'm'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (None, 0, "Hi".into()));
    }

    #[test]
    fn message_translations_migration_adds_a_table_that_follows_its_message() {
        let (_directory, mut db) = open_db();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        apply(&mut db, &manifest_up_to(28)).unwrap();
        db.execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES ('a', 1, 'Work', 'work@example.invalid', 'source', 't', 't');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
               attributes, unread_count, total_count, synced_at)
             VALUES ('f', 'a', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 0, 1, 't');
             INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
               message_id_header, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
               received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
             VALUES ('m', 'a', 'source', 'f', 7, 42, 'x@example.invalid', 'Hi', 'a@example.invalid',
               '[]', '[]', '[]', 't', 1, 1, '[]', 0, 'present', 't');",
        )
        .unwrap();
        apply(&mut db, MIGRATIONS).unwrap();
        let insert = |message: &str| {
            db.execute(
                "INSERT INTO message_translations(message_id, target_language, include_quotes, raw_sha256,
                   sanitizer_version, segmenter_version, provider, segments, partial, created_at)
                 VALUES (?1, 'en', 0, 'h', 1, 1, 'deepl', '[]', 0, 't')",
                [message],
            )
        };
        insert("m").unwrap();
        // Rows cannot outlive or precede their message.
        assert!(insert("missing").is_err());
        db.execute("DELETE FROM folders WHERE id = 'f'", [])
            .unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM message_translations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
    }

    #[test]
    fn manifest_must_start_at_one_and_be_contiguous() {
        let (_directory, mut db) = open_db();
        let invalid = [Migration {
            version: 2,
            description: "invalid",
            required_tables: &[],
            sql: V1,
        }];
        assert_eq!(
            apply(&mut db, &invalid).unwrap_err().code,
            ErrorCode::StorageIncompatible
        );
    }
}
