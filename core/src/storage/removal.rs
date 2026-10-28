use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use super::{Storage, decode, encode};
use crate::{
    AppError, ErrorCode,
    accounts::{model::Snapshot, types::RemoveAccountRequest},
};

impl Storage {
    pub(crate) fn create_request_was_removed(&self, request_id: &str) -> Result<bool, AppError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM removed_create_requests WHERE request_id = ?1)",
            [request_id],
            |row| row.get(0),
        )?)
    }

    /// All checks and durable cleanup intents share the transaction with the
    /// cascading delete. AccountService's lock also serializes queue claims.
    pub(crate) fn remove_account(
        &mut self,
        request: &RemoveAccountRequest,
    ) -> Result<Vec<String>, AppError> {
        let tx = self.db.transaction()?;
        let id = &request.account_id;
        let revision: Option<u32> = tx
            .query_row("SELECT revision FROM accounts WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        match revision {
            Some(revision) if revision != request.expected_revision => {
                return Err(AppError::new(ErrorCode::Conflict));
            }
            None => {
                let receipt: Option<(u32, String)> = tx
                    .query_row(
                        "SELECT revision, draft_ids FROM account_removals WHERE account_id = ?1",
                        [id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                return match receipt {
                    Some((revision, drafts)) if revision == request.expected_revision => {
                        decode(&drafts)
                    }
                    Some(_) => Err(AppError::new(ErrorCode::Conflict)),
                    None => Err(AppError::new(ErrorCode::NotFound)),
                };
            }
            Some(_) => {}
        }
        let sending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE account_id = ?1 AND state = 'sending')",
            [id],
            |row| row.get(0),
        )?;
        if sending {
            return Err(AppError::new(ErrorCode::AlreadySending));
        }
        let draft_ids = {
            let mut statement =
                tx.prepare("SELECT id FROM drafts WHERE account_id = ?1 ORDER BY id")?;
            statement
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut secret_refs = {
            let mut statement = tx.prepare(
                "SELECT secret_ref FROM auth_profiles WHERE account_id = ?1
                 UNION SELECT secret_ref FROM network_routes WHERE account_id = ?1 AND secret_ref IS NOT NULL",
            )?;
            statement
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<BTreeSet<_>, _>>()?
        };
        let mut create_requests = {
            let mut statement =
                tx.prepare("SELECT request_id FROM create_requests WHERE account_id = ?1")?;
            statement
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<BTreeSet<_>, _>>()?
        };
        {
            // Include applied and unapplied writes: either may have left keys
            // in the host store. Never run their recovery before removal.
            let mut statement = tx.prepare(
                "SELECT snapshot_json, new_refs_json, retired_refs_json, create_request_json
                 FROM credential_journal WHERE account_id = ?1",
            )?;
            let journals = statement.query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for journal in journals {
                let (snapshot, new, retired, create) = journal?;
                let snapshot: Snapshot = decode(&snapshot)?;
                if snapshot.account.id != *id {
                    return Err(AppError::new(ErrorCode::StorageIncompatible));
                }
                secret_refs.extend(snapshot.refs());
                secret_refs.extend(decode::<Vec<String>>(&new)?);
                secret_refs.extend(decode::<Vec<String>>(&retired)?);
                if let Some(create) = create {
                    create_requests.insert(decode::<(String, String)>(&create)?.0);
                }
            }
        }
        tx.execute(
            "INSERT INTO account_removals(account_id, revision, draft_ids) VALUES (?1, ?2, ?3)",
            params![id, request.expected_revision, encode(&draft_ids)?],
        )?;
        for request_id in create_requests {
            tx.execute(
                "INSERT INTO removed_create_requests(request_id, account_id) VALUES (?1, ?2)",
                params![request_id, id],
            )?;
        }
        for key in secret_refs {
            tx.execute(
                "INSERT OR IGNORE INTO secret_gc(secret_ref, account_id) VALUES (?1, ?2)",
                params![key, id],
            )?;
        }
        // v1 configuration and create-idempotency rows have no CASCADE.
        // Removing journals in this transaction prevents later recovery from
        // applying any obsolete snapshot or losing its key-cleanup intent.
        for table in [
            "credential_journal",
            "create_requests",
            "mail_connections",
            "network_routes",
            "auth_profiles",
        ] {
            tx.execute(&format!("DELETE FROM {table} WHERE account_id = ?1"), [id])?;
        }
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(draft_ids)
    }

    pub(crate) fn removed_secrets(&self) -> Result<Vec<String>, AppError> {
        let mut statement = self
            .db
            .prepare("SELECT DISTINCT secret_ref FROM secret_gc ORDER BY secret_ref")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn removed_secrets_pending(&self, account_id: &str) -> Result<bool, AppError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM secret_gc WHERE account_id = ?1)",
            [account_id],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn acknowledge_removed_secret(&self, key: &str) -> Result<(), AppError> {
        self.db
            .execute("DELETE FROM secret_gc WHERE secret_ref = ?1", [key])?;
        Ok(())
    }
}
