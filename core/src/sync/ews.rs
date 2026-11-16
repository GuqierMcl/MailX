//! EWS metadata sync uses the same worker, local cache, and revision events as IMAP.
//! Bodies remain on demand; a capped enumeration never proves a disappearance.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};

use super::{
    folders::FolderRecord,
    headers::MessageRecord,
    manager::{self, Outcome, Shared, SyncError, WorkerToken, with_store},
    messages::FolderRound,
};
use crate::{
    AppError, ErrorCode,
    accounts::{
        AccountService,
        sync_plan::{EwsPlan, PlanError},
    },
    ews::{EwsClient, EwsError, EwsFolder, EwsFolderRole, EwsItem},
    imap::ImapError,
    mail::FolderRole,
    storage::{
        ews::{EwsItem as RemoteItem, UID_VALIDITY},
        messages::{FolderSyncCursor, SyncTarget},
    },
};

const WINDOW_DAYS: i64 = 30;
const WINDOW_LIMIT: usize = 1_000;
const PAGE_SIZE: usize = 200;
// Allow smaller server pages without allowing a broken cursor to make an
// unlimited number of requests in one polling round.
const MAX_PAGES: usize = 20;

pub(super) async fn sync_once(shared: &Arc<Shared>, token: &WorkerToken) -> Outcome {
    match round(shared, token).await {
        Ok(outcome) => outcome,
        Err(error) => manager::from_sync_error(error),
    }
}

async fn round(shared: &Arc<Shared>, token: &WorkerToken) -> Result<Outcome, SyncError> {
    let id = token.account_id.clone();
    let plan = with_store(shared, token, move |service| Ok(service.ews_plan(&id))).await?;
    let plan = match plan {
        Ok(Some(plan)) if plan.mailbox_source_id == token.source => plan,
        Ok(_) => return Err(SyncError::Stale),
        Err(PlanError::CredentialMissing) => {
            return Ok(Outcome::Blocked(manager::failure(
                manager::SyncFailureCode::CredentialMissing,
                None,
                false,
            )));
        }
        Err(PlanError::CredentialUntrusted) => {
            return Ok(Outcome::Blocked(manager::failure(
                manager::SyncFailureCode::CredentialUntrusted,
                None,
                false,
            )));
        }
        Err(PlanError::Account(error)) => return Err(error.into()),
    };
    let mut client = EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password)
        .await
        .map_err(error)?;
    let folders = client.folders().await.map_err(error)?;
    let records = folder_records(&folders);
    let account = token.account_id.clone();
    let source = token.source.clone();
    let (targets, listed) = with_store(shared, token, move |service| {
        let shape = |folders: Vec<crate::mail::Folder>| {
            let mut shape: Vec<_> = folders
                .into_iter()
                .map(|folder| {
                    (
                        folder.id,
                        folder.parent_id,
                        folder.name,
                        folder.role,
                        folder.selectable,
                    )
                })
                .collect();
            shape.sort_by(|a, b| a.0.cmp(&b.0));
            shape
        };
        let before = shape(service.list_folders(&account)?);
        if !service.store_folders(&account, &source, &records)? {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let listed = before != shape(service.list_folders(&account)?);
        Ok((service.sync_folders(&account, &source)?, listed))
    })
    .await?;
    shared.reconcile_targets(token, &targets, listed);
    let counts: HashMap<_, _> = folders
        .into_iter()
        .map(|folder| (folder.id, (folder.unread, folder.total)))
        .collect();
    let mut failure = None;
    for (folder_id, remote_name) in targets {
        shared.folder_update(token, &folder_id, |folder| {
            folder.phase = manager::FolderSyncPhase::Syncing;
            folder.failure = None;
        });
        let mut round = FolderRound::default();
        let result = sync_folder(
            shared,
            token,
            &mut client,
            &folder_id,
            &remote_name,
            counts.get(&remote_name).copied(),
            &mut round,
        )
        .await;
        shared.announce(token, round.arrivals).await;
        match result {
            Ok(()) => shared.folder_update(token, &folder_id, |folder| {
                folder.phase = manager::FolderSyncPhase::Idle;
                folder.last_synced_at =
                    Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
                folder.failure = None;
                if round.changed {
                    folder.data_revision = folder.data_revision.saturating_add(1);
                }
            }),
            Err(error) => {
                let unusable = match &error {
                    SyncError::Imap(error) => !error.session_usable(),
                    _ => true,
                };
                let outcome = manager::from_sync_error(error);
                let next = match outcome {
                    Outcome::Transient(failure) | Outcome::Partial(failure) => failure,
                    Outcome::Blocked(failure) => {
                        shared.folder_update(token, &folder_id, |folder| {
                            folder.phase = manager::FolderSyncPhase::Failed;
                            folder.failure = Some(failure.clone());
                            folder.data_revision = folder.data_revision.saturating_add(1);
                        });
                        return Ok(Outcome::Blocked(failure));
                    }
                    Outcome::Gone => return Err(SyncError::Stale),
                    Outcome::Synced => unreachable!(),
                };
                shared.folder_update(token, &folder_id, |folder| {
                    folder.phase = manager::FolderSyncPhase::Failed;
                    folder.failure = Some(next.clone());
                    if round.changed {
                        folder.data_revision = folder.data_revision.saturating_add(1);
                    }
                });
                if unusable {
                    return Ok(Outcome::Transient(next));
                }
                failure.get_or_insert(next);
            }
        }
    }
    Ok(failure.map_or(Outcome::Synced, Outcome::Partial))
}

