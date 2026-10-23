//! Bounded per-folder metadata synchronization. Plans are pure; network operations
//! never hold the account service or a SQLite transaction.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate, Utc};
use tokio::time::Instant;

use crate::accounts::messages::{FolderSyncCursor, SyncTarget};
use crate::error::{AppError, ErrorCode};
use crate::events::NewMailMessage;
use crate::imap::{FolderStatus, ImapError, Session};

use super::headers::parse_header;
use super::manager::{Shared, SyncError, WorkerToken, with_store};

const WINDOW_DAYS: i64 = 30;
const WINDOW_LIMIT: usize = 1_000;
const HEADER_BATCH_SIZE: usize = 200;
const FLAGS_BATCH_SIZE: usize = 500;
/// Mail received longer ago than this (e.g. found after days offline) is old
/// news and never notified.
const NEW_MAIL_MAX_AGE_HOURS: i64 = 24;
/// How long a folder whose STATUS is unchanged may skip the full round. The
/// periodic full round also catches changes STATUS cannot show, such as
/// `\Flagged` being set or a message being replaced by another.
const FULL_RECONCILE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// STATUS of a folder when it last completed a full round, kept in memory by
/// the account worker. Starting a worker therefore always begins with full rounds.
#[derive(Debug, Clone, Copy)]
pub(super) struct StatusSnapshot {
    status: FolderStatus,
    reconciled_at: Instant,
}

pub(super) type FolderMemory = HashMap<String, StatusSnapshot>;

/// Unchanged UIDVALIDITY, UIDNEXT, MESSAGES and UNSEEN mean no message arrived,
/// disappeared or changed read state since the last full round. Without UIDNEXT
/// an arrival plus an expunge could look unchanged, so such servers never skip.
fn can_skip(previous: Option<&StatusSnapshot>, status: &FolderStatus, now: Instant) -> bool {
    previous.is_some_and(|previous| {
        status.uid_next.is_some()
            && status.uid_validity.is_some()
            && previous.status == *status
            && now.duration_since(previous.reconciled_at) < FULL_RECONCILE_INTERVAL
    })
}

#[derive(Debug, PartialEq, Eq)]
struct MessagePlan {
    fetch: Vec<u32>,
    refresh_flags: Vec<u32>,
    deleted: Vec<u32>,
    capped: bool,
}

/// SEARCH has already completed successfully. Its full result, rather than
/// the capped selection, is the only authority for disappearance in this range.
fn plan_messages(
    cursor: &FolderSyncCursor,
    cached: &[(u32, String)],
    server_uids: &[u32],
) -> MessagePlan {
    let remote: HashSet<u32> = server_uids.iter().copied().collect();
    let cached_ids: HashSet<u32> = cached.iter().map(|(uid, _)| *uid).collect();
    let mut selected: Vec<u32> = remote.iter().copied().collect();
    selected.sort_unstable_by(|a, b| b.cmp(a));
    let capped = selected.len() > WINDOW_LIMIT;
    selected.truncate(WINDOW_LIMIT);
    MessagePlan {
        fetch: selected
            .iter()
            .filter(|uid| !cached_ids.contains(uid))
            .copied()
            .collect(),
        refresh_flags: selected
            .into_iter()
            .filter(|uid| cached_ids.contains(uid))
            .collect(),
        deleted: cached
            .iter()
            .filter(|(uid, received_at)| {
                received_at.as_str() >= cursor.window_start.as_str() && !remote.contains(uid)
            })
            .map(|(uid, _)| *uid)
            .collect(),
        capped,
    }
}

/// Turns a store write into a sync result, noting whether it changed visible data.
fn written(result: Option<usize>, changed: &mut bool) -> Result<(), SyncError> {
    match result {
        None => Err(SyncError::Stale),
        Some(rows) => {
            *changed |= rows > 0;
            Ok(())
        }
    }
}

/// What a folder round produced, kept even when the round fails part way.
#[derive(Debug, Default)]
pub(super) struct FolderRound {
    /// Visible data changed, so the UI refreshes for what was already committed.
    pub changed: bool,
    /// New unread inbox mail stored in this round.
    pub arrivals: Vec<NewMailMessage>,
}

