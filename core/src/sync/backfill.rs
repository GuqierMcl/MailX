//! Background historical backfill engine for IMAP and EWS folders.
//!
//! Hot-window synchronization (the most recent 30 days plus all starred mail)
//! always takes priority; historical backfill runs inside the account's existing
//! sync worker between hot rounds, yielding immediately on IDLE or manual wake,
//! without opening extra connections, emitting new-mail notifications, or
//! turning sidebar account spinners on.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{
    ews,
    headers::parse_header,
    manager::{
        self, Outcome, Shared, SyncError, SyncFailure, SyncFailureCode, SyncWake, WorkerToken,
        with_store,
    },
};
use crate::{
    AppError, ErrorCode,
    accounts::sync_plan::PlanError,
    ews::{EwsClient, EwsItem},
    imap::{self, ImapError, Session},
    storage::{
        ews::UID_VALIDITY as EWS_UID_VALIDITY,
        messages::{BackfillFolderJob, SyncTarget},
    },
};

const BACKFILL_BATCH_SIZE: usize = 200;
const BACKFILL_SLICE_BUDGET: Duration = Duration::from_secs(60);
const REVISION_BUMP_INTERVAL: Duration = Duration::from_secs(2);

struct RevisionThrottle<'a> {
    shared: &'a Arc<Shared>,
    token: &'a WorkerToken,
    folder_id: &'a str,
    last_bump: Option<Instant>,
    pending: bool,
}

impl<'a> RevisionThrottle<'a> {
    fn new(shared: &'a Arc<Shared>, token: &'a WorkerToken, folder_id: &'a str) -> Self {
        Self {
            shared,
            token,
            folder_id,
            last_bump: None,
            pending: false,
        }
    }

    fn record(&mut self) {
        if self
            .last_bump
            .is_none_or(|last| last.elapsed() >= REVISION_BUMP_INTERVAL)
        {
            self.shared.bump_folder_revision(self.token, self.folder_id);
            self.last_bump = Some(Instant::now());
            self.pending = false;
        } else {
            self.pending = true;
        }
    }

    fn flush(&mut self) {
        if self.pending {
            self.shared.bump_folder_revision(self.token, self.folder_id);
            self.last_bump = Some(Instant::now());
            self.pending = false;
        }
    }
}

impl Drop for RevisionThrottle<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BackfillPhase {
    Running,
    Paused,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BackfillControlAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountBackfillTask {
    pub account_id: String,
    pub phase: BackfillPhase,
    pub target_date: Option<String>,
    pub covered_since: Option<String>,
    #[ts(type = "number")]
    pub fetched: u32,
    #[ts(type = "number | null")]
    pub total: Option<u32>,
    pub active_folder_name: Option<String>,
    pub failure: Option<SyncFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BackfillTurn {
    Done,
    Interrupted,
    ContinueSoon,
}

pub(super) enum FolderTurn {
    Completed,
    Stopped,
    Interrupted,
    ContinueSoon,
}

fn job_target(job: &BackfillFolderJob) -> SyncTarget {
    SyncTarget {
        account_id: job.account_id.clone(),
        mailbox_source_id: job.mailbox_source_id.clone(),
        folder_id: job.folder_id.clone(),
        uid_validity: job.uid_validity,
    }
}

fn failure_code_from_error(error: SyncError) -> Option<SyncFailureCode> {
    match manager::from_sync_error(error) {
        Outcome::Blocked(f) | Outcome::Transient(f) | Outcome::Partial(f) => Some(f.code),
        Outcome::Synced | Outcome::Gone => None,
    }
}

fn parse_date_field(value: &str) -> Result<NaiveDate, SyncError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible).into())
}

fn date_utc_start(date: NaiveDate) -> Result<DateTime<Utc>, SyncError> {
    Ok(date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?
        .and_utc())
}

async fn next_job(
    shared: &Arc<Shared>,
    token: &WorkerToken,
) -> Result<Option<BackfillFolderJob>, SyncError> {
    let account = token.account_id.clone();
    let source = token.source.clone();
    with_store(shared, token, move |service| {
        service.next_backfill_folder(&account, &source)
    })
    .await
}