async fn sync_folder(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    client: &mut EwsClient,
    folder_id: &str,
    remote_name: &str,
    counts: Option<(u32, u32)>,
    round: &mut FolderRound,
) -> Result<(), SyncError> {
    let target = SyncTarget {
        account_id: token.account_id.clone(),
        mailbox_source_id: token.source.clone(),
        folder_id: folder_id.into(),
        uid_validity: UID_VALIDITY,
    };
    let now = Utc::now();
    let initial_since = (now.date_naive() - Duration::days(WINDOW_DAYS)).to_string();
    let guard = target.clone();
    let cursor = with_store(shared, token, move |service| {
        service.ensure_generation(&guard, &initial_since)
    })
    .await?
    .ok_or(SyncError::Stale)?;
    let since = NaiveDate::parse_from_str(&cursor.window_start, "%Y-%m-%d")
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?
        .and_utc();
    let (items, capped) = enumerate(client, remote_name, since).await?;
    let guard = target.clone();
    let stored = with_store(shared, token, move |service| {
        store_snapshot(service, &guard, &cursor, &items, capped, counts, now)
    })
    .await?;
    round.changed = stored.changed;
    round.arrivals = stored.arrivals;
    Ok(())
}

async fn enumerate(
    client: &mut EwsClient,
    folder: &str,
    since: DateTime<Utc>,
) -> Result<(Vec<EwsItem>, bool), SyncError> {
    let mut items = Vec::new();
    let mut ids = HashSet::new();
    let mut offset = 0;
    let mut scanned = 0;
    let mut pages = 0;
    loop {
        let page = client
            .items(folder, since, offset, PAGE_SIZE.min(WINDOW_LIMIT - scanned))
            .await
            .map_err(error)?;
        pages += 1;
        if page.items.is_empty() && page.next_offset.is_some() {
            return Err(
                ImapError::Protocol("Exchange returned an empty nonterminal page".into()).into(),
            );
        }
        scanned += page.items.len();
        for item in page.items {
            if ids.insert(item.id.clone()) {
                items.push(item);
            }
        }
        let Some(next) = page.next_offset else {
            return Ok((items, false));
        };
        if next <= offset {
            return Err(ImapError::Protocol("Exchange paging did not advance".into()).into());
        }
        if scanned >= WINDOW_LIMIT || pages >= MAX_PAGES {
            return Ok((items, true));
        }
        offset = next;
    }
}

