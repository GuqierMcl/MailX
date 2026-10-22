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