async fn record_failure(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    job: &BackfillFolderJob,
    code: SyncFailureCode,
) {
    let target = job_target(job);
    let _ = with_store(shared, token, move |service| {
        service.fail_backfill_folder(&target, code)
    })
    .await;
    shared.emit_backfill_tasks();
}

/// Runs up to `BACKFILL_SLICE_BUDGET` of historical backfill for the account
/// between hot rounds. Returns cleanly without opening a network connection
/// when no folder is queued.
pub(super) async fn run_budgeted(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    wake: &SyncWake,
) -> BackfillTurn {
    let first = match next_job(shared, token).await {
        Ok(Some(job)) => job,
        Ok(None) | Err(SyncError::Stale | SyncError::Closed) => return BackfillTurn::Done,
        Err(error) => {
            let _ = error;
            return BackfillTurn::Done;
        }
    };
    if wake.has_pending_hot_wake() {
        return BackfillTurn::Interrupted;
    }
    if token.is_ews {
        run_ews(shared, token, wake, first).await
    } else {
        run_imap(shared, token, wake, first).await
    }
}

async fn run_imap(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    wake: &SyncWake,
    first: BackfillFolderJob,
) -> BackfillTurn {
    let id = token.account_id.clone();
    let plan = match with_store(shared, token, move |service| Ok(service.sync_plan(&id))).await {
        Ok(Ok(plan)) => plan,
        Ok(Err(PlanError::CredentialMissing)) => {
            record_failure(shared, token, &first, SyncFailureCode::CredentialMissing).await;
            return BackfillTurn::Done;
        }
        Ok(Err(PlanError::CredentialUntrusted)) => {
            record_failure(shared, token, &first, SyncFailureCode::CredentialUntrusted).await;
            return BackfillTurn::Done;
        }
        Ok(Err(PlanError::Account(error))) => {
            if let Some(code) = failure_code_from_error(error.into()) {
                record_failure(shared, token, &first, code).await;
            }
            return BackfillTurn::Done;
        }
        Err(error) => {
            if let Some(code) = failure_code_from_error(error) {
                record_failure(shared, token, &first, code).await;
            }
            return BackfillTurn::Done;
        }
    };
    if plan.mailbox_source_id != token.source {
        return BackfillTurn::Done;
    }
    let mut session = match imap::open_session(&plan.route, &plan.login).await {
        Ok(session) => session,
        Err(error) => {
            if let Some(code) = failure_code_from_error(error.into()) {
                record_failure(shared, token, &first, code).await;
            }
            return BackfillTurn::Done;
        }
    };
    let started = Instant::now();
    let mut did_work = false;
    let mut current = Some(first);
    while let Some(job) = current.take() {
        if wake.has_pending_hot_wake() {
            let _ = session.logout().await;
            return BackfillTurn::Interrupted;
        }
        if did_work && started.elapsed() >= BACKFILL_SLICE_BUDGET {
            let _ = session.logout().await;
            return BackfillTurn::ContinueSoon;
        }
        match backfill_imap_folder(
            shared,
            token,
            wake,
            &mut session,
            &job,
            started,
            &mut did_work,
        )
        .await
        {
            Ok(FolderTurn::Completed) => {}
            Ok(FolderTurn::Stopped) => {
                let _ = session.logout().await;
                return BackfillTurn::Done;
            }
            Ok(FolderTurn::Interrupted) => {
                let _ = session.logout().await;
                return BackfillTurn::Interrupted;
            }
            Ok(FolderTurn::ContinueSoon) => {
                let _ = session.logout().await;
                return BackfillTurn::ContinueSoon;
            }
            Err(error) => {
                if let Some(code) = failure_code_from_error(error) {
                    record_failure(shared, token, &job, code).await;
                }
                return BackfillTurn::Done;
            }
        }
        current = match next_job(shared, token).await {
            Ok(job) => job,
            Err(_) => return BackfillTurn::Done,
        };
    }
    let _ = session.logout().await;
    BackfillTurn::Done
}