fn store_snapshot(
    service: &mut AccountService,
    target: &SyncTarget,
    cursor: &FolderSyncCursor,
    items: &[EwsItem],
    capped: bool,
    counts: Option<(u32, u32)>,
    now: DateTime<Utc>,
) -> Result<FolderRound, AppError> {
    let mut round = FolderRound {
        changed: cursor.created,
        ..FolderRound::default()
    };
    let checked = |value: Option<usize>| value.ok_or_else(|| AppError::new(ErrorCode::Conflict));
    if let Some((unread, total)) = counts {
        round.changed |= checked(service.update_folder_counts(
            &target.account_id,
            &target.mailbox_source_id,
            &target.folder_id,
            unread,
            total,
        )?)? > 0;
    }
    let cached = service.cached_uids(target)?;
    let cached_ids: HashSet<_> = cached.iter().map(|(uid, _)| *uid).collect();
    let keys: Vec<_> = items
        .iter()
        .map(|item| (item.id.clone(), item.change_key.clone()))
        .collect();
    let assigned = service
        .ews_assign(target, &keys)?
        .ok_or_else(|| AppError::new(ErrorCode::Conflict))?;
    // Only a complete enumeration proves a remote item disappeared. The
    // capped newest page says nothing about cached messages beyond it.
    if !capped {
        let remote: HashSet<_> = items.iter().map(|item| item.id.as_str()).collect();
        let mut deleted = Vec::new();
        for (uid, received) in &cached {
            if received.as_str() >= cursor.window_start.as_str()
                && let Some(item) = service.ews_item(target, *uid)?
                && !remote.contains(item.item_id.as_str())
            {
                deleted.push(*uid);
            }
        }
        round.changed |= checked(service.mark_deleted(target, &deleted)?)? > 0;
    }
    let new: Vec<_> = items
        .iter()
        .zip(&assigned)
        .filter(|(_, assigned)| !cached_ids.contains(&assigned.uid))
        .map(|(item, assigned)| message_record(assigned.uid, item, now))
        .collect();
    let flags: Vec<_> = items
        .iter()
        .zip(&assigned)
        .map(|(item, assigned)| (assigned.uid, flags(item)))
        .collect();
    let highest = assigned
        .iter()
        .map(|item| item.uid)
        .max()
        .unwrap_or(0)
        .max(cursor.highest_uid);
    checked(service.begin_backfill(target, capped)?)?;
    // Existing messages are refreshed through apply_flags, which preserves a
    // local pending change; store_message_batch is for new/revived records.
    if !new.is_empty() {
        round.changed |= checked(service.store_message_batch(target, &new, highest)?)? > 0;
    }
    round.changed |= checked(service.apply_flags(target, &flags)?)? > 0;
    let mut previews = HashMap::new();
    for (item, mapped) in items.iter().zip(&assigned) {
        if let Some(preview) = item.preview.as_deref()
            && let Some(id) = service.ews_message_id(target, mapped.uid)?
        {
            previews.insert(id, crate::mail::render::preview(Some(preview)));
        }
    }
    if !previews.is_empty() {
        let ids = previews.keys().cloned().collect::<Vec<_>>();
        let outcomes: Vec<_> = service
            .preview_targets(&ids)?
            .into_iter()
            .map(|identity| {
                let preview = previews
                    .remove(&identity.message_id)
                    .filter(|value| !value.is_empty());
                crate::storage::previews::PreviewOutcome { identity, preview }
            })
            .collect();
        round.changed |= !service.preview_commit(&outcomes)?.is_empty();
    }
    checked(service.finish_backfill(target, capped)?)?;
    round.changed |= !cursor.backfill_done || cursor.window_capped != capped;
    round.changed |= checked(service.reconcile_sent_folder(
        &target.account_id,
        &target.mailbox_source_id,
        &target.folder_id,
    )?)? > 0;
    if !cursor.created && (cursor.backfill_done || cursor.highest_uid > 0) {
        let arrived: Vec<_> = new
            .iter()
            .filter(|item| item.uid > cursor.highest_uid)
            .map(|item| item.uid)
            .collect();
        round.arrivals = service.new_arrivals(
            target,
            &arrived,
            &(now - Duration::hours(24)).to_rfc3339_opts(SecondsFormat::Millis, true),
        )?;
    }
    Ok(round)
}

fn flags(item: &EwsItem) -> Vec<String> {
    let mut flags = Vec::new();
    if item.is_read {
        flags.push("\\Seen".into());
    }
    if item.is_flagged {
        flags.push("\\Flagged".into());
    }
    flags
}

fn identifier(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('<').trim_end_matches('>');
    (!value.is_empty() && value.len() <= 998 && !value.chars().any(char::is_whitespace))
        .then(|| value.to_owned())
}