/// Syncs one folder.
pub(super) async fn sync_folder(
    shared: &std::sync::Arc<Shared>,
    token: &WorkerToken,
    folder_id: &str,
    remote_name: &str,
    session: &mut Session,
    memory: &mut FolderMemory,
    round: &mut FolderRound,
) -> Result<(), SyncError> {
    let changed = &mut round.changed;
    // Forgotten until this round completes, so any failure forces a full round next time.
    let previous = memory.remove(folder_id);
    let account_id = &token.account_id;
    let source = &token.source;
    let status = session.status(remote_name).await?;
    let id = account_id.to_owned();
    let source_id = source.to_owned();
    let folder = folder_id.to_owned();
    written(
        with_store(shared, token, move |service| {
            service.update_folder_counts(&id, &source_id, &folder, status.unseen, status.messages)
        })
        .await?,
        changed,
    )?;

    let started = Instant::now();
    if let Some(previous) = previous.filter(|previous| can_skip(Some(previous), &status, started)) {
        memory.insert(folder_id.to_owned(), previous);
        return Ok(());
    }

    let mailbox = session.examine(remote_name).await?;
    let target = SyncTarget {
        account_id: account_id.to_owned(),
        mailbox_source_id: source.to_owned(),
        folder_id: folder_id.to_owned(),
        uid_validity: mailbox.uid_validity,
    };
    let now = Utc::now();
    let window_start = (now.date_naive() - Duration::days(WINDOW_DAYS)).to_string();
    let guard = target.clone();
    let cursor = with_store(shared, token, move |service| {
        service.ensure_generation(&guard, &window_start)
    })
    .await?
    .ok_or(SyncError::Stale)?;
    // A new generation hides the old messages and resets coverage.
    *changed |= cursor.created;
    let since = NaiveDate::parse_from_str(&cursor.window_start, "%Y-%m-%d")
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    let remote = session.search_since(since).await?;
    let guard = target.clone();
    let cached = with_store(shared, token, move |service| service.cached_uids(&guard)).await?;
    let plan = plan_messages(&cursor, &cached, &remote);
    let guard = target.clone();
    let capped = plan.capped;
    // Backfill is marked open for the round and closed at its end; only the
    // net coverage change between rounds matters to readers, checked below.
    let mut transient = false;
    written(
        with_store(shared, token, move |service| {
            service.begin_backfill(&guard, capped)
        })
        .await?,
        &mut transient,
    )?;

    // Mail above the highest UID seen so far arrived since then: UIDs only grow
    // within a generation. A new generation (first sync, rebuilt folder) has
    // nothing to compare with, and neither has one whose first round stored
    // nothing, so all their mail counts as already there.
    let arrived_after = (!cursor.created && (cursor.backfill_done || cursor.highest_uid > 0))
        .then_some(cursor.highest_uid);
    let mut highest_uid = cursor.highest_uid;
    for batch in plan.fetch.chunks(HEADER_BATCH_SIZE) {
        let headers = session.fetch_headers(batch).await?;
        let fetched: HashSet<u32> = headers.iter().map(|header| header.uid).collect();
        let complete = batch.iter().all(|uid| fetched.contains(uid));
        let records: Vec<_> = headers
            .iter()
            .map(|header| parse_header(header, now))
            .collect();
        highest_uid = highest_uid.max(records.iter().map(|record| record.uid).max().unwrap_or(0));
        let guard = target.clone();
        written(
            with_store(shared, token, move |service| {
                service.store_message_batch(&guard, &records, highest_uid)
            })
            .await?,
            changed,
        )?;
        if !complete {
            // A concurrent expunge may remove a searched UID. Keep committed
            // records but re-enumerate before claiming complete coverage.
            return Err(ImapError::Mailbox("mailbox changed during header fetch".into()).into());
        }
    }

    if let Some(after) = arrived_after {
        let uids: Vec<u32> = plan
            .fetch
            .iter()
            .copied()
            .filter(|uid| *uid > after)
            .collect();
        if !uids.is_empty() {
            let guard = target.clone();
            let since = (now - Duration::hours(NEW_MAIL_MAX_AGE_HOURS))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            round.arrivals.extend(
                with_store(shared, token, move |service| {
                    service.new_arrivals(&guard, &uids, &since)
                })
                .await?,
            );
        }
    }
    let guard = target.clone();
    written(
        with_store(shared, token, move |service| {
            service.mark_deleted(&guard, &plan.deleted)
        })
        .await?,
        changed,
    )?;
    for batch in plan.refresh_flags.chunks(FLAGS_BATCH_SIZE) {
        let flags = session.fetch_flags(batch).await?;
        let fetched: HashSet<u32> = flags.iter().map(|(uid, _)| *uid).collect();
        let complete = batch.iter().all(|uid| fetched.contains(uid));
        let guard = target.clone();
        written(
            with_store(shared, token, move |service| {
                service.apply_flags(&guard, &flags)
            })
            .await?,
            changed,
        )?;
        if !complete {
            return Err(ImapError::Mailbox("mailbox changed during flag fetch".into()).into());
        }
    }
    let guard = target;
    written(
        with_store(shared, token, move |service| {
            service.finish_backfill(&guard, capped)
        })
        .await?,
        &mut transient,
    )?;
    *changed |= !cursor.backfill_done || cursor.window_capped != capped;
    memory.insert(
        folder_id.to_owned(),
        StatusSnapshot {
            status,
            reconciled_at: started,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> FolderSyncCursor {
        FolderSyncCursor {
            uid_validity: 42,
            window_start: "2026-09-18".into(),
            window_capped: false,
            backfill_done: false,
            highest_uid: 0,
            updated_at: "2026-10-18T00:00:00.000Z".into(),
            created: false,
        }
    }

    fn status(messages: u32, unseen: u32, uid_next: Option<u32>) -> FolderStatus {
        FolderStatus {
            messages,
            unseen,
            uid_next,
            uid_validity: Some(7),
        }
    }

    #[test]
    fn unchanged_status_skips_until_the_next_full_round() {
        let start = Instant::now();
        let snapshot = StatusSnapshot {
            status: status(10, 2, Some(11)),
            reconciled_at: start,
        };
        assert!(can_skip(
            Some(&snapshot),
            &status(10, 2, Some(11)),
            start + std::time::Duration::from_secs(60)
        ));
        assert!(
            !can_skip(
                Some(&snapshot),
                &status(10, 2, Some(11)),
                start + FULL_RECONCILE_INTERVAL
            ),
            "a full round is due every ten minutes"
        );
        assert!(
            !can_skip(None, &status(10, 2, Some(11)), start),
            "the first round is always full"
        );
    }

    #[test]
    fn any_status_change_or_missing_uidnext_forces_a_full_round() {
        let start = Instant::now();
        let snapshot = StatusSnapshot {
            status: status(10, 2, Some(11)),
            reconciled_at: start,
        };
        for changed in [
            status(11, 2, Some(12)),
            status(10, 1, Some(11)),
            status(10, 2, Some(12)),
        ] {
            assert!(!can_skip(Some(&snapshot), &changed, start));
        }
        let mut other_generation = status(10, 2, Some(11));
        other_generation.uid_validity = Some(8);
        assert!(!can_skip(Some(&snapshot), &other_generation, start));

        let without_uidnext = StatusSnapshot {
            status: status(10, 2, None),
            reconciled_at: start,
        };
        assert!(
            !can_skip(Some(&without_uidnext), &status(10, 2, None), start),
            "an arrival plus an expunge is invisible without UIDNEXT"
        );
    }

    #[test]
    fn initial_sync_uses_actual_uids_in_descending_order() {
        let plan = plan_messages(&cursor(), &[], &[2, 98, 4, 98]);
        assert_eq!(plan.fetch, [98, 4, 2]);
        assert!(plan.refresh_flags.is_empty());
        assert!(plan.deleted.is_empty());
        assert!(!plan.capped);
    }

    #[test]
    fn incremental_plan_does_not_skip_holes_below_the_high_water_mark() {
        let mut cursor = cursor();
        cursor.highest_uid = 100;
        let cached = vec![(100, "2026-10-17T00:00:00.000Z".into())];
        let plan = plan_messages(&cursor, &cached, &[99, 100, 101]);
        assert_eq!(plan.fetch, [101, 99]);
        assert_eq!(plan.refresh_flags, [100]);
    }

    #[test]
    fn capped_selection_never_implies_deletion_of_unselected_remote_mail() {
        let remote: Vec<_> = (1..=1001).collect();
        let cached = vec![(1, "2026-10-17T00:00:00.000Z".into())];
        let plan = plan_messages(&cursor(), &cached, &remote);
        assert!(plan.capped);
        assert_eq!(plan.fetch.len(), WINDOW_LIMIT);
        assert_eq!(plan.fetch.first(), Some(&1001));
        assert_eq!(plan.fetch.last(), Some(&2));
        assert!(plan.deleted.is_empty());
    }

    #[test]
    fn deletion_only_reconciles_the_successfully_enumerated_window() {
        let cached = vec![
            (1, "2026-09-17T23:59:59.999Z".into()),
            (2, "2026-09-18T00:00:00.000Z".into()),
            (3, "2026-10-17T00:00:00.000Z".into()),
        ];
        let plan = plan_messages(&cursor(), &cached, &[3]);
        assert_eq!(plan.deleted, [2]);
        assert_eq!(plan.refresh_flags, [3]);
        assert_eq!(plan_messages(&cursor(), &cached, &[]).deleted, [2, 3]);
    }

    #[test]
    fn generation_change_uses_only_the_new_generations_cache() {
        // The store switches the generation and supplies no old present rows.
        let mut cursor = cursor();
        cursor.uid_validity = 43;
        let plan = plan_messages(&cursor, &[], &[1, 2]);
        assert_eq!(plan.fetch, [2, 1]);
        assert!(plan.deleted.is_empty());
    }
}