pub(super) async fn backfill_imap_folder(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    wake: &SyncWake,
    session: &mut Session,
    job: &BackfillFolderJob,
    started: Instant,
    did_work: &mut bool,
) -> Result<FolderTurn, SyncError> {
    let target = job_target(job);
    if job.history_covered_since.as_deref() == Some("all") {
        let guard = target.clone();
        with_store(shared, token, move |service| {
            service.complete_backfill_folder(&guard, "all")
        })
        .await?;
        shared.emit_backfill_tasks();
        return Ok(FolderTurn::Completed);
    }

    let since_date = if job.history_target_date == "all" {
        None
    } else {
        Some(parse_date_field(&job.history_target_date)?)
    };
    let upper_str = job
        .history_covered_since
        .as_deref()
        .unwrap_or(&job.window_start);
    let upper_date = parse_date_field(upper_str)?;
    let before_date = upper_date + ChronoDuration::days(1);

    let mailbox = session.examine(&job.remote_name).await?;
    if mailbox.uid_validity != job.uid_validity {
        let next_target = SyncTarget {
            uid_validity: mailbox.uid_validity,
            ..target.clone()
        };
        let window_start = job.window_start.clone();
        with_store(shared, token, move |service| {
            service.ensure_generation(&next_target, &window_start)
        })
        .await?;
        shared.emit_backfill_tasks();
        return Ok(FolderTurn::ContinueSoon);
    }

    let remote_uids = session
        .search_history(since_date, before_date, mailbox)
        .await?;
    let guard = target.clone();
    let cached = with_store(shared, token, move |service| service.cached_uids(&guard)).await?;
    let cached_ids: HashSet<u32> = cached.iter().map(|(uid, _, _)| *uid).collect();
    let mut uids: Vec<u32> = remote_uids
        .into_iter()
        .filter(|uid| !cached_ids.contains(uid))
        .collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.dedup();

    if uids.is_empty() {
        let guard = target.clone();
        let target_date = job.history_target_date.clone();
        with_store(shared, token, move |service| {
            service.complete_backfill_folder(&guard, &target_date)
        })
        .await?;
        shared.emit_backfill_tasks();
        return Ok(FolderTurn::Completed);
    }

    let estimated_total = job.history_total.unwrap_or_else(|| {
        job.history_fetched
            .saturating_add(u32::try_from(uids.len()).unwrap_or(u32::MAX))
    });
    if job.history_total != Some(estimated_total) {
        let guard = target.clone();
        with_store(shared, token, move |service| {
            service.update_backfill_progress(&guard, 0, Some(estimated_total), None, None)?;
            Ok(())
        })
        .await?;
        shared.emit_backfill_tasks();
    }
    let compatibility = session.compatibility();
    let mut highest_uid = job.highest_uid;
    let mut revision_throttle = RevisionThrottle::new(shared, token, &job.folder_id);

    for batch in uids.chunks(BACKFILL_BATCH_SIZE) {
        if wake.has_pending_hot_wake() {
            return Ok(FolderTurn::Interrupted);
        }
        if *did_work && started.elapsed() >= BACKFILL_SLICE_BUDGET {
            return Ok(FolderTurn::ContinueSoon);
        }
        let Some(active) = next_job(shared, token).await? else {
            return Ok(FolderTurn::Stopped);
        };
        if active.folder_id != job.folder_id {
            return Ok(FolderTurn::ContinueSoon);
        }

        let headers = session.fetch_headers(batch).await?;
        let now = Utc::now();
        let records: Vec<_> = headers
            .iter()
            .map(|header| parse_header(header, now, compatibility))
            .collect();
        highest_uid = highest_uid.max(records.iter().map(|record| record.uid).max().unwrap_or(0));
        let fetched_count = u32::try_from(records.len()).unwrap_or(u32::MAX);
        let guard = target.clone();
        let changed = with_store(shared, token, move |service| {
            let stored = if records.is_empty() {
                0
            } else {
                service
                    .store_message_batch(&guard, &records, highest_uid)?
                    .unwrap_or(0)
            };
            service.update_backfill_progress(
                &guard,
                fetched_count,
                Some(estimated_total),
                None,
                None,
            )?;
            Ok(stored > 0)
        })
        .await?;
        *did_work = true;
        if changed {
            revision_throttle.record();
        }
        shared.emit_backfill_tasks();
    }

    let guard = target.clone();
    let target_date = job.history_target_date.clone();
    with_store(shared, token, move |service| {
        service.complete_backfill_folder(&guard, &target_date)
    })
    .await?;
    revision_throttle.flush();
    shared.emit_backfill_tasks();
    Ok(FolderTurn::Completed)
}