fn message_record(uid: u32, item: &EwsItem, now: DateTime<Utc>) -> MessageRecord {
    MessageRecord {
        uid,
        message_id_header: item.internet_message_id.as_deref().and_then(identifier),
        original_message_id: None,
        sent_identity_policy: crate::providers::for_imap_host("").policy_key(),
        in_reply_to: item.in_reply_to.as_deref().and_then(identifier),
        references_header: item
            .references
            .as_deref()
            .map(|value| {
                value
                    .split_whitespace()
                    .filter_map(identifier)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|value| !value.is_empty()),
        subject: item.subject.clone(),
        from_name: item.from.as_ref().and_then(|from| from.name.clone()),
        from_address: item
            .from
            .as_ref()
            .map(|from| from.address.clone())
            .unwrap_or_default(),
        to_addresses: item.to.clone(),
        cc_addresses: item.cc.clone(),
        reply_to_addresses: item.reply_to.clone(),
        received_at: item
            .received_at
            .unwrap_or(now)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        sent_at: item
            .sent_at
            .map(|sent| sent.to_rfc3339_opts(SecondsFormat::Millis, true)),
        size: u32::try_from(item.size).ok(),
        is_read: item.is_read,
        is_flagged: item.is_flagged,
        flags: flags(item),
        has_attachment: item.has_attachments,
    }
}

fn folder_records(folders: &[EwsFolder]) -> Vec<FolderRecord> {
    let ids: HashSet<_> = folders.iter().map(|folder| folder.id.as_str()).collect();
    let parents: HashMap<_, _> = folders
        .iter()
        .map(|folder| {
            (
                folder.id.as_str(),
                folder
                    .parent_id
                    .as_deref()
                    .filter(|parent| ids.contains(parent)),
            )
        })
        .collect();
    folders
        .iter()
        .map(|folder| FolderRecord {
            remote_name: folder.id.clone(),
            parent_remote_name: {
                let direct = parents[folder.id.as_str()];
                let mut parent = direct;
                let mut visited = HashSet::from([folder.id.as_str()]);
                let mut valid = true;
                while let Some(id) = parent {
                    if !visited.insert(id) {
                        valid = false;
                        break;
                    }
                    parent = parents.get(id).copied().flatten();
                }
                if valid {
                    direct.map(str::to_owned)
                } else {
                    None
                }
            },
            name: folder.name.clone(),
            delimiter: None,
            role: match folder.role {
                Some(EwsFolderRole::Inbox) => FolderRole::Inbox,
                Some(EwsFolderRole::Sent) => FolderRole::Sent,
                Some(EwsFolderRole::Drafts) => FolderRole::Drafts,
                Some(EwsFolderRole::Deleted) => FolderRole::Trash,
                Some(EwsFolderRole::Junk) => FolderRole::Junk,
                None => FolderRole::Other,
            },
            selectable: true,
            attributes: Vec::new(),
            status: None,
        })
        .collect()
}

pub(crate) fn error(error: EwsError) -> ImapError {
    match error {
        EwsError::Net(error) => ImapError::Net(error),
        EwsError::Timeout => ImapError::Timeout,
        EwsError::Authentication => {
            ImapError::Authentication("Exchange refused the sign-in".into())
        }
        EwsError::AuthUnsupported => {
            ImapError::Unsupported("Exchange requires a supported NTLM sign-in".into())
        }
        EwsError::TooLarge => ImapError::TooLarge,
        EwsError::NotFound => ImapError::Mailbox("Exchange item no longer exists".into()),
        EwsError::Server { .. } => ImapError::Mailbox("Exchange rejected the request".into()),
        _ => ImapError::Protocol("Exchange request failed".into()),
    }
}

pub(crate) async fn fetch_mime(
    plan: &EwsPlan,
    item: &RemoteItem,
    limit: usize,
) -> Result<Option<Vec<u8>>, ImapError> {
    let mut client = EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password)
        .await
        .map_err(error)?;
    match client
        .mime(&item.item_id, item.change_key.as_deref(), limit)
        .await
    {
        Ok(raw) => Ok(Some(raw)),
        Err(EwsError::NotFound) => Ok(None),
        Err(failure) => Err(error(failure)),
    }
}

#[cfg(test)]
pub(crate) mod tests;
