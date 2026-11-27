use chrono::{Duration, NaiveDate};
use rusqlite::{OptionalExtension, params};

use super::{Storage, current_folder, current_target, now};
use crate::{
    AppError, ErrorCode,
    settings::SyncWindow,
    sync::{
        backfill::{AccountBackfillTask, BackfillControlAction, BackfillPhase},
        manager::{SyncFailureCode, failure},
    },
};

use super::SyncTarget;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BackfillFolderJob {
    pub account_id: String,
    pub mailbox_source_id: String,
    pub folder_id: String,
    pub folder_name: String,
    pub remote_name: String,
    pub uid_validity: u32,
    pub highest_uid: u32,
    pub window_start: String,
    pub history_target_date: String,
    pub history_covered_since: Option<String>,
    pub history_cursor_before: Option<String>,
    pub history_fetched: u32,
    pub history_total: Option<u32>,
}

fn covers_date(covered: &str, desired: &str) -> bool {
    if covered == "all" {
        return true;
    }
    if desired == "all" {
        return false;
    }
    covered <= desired
}

fn failure_code_to_db(code: SyncFailureCode) -> &'static str {
    match code {
        SyncFailureCode::Authentication => "authentication",
        SyncFailureCode::SignInRequired => "signInRequired",
        SyncFailureCode::ProviderPolicy => "providerPolicy",
        SyncFailureCode::CredentialUntrusted => "credentialUntrusted",
        SyncFailureCode::CredentialMissing => "credentialMissing",
        SyncFailureCode::Network => "network",
        SyncFailureCode::Proxy => "proxy",
        SyncFailureCode::Tls => "tls",
        SyncFailureCode::Timeout => "timeout",
        SyncFailureCode::Unsupported => "unsupported",
        SyncFailureCode::Protocol => "protocol",
        SyncFailureCode::FlagsIncomplete => "flagsIncomplete",
        SyncFailureCode::Storage => "storage",
    }
}

fn failure_code_from_db(value: &str) -> SyncFailureCode {
    match value {
        "authentication" => SyncFailureCode::Authentication,
        "signInRequired" => SyncFailureCode::SignInRequired,
        "providerPolicy" => SyncFailureCode::ProviderPolicy,
        "credentialUntrusted" => SyncFailureCode::CredentialUntrusted,
        "credentialMissing" => SyncFailureCode::CredentialMissing,
        "network" => SyncFailureCode::Network,
        "proxy" => SyncFailureCode::Proxy,
        "tls" => SyncFailureCode::Tls,
        "timeout" => SyncFailureCode::Timeout,
        "unsupported" => SyncFailureCode::Unsupported,
        "flagsIncomplete" => SyncFailureCode::FlagsIncomplete,
        "storage" => SyncFailureCode::Storage,
        _ => SyncFailureCode::Protocol,
    }
}

fn settle_completed_if_account_idle(
    tx: &rusqlite::Transaction<'_>,
    account_id: &str,
    source: &str,
) -> Result<(), AppError> {
    let active: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM folder_sync_cursors c
            JOIN folders f ON f.id = c.folder_id
            WHERE f.account_id = ?1 AND f.mailbox_source_id = ?2 AND f.selectable = 1
              AND c.history_state IN ('running', 'paused', 'failed')
        )",
        params![account_id, source],
        |row| row.get(0),
    )?;
    if !active {
        tx.execute(
            "UPDATE folder_sync_cursors
             SET history_state = 'idle', history_fetched = 0, history_total = NULL,
                 history_cursor_before = NULL, history_failure_code = NULL
             WHERE history_state = 'completed'
               AND folder_id IN (SELECT id FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2)",
            params![account_id, source],
        )?;
    }
    Ok(())
}