async fn run_ews(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    wake: &SyncWake,
    first: BackfillFolderJob,
) -> BackfillTurn {
    let id = token.account_id.clone();
    let plan = match with_store(shared, token, move |service| Ok(service.ews_plan(&id))).await {
        Ok(Ok(Some(plan))) if plan.mailbox_source_id == token.source => plan,
        Ok(Ok(_)) | Err(SyncError::Stale | SyncError::Closed) => return BackfillTurn::Done,
        Ok(Err(PlanError::CredentialMissing)) => {
            record_failure(shared, token, &first, SyncFailureCode::CredentialMissing).await;
            return BackfillTurn::Done;
        }
        Ok(Err(PlanError::CredentialUntrusted)) => {
            record_failure(shared, token, &first, SyncFailureCode::CredentialUntrusted).await;
            return BackfillTurn::Done;
        }
        Ok(Err(PlanError::Account(error))) => {
            if let Some(code) = failure_code_from_error(error.into()) {
                record_failure(shared, token, &first, code).await;
            }
            return BackfillTurn::Done;
        }
        Err(error) => {
            if let Some(code) = failure_code_from_error(error) {
                record_failure(shared, token, &first, code).await;
            }
            return BackfillTurn::Done;
        }
    };
    let mut client =
        match EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password)
            .await
            .map_err(ews::error)
        {
            Ok(client) => client,
            Err(error) => {
                if let Some(code) = failure_code_from_error(error.into()) {
                    record_failure(shared, token, &first, code).await;
                }
                return BackfillTurn::Done;
            }
        };

    let started = Instant::now();
    let mut did_work = false;
    let mut current = Some(first);
    while let Some(job) = current.take() {
        if wake.has_pending_hot_wake() {
            return BackfillTurn::Interrupted;
        }
        if did_work && started.elapsed() >= BACKFILL_SLICE_BUDGET {
            return BackfillTurn::ContinueSoon;
        }
        match backfill_ews_folder(
            shared,
            token,
            wake,
            &mut client,
            &job,
            started,
            &mut did_work,
        )
        .await
        {
            Ok(FolderTurn::Completed) => {}
            Ok(FolderTurn::Stopped) => return BackfillTurn::Done,
            Ok(FolderTurn::Interrupted) => return BackfillTurn::Interrupted,
            Ok(FolderTurn::ContinueSoon) => return BackfillTurn::ContinueSoon,
            Err(error) => {
                if let Some(code) = failure_code_from_error(error) {
                    record_failure(shared, token, &job, code).await;
                }
                return BackfillTurn::Done;
            }
        }
        current = match next_job(shared, token).await {
            Ok(job) => job,
            Err(_) => return BackfillTurn::Done,
        };
    }
    BackfillTurn::Done
}

