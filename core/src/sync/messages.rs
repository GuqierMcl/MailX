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
/// Cap on starred messages outside the ordinary date window pulled per folder,
/// so a mailbox with thousands of historical stars stays bounded.
const FLAGGED_WINDOW_LIMIT: usize = 500;
const HEADER_BATCH_SIZE: usize = 200;
const FLAGS_BATCH_SIZE: usize = 500;
const FLAGS_RETRY_BATCH_SIZE: usize = 25;
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
/// Starred messages outside the date window are fetched up to
/// [`FLAGGED_WINDOW_LIMIT`] and existing starred cache rows always refresh
/// their flags so unstarring an older message converges without treating it as
/// deleted.
fn plan_messages(
    cursor: &FolderSyncCursor,
    cached: &[(u32, String, bool)],
    server_uids: &[u32],
    flagged_uids: &[u32],
) -> MessagePlan {
    let remote: HashSet<u32> = server_uids.iter().copied().collect();
    let cached_ids: HashSet<u32> = cached.iter().map(|(uid, _, _)| *uid).collect();
    let mut selected: Vec<u32> = remote.iter().copied().collect();
    selected.sort_unstable_by(|a, b| b.cmp(a));
    let capped = selected.len() > WINDOW_LIMIT;
    selected.truncate(WINDOW_LIMIT);

    let selected_ids: HashSet<u32> = selected.iter().copied().collect();
    let mut extra_flagged: Vec<u32> = flagged_uids
        .iter()
        .copied()
        .filter(|uid| !selected_ids.contains(uid))
        .collect();
    extra_flagged.sort_unstable_by(|a, b| b.cmp(a));
    extra_flagged.dedup();
    extra_flagged.truncate(FLAGGED_WINDOW_LIMIT);

    let mut fetch: Vec<u32> = selected
        .iter()
        .chain(extra_flagged.iter())
        .filter(|uid| !cached_ids.contains(uid))
        .copied()
        .collect();
    fetch.sort_unstable_by(|a, b| b.cmp(a));
    fetch.dedup();

    let mut refresh_flags: Vec<u32> = selected
        .into_iter()
        .chain(extra_flagged)
        .filter(|uid| cached_ids.contains(uid))
        .chain(
            cached
                .iter()
                .filter(|(_, _, is_flagged)| *is_flagged)
                .map(|(uid, _, _)| *uid),
        )
        .collect();
    refresh_flags.sort_unstable_by(|a, b| b.cmp(a));
    refresh_flags.dedup();

    MessagePlan {
        fetch,
        refresh_flags,
        deleted: cached
            .iter()
            .filter(|(uid, received_at, _)| {
                received_at.as_str() >= cursor.window_start.as_str() && !remote.contains(uid)
            })
            .map(|(uid, _, _)| *uid)
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
    /// Cached mails whose flags the server would not confirm; they keep their
    /// last known state. Reported as a retryable warning, not a failure.
    pub unresolved_flags: usize,
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
    let (status, examined) = session.status_or_examine(remote_name).await?;
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

    let compatibility = session.compatibility();
    let policy = compatibility.policy_key();
    let (id, source_id, folder) = (account_id.clone(), source.clone(), folder_id.to_owned());
    let pending_identity = with_store(shared, token, move |service| {
        service.sent_identity_refresh_uids(&id, &source_id, &folder, policy)
    })
    .await?;
    let started = Instant::now();
    if let Some(previous) = previous.filter(|previous| {
        pending_identity.is_empty() && can_skip(Some(previous), &status, started)
    }) {
        reconcile_cached_sent(shared, token, folder_id, changed).await?;
        memory.insert(folder_id.to_owned(), previous);
        return Ok(());
    }

    let mailbox = match examined {
        Some(mailbox) => mailbox,
        None => session.examine(remote_name).await?,
    };
    let target = SyncTarget {
        account_id: account_id.to_owned(),
        mailbox_source_id: source.to_owned(),
        folder_id: folder_id.to_owned(),
        uid_validity: mailbox.uid_validity,
    };
    let now = Utc::now();
    let window_start = (now.date_naive() - Duration::days(WINDOW_DAYS)).to_string();
    let global_window = shared.global_sync_window();
    let guard = target.clone();
    let account_id_for_window = account_id.to_owned();
    let today = now.date_naive();
    let (cursor, scheduled) = with_store(shared, token, move |service| {
        let Some(cursor) = service.ensure_generation(&guard, &window_start)? else {
            return Ok((None, false));
        };
        let mut scheduled = false;
        if cursor.created {
            let effective = service
                .get(&account_id_for_window)?
                .sync_window
                .unwrap_or(global_window);
            if effective != crate::settings::SyncWindow::Days30 {
                scheduled =
                    service.schedule_account_backfill(&account_id_for_window, effective, today)?;
            }
        }
        Ok((Some(cursor), scheduled))
    })
    .await?;
    let cursor = cursor.ok_or(SyncError::Stale)?;
    if scheduled {
        shared.emit_backfill_tasks();
    }
    // A new generation hides the old messages and resets coverage.
    *changed |= cursor.created;
    let since = NaiveDate::parse_from_str(&cursor.window_start, "%Y-%m-%d")
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    let remote = session.search_since(since, mailbox).await?;
    let flagged = if mailbox.exists == Some(0) && remote.is_empty() {
        Vec::new()
    } else {
        let mut flagged_mailbox = mailbox;
        if !remote.is_empty() {
            flagged_mailbox.exists = None;
        }
        match session.search_flagged(flagged_mailbox).await {
            Ok(uids) => uids,
            // Servers that refuse FLAGGED search still sync the ordinary window.
            Err(ImapError::Rejected) => Vec::new(),
            Err(error) => return Err(error.into()),
        }
    };
    let guard = target.clone();
    let cached = with_store(shared, token, move |service| service.cached_uids(&guard)).await?;
    let plan = plan_messages(&cursor, &cached, &remote, &flagged);
    // Commit known disappearances before any new identity can release a local
    // snapshot against an already-removed server copy from this round.
    let guard = target.clone();
    let deleted = plan.deleted.clone();
    written(
        with_store(shared, token, move |service| {
            service.mark_deleted(&guard, &deleted)
        })
        .await?,
        changed,
    )?;
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
            .map(|header| parse_header(header, now, compatibility))
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

    // Existing cached UIDs normally refresh only flags. Repair their identity
    // once per policy version, even when STATUS is unchanged or mail is older
    // than the ordinary sync window. The second selection follows any UIDVALIDITY
    // reset/new-header writes above, so it cannot repair the previous generation.
    let guard = target.clone();
    let refresh = with_store(shared, token, move |service| {
        service.sent_identity_refresh_uids(
            &guard.account_id,
            &guard.mailbox_source_id,
            &guard.folder_id,
            policy,
        )
    })
    .await?;
    if !refresh.is_empty() {
        let headers = session.fetch_headers(&refresh).await?;
        let records: Vec<_> = headers
            .iter()
            .map(|header| parse_header(header, now, compatibility))
            .collect();
        let guard = target.clone();
        written(
            with_store(shared, token, move |service| {
                service.store_sent_identity_batch(&guard, &records, &refresh, policy)
            })
            .await?,
            changed,
        )?;
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
    // Best effort: a server that answers a flags fetch with no data at all (2925)
    // must not fail the folder every round and keep the account reconnecting.
    // Those mails keep their last known state; the round still completes and the
    // next one tries again.
    round.unresolved_flags = refresh_cached_flags(
        shared,
        token,
        session,
        &target,
        &plan.refresh_flags,
        changed,
    )
    .await?;
    let guard = target;
    written(
        with_store(shared, token, move |service| {
            service.finish_backfill(&guard, capped)
        })
        .await?,
        &mut transient,
    )?;
    *changed |= !cursor.backfill_done || cursor.window_capped != capped;
    reconcile_cached_sent(shared, token, folder_id, changed).await?;
    memory.insert(
        folder_id.to_owned(),
        StatusSnapshot {
            status,
            reconciled_at: started,
        },
    );
    Ok(())
}

async fn refresh_cached_flags(
    shared: &std::sync::Arc<Shared>,
    token: &WorkerToken,
    session: &mut Session,
    target: &SyncTarget,
    uids: &[u32],
    changed: &mut bool,
) -> Result<usize, SyncError> {
    let mut unresolved = 0;
    for batch in uids.chunks(FLAGS_BATCH_SIZE) {
        let flags = session.fetch_flags(batch).await?;
        let fetched: HashSet<u32> = flags.iter().map(|(uid, _)| *uid).collect();
        store_flags(shared, token, target, flags, changed).await?;
        let missing: Vec<u32> = batch
            .iter()
            .copied()
            .filter(|uid| !fetched.contains(uid))
            .collect();
        if missing.is_empty() {
            continue;
        }

        // FETCH can omit a UID that was expunged since enumeration. Confirm
        // disappearance in the selected folder before changing local state.
        let present = session.search_uids(&missing).await?;
        let gone: Vec<u32> = missing
            .into_iter()
            .filter(|uid| present.binary_search(uid).is_err())
            .collect();
        if !gone.is_empty() {
            let guard = target.clone();
            written(
                with_store(shared, token, move |service| {
                    service.mark_deleted(&guard, &gone)
                })
                .await?,
                changed,
            )?;
        }
        for retry in present.chunks(FLAGS_RETRY_BATCH_SIZE) {
            let flags = session.retry_flags(retry).await?;
            let fetched: HashSet<u32> = flags.iter().map(|(uid, _)| *uid).collect();
            unresolved += retry.iter().filter(|uid| !fetched.contains(uid)).count();
            store_flags(shared, token, target, flags, changed).await?;
        }
    }
    Ok(unresolved)
}

async fn store_flags(
    shared: &std::sync::Arc<Shared>,
    token: &WorkerToken,
    target: &SyncTarget,
    flags: Vec<(u32, Vec<String>)>,
    changed: &mut bool,
) -> Result<(), SyncError> {
    let guard = target.clone();
    written(
        with_store(shared, token, move |service| {
            service.apply_flags(&guard, &flags)
        })
        .await?,
        changed,
    )
}

async fn reconcile_cached_sent(
    shared: &std::sync::Arc<Shared>,
    token: &WorkerToken,
    folder: &str,
    changed: &mut bool,
) -> Result<(), SyncError> {
    let (account, source, folder) = (
        token.account_id.clone(),
        token.source.clone(),
        folder.to_owned(),
    );
    written(
        with_store(shared, token, move |service| {
            service.reconcile_sent_folder(&account, &source, &folder)
        })
        .await?,
        changed,
    )
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
            history_target_date: None,
            history_covered_since: None,
            history_cursor_before: None,
            history_fetched: 0,
            history_total: None,
            history_state: "idle".into(),
            history_failure_code: None,
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
        let plan = plan_messages(&cursor(), &[], &[2, 98, 4, 98], &[]);
        assert_eq!(plan.fetch, [98, 4, 2]);
        assert!(plan.refresh_flags.is_empty());
        assert!(plan.deleted.is_empty());
        assert!(!plan.capped);
    }

    #[test]
    fn incremental_plan_does_not_skip_holes_below_the_high_water_mark() {
        let mut cursor = cursor();
        cursor.highest_uid = 100;
        let cached = vec![(100, "2026-10-17T00:00:00.000Z".into(), false)];
        let plan = plan_messages(&cursor, &cached, &[99, 100, 101], &[]);
        assert_eq!(plan.fetch, [101, 99]);
        assert_eq!(plan.refresh_flags, [100]);
    }

    #[test]
    fn capped_selection_never_implies_deletion_of_unselected_remote_mail() {
        let remote: Vec<_> = (1..=1001).collect();
        let cached = vec![(1, "2026-10-17T00:00:00.000Z".into(), false)];
        let plan = plan_messages(&cursor(), &cached, &remote, &[]);
        assert!(plan.capped);
        assert_eq!(plan.fetch.len(), WINDOW_LIMIT);
        assert_eq!(plan.fetch.first(), Some(&1001));
        assert_eq!(plan.fetch.last(), Some(&2));
        assert!(plan.deleted.is_empty());
    }

    #[test]
    fn deletion_only_reconciles_the_successfully_enumerated_window() {
        let cached = vec![
            (1, "2026-09-17T23:59:59.999Z".into(), false),
            (2, "2026-09-18T00:00:00.000Z".into(), false),
            (3, "2026-10-17T00:00:00.000Z".into(), false),
        ];
        let plan = plan_messages(&cursor(), &cached, &[3], &[]);
        assert_eq!(plan.deleted, [2]);
        assert_eq!(plan.refresh_flags, [3]);
        assert_eq!(plan_messages(&cursor(), &cached, &[], &[]).deleted, [2, 3]);
    }

    #[test]
    fn generation_change_uses_only_the_new_generations_cache() {
        // The store switches the generation and supplies no old present rows.
        let mut cursor = cursor();
        cursor.uid_validity = 43;
        let plan = plan_messages(&cursor, &[], &[1, 2], &[]);
        assert_eq!(plan.fetch, [2, 1]);
        assert!(plan.deleted.is_empty());
    }

    #[test]
    fn flagged_messages_outside_the_date_window_are_fetched_bounded_and_refreshed_on_unstar() {
        let cached = vec![
            (5, "2026-08-01T00:00:00.000Z".into(), true),
            (10, "2026-08-10T00:00:00.000Z".into(), true),
            (100, "2026-10-17T00:00:00.000Z".into(), false),
        ];
        // UID 5 was unstarred remotely (no longer in flagged_uids), UID 10 is still
        // starred, UID 20 is a newly discovered historical star outside the date window.
        let plan = plan_messages(&cursor(), &cached, &[100, 101], &[10, 20, 100]);
        assert_eq!(plan.fetch, [101, 20]);
        assert_eq!(plan.refresh_flags, [100, 10, 5]);
        assert!(
            plan.deleted.is_empty(),
            "unstarring an older message must never treat it as deleted"
        );
        assert!(!plan.capped);

        // Historical stars beyond FLAGGED_WINDOW_LIMIT keep the newest ones without
        // marking the ordinary date window as capped.
        let many_flagged: Vec<u32> = (1..=(FLAGGED_WINDOW_LIMIT as u32 + 25)).collect();
        let bounded = plan_messages(&cursor(), &[], &[10_000], &many_flagged);
        assert_eq!(bounded.fetch.len(), 1 + FLAGGED_WINDOW_LIMIT);
        assert_eq!(bounded.fetch.first(), Some(&10_000));
        assert_eq!(bounded.fetch.last(), Some(&26));
        assert!(!bounded.capped);
    }
}