impl Storage {
    /// Schedules or shrinks historical backfill for an account's eligible folders
    /// (`inbox`, `sent`, `archive`, `other`; skipping `junk` and `trash`).
    /// Shrinking preserves all cached mail and simply stops deeper backfilling.
    pub(crate) fn schedule_account_backfill(
        &mut self,
        account_id: &str,
        window: SyncWindow,
        today: NaiveDate,
    ) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        let source: Option<String> = tx
            .query_row(
                "SELECT mailbox_source_id FROM accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(source) = source else {
            return Err(AppError::new(ErrorCode::NotFound));
        };
        settle_completed_if_account_idle(&tx, account_id, &source)?;

        let desired = window
            .target_date(today)
            .map(|date| date.to_string())
            .unwrap_or_else(|| "all".into());

        struct ScheduledRow {
            folder_id: String,
            window_start: String,
            covered_since: Option<String>,
            existing_target: Option<String>,
            state: String,
        }
        let mut stmt = tx.prepare(
            "SELECT f.id, c.window_start, c.history_covered_since, c.history_target_date, c.history_state
             FROM folders f
             JOIN folder_sync_cursors c ON c.folder_id = f.id
             WHERE f.account_id = ?1 AND f.mailbox_source_id = ?2 AND f.selectable = 1
               AND f.role NOT IN ('junk', 'trash')",
        )?;
        let rows: Vec<ScheduledRow> = stmt
            .query_map(params![account_id, source], |row| {
                Ok(ScheduledRow {
                    folder_id: row.get(0)?,
                    window_start: row.get(1)?,
                    covered_since: row.get(2)?,
                    existing_target: row.get(3)?,
                    state: row.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        drop(stmt);

        let timestamp = now();
        let mut queued = false;
        for row in rows {
            let ScheduledRow {
                folder_id,
                window_start,
                covered_since,
                existing_target,
                state,
            } = row;
            let effective_covered = covered_since.as_deref().unwrap_or(&window_start);
            if covers_date(effective_covered, &desired) {
                if matches!(state.as_str(), "running" | "paused" | "failed") {
                    tx.execute(
                        "UPDATE folder_sync_cursors
                         SET history_state = 'idle', history_failure_code = NULL,
                             history_cursor_before = NULL, updated_at = ?2
                         WHERE folder_id = ?1",
                        params![folder_id, timestamp],
                    )?;
                }
                continue;
            }
            if state == "running"
                && existing_target
                    .as_deref()
                    .is_some_and(|target| covers_date(target, &desired))
            {
                queued = true;
                continue;
            }
            let target_changed = existing_target.as_deref() != Some(desired.as_str());
            let keep_progress =
                matches!(state.as_str(), "running" | "paused" | "failed") && !target_changed;
            tx.execute(
                "UPDATE folder_sync_cursors
                 SET history_target_date = ?2,
                     history_state = 'running',
                     history_failure_code = NULL,
                     history_fetched = CASE WHEN ?3 THEN history_fetched ELSE 0 END,
                     history_total = CASE WHEN ?3 THEN history_total ELSE NULL END,
                     history_cursor_before = CASE WHEN ?3 THEN history_cursor_before ELSE NULL END,
                     updated_at = ?4
                 WHERE folder_id = ?1",
                params![folder_id, desired, keep_progress, timestamp],
            )?;
            queued = true;
        }
        settle_completed_if_account_idle(&tx, account_id, &source)?;
        tx.commit()?;
        Ok(queued)
    }

    /// Extends a single folder's history backfill target by `days` earlier than its
    /// current covered boundary without modifying the account's `sync_window` setting.
    pub(crate) fn load_older_folder(
        &mut self,
        folder_id: &str,
        days: i64,
    ) -> Result<(String, bool), AppError> {
        let tx = self.db.transaction()?;
        struct LoadOlderRow {
            account_id: String,
            source: String,
            window_start: String,
            covered_since: Option<String>,
            existing_target: Option<String>,
            state: String,
        }
        let row: Option<LoadOlderRow> = tx
            .query_row(
                "SELECT f.account_id, f.mailbox_source_id, c.window_start,
                        c.history_covered_since, c.history_target_date, c.history_state
                 FROM folders f
                 JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
                 LEFT JOIN folder_sync_cursors c ON c.folder_id = f.id
                 WHERE f.id = ?1 AND f.selectable = 1",
                [folder_id],
                |row| {
                    Ok(LoadOlderRow {
                        account_id: row.get(0)?,
                        source: row.get(1)?,
                        window_start: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        covered_since: row.get(3)?,
                        existing_target: row.get(4)?,
                        state: row.get::<_, Option<String>>(5)?
                            .unwrap_or_else(|| "idle".into()),
                    })
                },
            )
            .optional()?;
        let Some(LoadOlderRow {
            account_id,
            source,
            window_start,
            covered_since,
            existing_target,
            state,
        }) = row
        else {
            return Err(AppError::new(ErrorCode::NotFound));
        };
        if window_start.is_empty() {
            return Ok((account_id, false));
        }
        if covered_since.as_deref() == Some("all") {
            return Ok((account_id, false));
        }
        settle_completed_if_account_idle(&tx, &account_id, &source)?;
        let base_str = covered_since.as_deref().unwrap_or(&window_start);
        let base_date = NaiveDate::parse_from_str(base_str, "%Y-%m-%d")
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
        let next_target = (base_date - Duration::days(days)).to_string();
        let target = if state == "running"
            && existing_target
                .as_deref()
                .is_some_and(|existing| covers_date(existing, &next_target))
        {
            existing_target.unwrap()
        } else {
            next_target
        };
        let reset_counters = !matches!(state.as_str(), "running" | "paused" | "failed");
        tx.execute(
            "UPDATE folder_sync_cursors
             SET history_target_date = ?2,
                 history_state = 'running',
                 history_failure_code = NULL,
                 history_fetched = CASE WHEN ?3 THEN 0 ELSE history_fetched END,
                 history_total = NULL,
                 history_cursor_before = CASE WHEN ?3 THEN NULL ELSE history_cursor_before END,
                 updated_at = ?4
             WHERE folder_id = ?1",
            params![folder_id, target, reset_counters, now()],
        )?;
        tx.commit()?;
        Ok((account_id, true))
    }

    pub(crate) fn control_account_backfill(
        &mut self,
        account_id: &str,
        action: BackfillControlAction,
    ) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        let source: Option<String> = tx
            .query_row(
                "SELECT mailbox_source_id FROM accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(source) = source else {
            return Err(AppError::new(ErrorCode::NotFound));
        };
        let timestamp = now();
        let changed = match action {
            BackfillControlAction::Pause => tx.execute(
                "UPDATE folder_sync_cursors
                 SET history_state = 'paused', updated_at = ?3
                 WHERE history_state = 'running'
                   AND folder_id IN (SELECT id FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2)",
                params![account_id, source, timestamp],
            )?,
            BackfillControlAction::Resume => tx.execute(
                "UPDATE folder_sync_cursors
                 SET history_state = 'running', history_failure_code = NULL, updated_at = ?3
                 WHERE history_state IN ('paused', 'failed')
                   AND folder_id IN (SELECT id FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2)",
                params![account_id, source, timestamp],
            )?,
            BackfillControlAction::Cancel => tx.execute(
                "UPDATE folder_sync_cursors
                 SET history_state = 'idle', history_failure_code = NULL,
                     history_cursor_before = NULL, history_fetched = 0, history_total = NULL,
                     updated_at = ?3
                 WHERE history_state IN ('running', 'paused', 'failed', 'completed')
                   AND folder_id IN (SELECT id FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2)",
                params![account_id, source, timestamp],
            )?,
        };
        tx.commit()?;
        Ok(changed > 0)
    }

    pub(crate) fn next_backfill_folder(
        &self,
        account_id: &str,
        source: &str,
    ) -> Result<Option<BackfillFolderJob>, AppError> {
        let row = self
            .db
            .query_row(
                "SELECT f.id, f.name, f.remote_name, c.uid_validity, c.highest_uid, c.window_start,
                        c.history_target_date, c.history_covered_since, c.history_cursor_before,
                        c.history_fetched, c.history_total
                 FROM folder_sync_cursors c
                 JOIN folders f ON f.id = c.folder_id
                 JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
                 WHERE f.account_id = ?1 AND f.mailbox_source_id = ?2 AND f.selectable = 1
                   AND c.history_state = 'running' AND c.history_target_date IS NOT NULL
                 ORDER BY (f.role = 'inbox') DESC, (f.role = 'sent') DESC, f.remote_name, f.id
                 LIMIT 1",
                params![account_id, source],
                |row| {
                    let fetched: i64 = row.get(9)?;
                    let total: Option<i64> = row.get(10)?;
                    Ok(BackfillFolderJob {
                        account_id: account_id.to_owned(),
                        mailbox_source_id: source.to_owned(),
                        folder_id: row.get(0)?,
                        folder_name: row.get(1)?,
                        remote_name: row.get(2)?,
                        uid_validity: row.get(3)?,
                        highest_uid: row.get(4)?,
                        window_start: row.get(5)?,
                        history_target_date: row.get(6)?,
                        history_covered_since: row.get(7)?,
                        history_cursor_before: row.get(8)?,
                        history_fetched: u32::try_from(fetched).unwrap_or(u32::MAX),
                        history_total: total.and_then(|v| u32::try_from(v).ok()),
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub(crate) fn update_backfill_progress(
        &mut self,
        target: &SyncTarget,
        fetched_delta: u32,
        total: Option<u32>,
        covered_since: Option<&str>,
        cursor_before: Option<&str>,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE folder_sync_cursors
             SET history_fetched = history_fetched + ?2,
                 history_total = COALESCE(history_total, ?3),
                 history_covered_since = CASE
                     WHEN history_covered_since = 'all' THEN 'all'
                     WHEN ?4 IS NULL THEN history_covered_since
                     WHEN history_covered_since IS NULL OR ?4 < history_covered_since THEN ?4
                     ELSE history_covered_since
                 END,
                 history_cursor_before = COALESCE(?5, history_cursor_before),
                 updated_at = ?6
             WHERE folder_id = ?1 AND uid_validity = ?7 AND history_state = 'running'",
            params![
                target.folder_id,
                i64::from(fetched_delta),
                total.map(i64::from),
                covered_since,
                cursor_before,
                now(),
                target.uid_validity
            ],
        )?;
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn complete_backfill_folder(
        &mut self,
        target: &SyncTarget,
        covered_since: &str,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE folder_sync_cursors
             SET history_state = 'completed',
                 history_failure_code = NULL,
                 history_cursor_before = NULL,
                 history_covered_since = CASE
                     WHEN history_covered_since = 'all' OR ?2 = 'all' THEN 'all'
                     WHEN history_covered_since IS NULL OR ?2 < history_covered_since THEN ?2
                     ELSE history_covered_since
                 END,
                 updated_at = ?3
             WHERE folder_id = ?1 AND uid_validity = ?4 AND history_state = 'running'",
            params![target.folder_id, covered_since, now(), target.uid_validity],
        )?;
        settle_completed_if_account_idle(&tx, &target.account_id, &target.mailbox_source_id)?;
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn fail_backfill_folder(
        &mut self,
        target: &SyncTarget,
        code: SyncFailureCode,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_folder(
            &tx,
            &target.account_id,
            &target.mailbox_source_id,
            &target.folder_id,
        )? {
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE folder_sync_cursors
             SET history_state = 'failed',
                 history_failure_code = ?2,
                 updated_at = ?3
             WHERE folder_id = ?1 AND uid_validity = ?4 AND history_state = 'running'",
            params![
                target.folder_id,
                failure_code_to_db(code),
                now(),
                target.uid_validity
            ],
        )?;
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn backfill_tasks(&self) -> Result<Vec<AccountBackfillTask>, AppError> {
        let mut stmt = self.db.prepare(
            "SELECT f.account_id, f.name, c.history_state, c.history_target_date,
                    c.history_covered_since, c.window_start, c.history_fetched,
                    c.history_total, c.history_failure_code
             FROM folder_sync_cursors c
             JOIN folders f ON f.id = c.folder_id
             JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
             WHERE f.selectable = 1 AND c.history_state IN ('running', 'paused', 'failed', 'completed')
             ORDER BY a.created_at, a.id, (f.role = 'inbox') DESC, (f.role = 'sent') DESC, f.remote_name, f.id",
        )?;
        struct RowData {
            account_id: String,
            folder_name: String,
            state: String,
            target_date: Option<String>,
            covered_since: Option<String>,
            window_start: String,
            fetched: u32,
            total: Option<u32>,
            failure_code: Option<String>,
        }
        let rows: Vec<RowData> = stmt
            .query_map([], |row| {
                let fetched: i64 = row.get(6)?;
                let total: Option<i64> = row.get(7)?;
                Ok(RowData {
                    account_id: row.get(0)?,
                    folder_name: row.get(1)?,
                    state: row.get(2)?,
                    target_date: row.get(3)?,
                    covered_since: row.get(4)?,
                    window_start: row.get(5)?,
                    fetched: u32::try_from(fetched).unwrap_or(u32::MAX),
                    total: total.and_then(|v| u32::try_from(v).ok()),
                    failure_code: row.get(8)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        let mut tasks = Vec::new();
        let mut idx = 0;
        while idx < rows.len() {
            let account_id = &rows[idx].account_id;
            let mut end = idx + 1;
            while end < rows.len() && &rows[end].account_id == account_id {
                end += 1;
            }
            let slice = &rows[idx..end];
            idx = end;

            let has_running = slice.iter().any(|row| row.state == "running");
            let has_failed = slice.iter().any(|row| row.state == "failed");
            let has_paused = slice.iter().any(|row| row.state == "paused");
            if !has_running && !has_failed && !has_paused {
                continue;
            }

            let phase = if has_running {
                BackfillPhase::Running
            } else if has_failed {
                BackfillPhase::Failed
            } else {
                BackfillPhase::Paused
            };
            let focus = slice
                .iter()
                .find(|row| row.state == "running")
                .or_else(|| slice.iter().find(|row| row.state == "failed"))
                .or_else(|| slice.iter().find(|row| row.state == "paused"))
                .unwrap();

            let fetched: u32 = slice
                .iter()
                .map(|row| row.fetched)
                .fold(0u32, u32::saturating_add);
            let mut known_total = 0u32;
            let mut has_known_total = false;
            for row in slice {
                if row.state == "completed" {
                    known_total = known_total.saturating_add(row.fetched);
                    has_known_total = true;
                } else if let Some(folder_total) = row.total {
                    known_total = known_total.saturating_add(folder_total);
                    has_known_total = true;
                }
            }
            let total = has_known_total.then_some(known_total.max(fetched));
            let covered_since = focus
                .covered_since
                .clone()
                .or_else(|| Some(focus.window_start.clone()));
            let failure = if phase == BackfillPhase::Failed {
                focus
                    .failure_code
                    .as_deref()
                    .map(|code| failure(failure_code_from_db(code), None, true))
            } else {
                None
            };
            tasks.push(AccountBackfillTask {
                account_id: account_id.clone(),
                phase,
                target_date: focus.target_date.clone(),
                covered_since,
                fetched,
                total,
                active_folder_name: Some(focus.folder_name.clone()),
                failure,
            });
        }
        Ok(tasks)
    }
}