pub(super) async fn backfill_ews_folder(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    wake: &SyncWake,
    client: &mut EwsClient,
    job: &BackfillFolderJob,
    started: Instant,
    did_work: &mut bool,
) -> Result<FolderTurn, SyncError> {
    let target = SyncTarget {
        uid_validity: EWS_UID_VALIDITY,
        ..job_target(job)
    };
    if job.history_covered_since.as_deref() == Some("all") {
        let guard = target.clone();
        with_store(shared, token, move |service| {
            service.complete_backfill_folder(&guard, "all")
        })
        .await?;
        shared.emit_backfill_tasks();
        return Ok(FolderTurn::Completed);
    }

    let since = if job.history_target_date == "all" {
        None
    } else {
        Some(date_utc_start(parse_date_field(&job.history_target_date)?)?)
    };
    let mut batch_before = match job.history_cursor_before.as_deref() {
        Some(ts) => DateTime::parse_from_rfc3339(ts)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?,
        None => {
            let upper_str = job
                .history_covered_since
                .as_deref()
                .unwrap_or(&job.window_start);
            let upper = parse_date_field(upper_str)?;
            date_utc_start(upper + ChronoDuration::days(1))?
        }
    };

    let mut offset = 0usize;
    let mut highest_uid = job.highest_uid;
    let mut revision_throttle = RevisionThrottle::new(shared, token, &job.folder_id);

    loop {
        if wake.has_pending_hot_wake() {
            return Ok(FolderTurn::Interrupted);
        }
        if *did_work && started.elapsed() >= BACKFILL_SLICE_BUDGET {
            return Ok(FolderTurn::ContinueSoon);
        }
        let Some(active) = next_job(shared, token).await? else {
            return Ok(FolderTurn::Stopped);
        };
        if active.folder_id != job.folder_id {
            return Ok(FolderTurn::ContinueSoon);
        }

        let page = client
            .items_range(
                &job.remote_name,
                since,
                Some(batch_before),
                offset,
                BACKFILL_BATCH_SIZE,
            )
            .await
            .map_err(ews::error)?;
        if page.scanned == 0 && page.next_offset.is_some() {
            return Err(
                ImapError::Protocol("Exchange returned an empty nonterminal page".into()).into(),
            );
        }

        let oldest_dt = page.items.iter().filter_map(|item| item.received_at).min();
        let guard = target.clone();
        let ids: Vec<String> = page.items.iter().map(|item| item.id.clone()).collect();
        let already_cached = with_store(shared, token, move |service| {
            let id_refs: Vec<_> = ids.iter().map(String::as_str).collect();
            service.ews_cached_items(&guard, &id_refs)
        })
        .await?;

        let mut fetch_items: Vec<EwsItem> = page
            .items
            .iter()
            .filter(|item| !already_cached.contains(&item.id))
            .cloned()
            .collect();
        if !fetch_items.is_empty() {
            client
                .recipients(&mut fetch_items)
                .await
                .map_err(ews::error)?;
        }
        let mut full_by_id: HashMap<String, EwsItem> = fetch_items
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let mut items = page.items;
        for item in &mut items {
            if let Some(full) = full_by_id.remove(&item.id) {
                *item = full;
            }
        }

        let page_total = page
            .total_in_view
            .and_then(|v| u32::try_from(v).ok())
            .map(|view_total| job.history_fetched.saturating_add(view_total));
        let now = Utc::now();
        let finished = page.next_offset.is_none();
        let covered_str = oldest_dt.map(|dt| {
            let date = dt.date_naive().to_string();
            if job.history_target_date != "all" && date < job.history_target_date {
                job.history_target_date.clone()
            } else {
                date
            }
        });
        let strict_older = oldest_dt.filter(|dt| *dt < batch_before);
        let next_cursor_before =
            strict_older.map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true));
        let guard = target.clone();
        let target_date = job.history_target_date.clone();
        let (stored_any, next_highest) = with_store(shared, token, move |service| {
            let existing_uids: HashSet<u32> = service
                .cached_uids(&guard)?
                .into_iter()
                .map(|(uid, _, _)| uid)
                .collect();
            let keys: Vec<_> = items
                .iter()
                .map(|item| (item.id.clone(), item.change_key.clone()))
                .collect();
            let assigned = service
                .ews_assign(&guard, &keys)?
                .ok_or_else(|| AppError::new(ErrorCode::Conflict))?;
            let records: Vec<_> = items
                .iter()
                .zip(&assigned)
                .filter(|(item, mapped)| {
                    !already_cached.contains(&item.id) && !existing_uids.contains(&mapped.uid)
                })
                .map(|(item, mapped)| ews::message_record(mapped.uid, item, now))
                .collect();
            let max_uid = highest_uid.max(assigned.iter().map(|m| m.uid).max().unwrap_or(0));
            let stored = if records.is_empty() {
                0
            } else {
                service
                    .store_message_batch(&guard, &records, max_uid)?
                    .unwrap_or(0)
            };
            let delta = u32::try_from(records.len()).unwrap_or(u32::MAX);
            service.update_backfill_progress(
                &guard,
                delta,
                page_total,
                covered_str.as_deref(),
                next_cursor_before.as_deref(),
            )?;
            if finished {
                service.complete_backfill_folder(&guard, &target_date)?;
            }
            Ok((stored > 0, max_uid))
        })
        .await?;

        highest_uid = next_highest;
        *did_work = true;
        if stored_any {
            revision_throttle.record();
        }
        if finished {
            revision_throttle.flush();
        }
        shared.emit_backfill_tasks();

        let Some(next_off) = page.next_offset else {
            return Ok(FolderTurn::Completed);
        };
        if let Some(older) = strict_older {
            batch_before = older;
            offset = 0;
        } else {
            offset = next_off;
        }
    }
}
