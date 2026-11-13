//! One polling round for an on-premises Exchange (EWS) account.
//!
//! EWS has no UIDs, no IDLE and no partial fetch. ItemIds are translated into
//! stable local uids by `storage/ews.rs`, and whole MIME messages are fetched
//! and rendered during the round so the on-demand body and preview readers
//! never open an IMAP session for this account. The worker reuses the manager's
//! cadence, generation guards and outcome handling; only the protocol differs.
use std::sync::Arc;

use chrono::{Duration, Utc};

use super::manager::{Outcome, Shared, SyncError, WorkerToken, from_sync_error, with_store};
use crate::accounts::sync_plan::PlanError;
use crate::error::{AppError, ErrorCode};
use crate::ews::http::HttpError;
use crate::ews::soap::{Item, Party};
use crate::ews::{EwsClient, EwsError};
use crate::imap::{FolderStatus, ImapError};
use crate::mail::{FolderRole, MailAddress};
use crate::storage::Flag;
use crate::storage::messages::SyncTarget;
use crate::sync::folders::FolderRecord;
use crate::sync::headers::MessageRecord;

/// EWS answers up to this many messages per folder per round. Folders with
/// more mail fill in from the newest end on later rounds.
const ITEMS_PER_FOLDER: usize = 500;
/// Only headers inside the initial sync window are worth caching.
const WINDOW_DAYS: i64 = 30;
/// EWS ItemIds are opaque; the local identity uses a fixed generation.
const UID_VALIDITY: u32 = 1;

pub(super) async fn sync_once(shared: &Arc<Shared>, token: &WorkerToken) -> Outcome {
    match round(shared, token).await {
        Ok(()) => Outcome::Synced,
        Err(error) => from_sync_error(error),
    }
}

async fn round(shared: &Arc<Shared>, token: &WorkerToken) -> Result<(), SyncError> {
    let id = token.account_id.clone();
    let planned = with_store(shared, token, move |service| Ok(service.ews_plan(&id))).await?;
    let plan = match planned {
        Ok(Some(plan)) => plan,
        // The account changed away from EWS while its worker was running.
        Ok(None) => return Err(SyncError::Stale),
        Err(PlanError::Account(error)) => return Err(SyncError::Storage(error)),
        Err(PlanError::CredentialUntrusted | PlanError::CredentialMissing) => {
            return Err(SyncError::Imap(ImapError::Authentication(
                "the saved EWS credential cannot be used".into(),
            )));
        }
    };
    if plan.mailbox_source_id != token.source {
        return Err(SyncError::Stale);
    }
    let mut client = EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password)
        .await
        .map_err(ews_failure)?;

    let folders = client.folders().await.map_err(ews_failure)?;
    let records: Vec<FolderRecord> = folders
        .iter()
        .map(|folder| FolderRecord {
            remote_name: folder.id.clone(),
            parent_remote_name: None,
            name: folder.display_name.clone(),
            delimiter: None,
            role: role_of(&folder.display_name),
            selectable: true,
            attributes: Vec::new(),
            status: Some(FolderStatus {
                messages: folder.total_count,
                unseen: 0,
                uid_next: None,
                uid_validity: Some(UID_VALIDITY),
            }),
        })
        .collect();

    let account_id = token.account_id.clone();
    let source = token.source.clone();
    let targets = with_store(shared, token, move |service| {
        let listed = records;
        if !service.store_folders(&account_id, &source, &listed)? {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        service.sync_folders(&account_id, &source)
    })
    .await?;
    shared.reconcile_targets(token, &targets, true);

    for (folder_id, remote_name) in &targets {
        sync_folder(shared, token, &mut client, folder_id, remote_name).await?;
    }
    write_back(shared, token, &mut client).await?;
    Ok(())
}

async fn sync_folder(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    client: &mut EwsClient,
    folder_id: &str,
    remote_name: &str,
) -> Result<(), SyncError> {
    let items = client
        .items(remote_name, ITEMS_PER_FOLDER)
        .await
        .map_err(ews_failure)?;
    let account_id = token.account_id.clone();
    let source = token.source.clone();
    let folder = folder_id.to_owned();
    let remote = remote_name.to_owned();
    let keys: Vec<(String, Option<String>)> = items
        .iter()
        .map(|item| (item.id.clone(), item.change_key.clone()))
        .collect();
    let now = Utc::now();
    let window_start = (now.date_naive() - Duration::days(WINDOW_DAYS)).to_string();
    let pending = with_store(
        shared,
        token,
        move |service| -> Result<Vec<(String, String, Option<String>)>, AppError> {
            let assigned = service.ews_assign(&account_id, &remote, &keys)?;
            let target = SyncTarget {
                account_id: account_id.clone(),
                mailbox_source_id: source.clone(),
                folder_id: folder.clone(),
                uid_validity: UID_VALIDITY,
            };
            let cursor = service
                .ensure_generation(&target, &window_start)?
                .ok_or_else(|| AppError::new(ErrorCode::Conflict))?;
            let by_uid: std::collections::HashMap<u32, &Item> = items
                .iter()
                .zip(assigned.iter())
                .map(|(item, mapped)| (mapped.uid, item))
                .collect();
            let records: Vec<MessageRecord> = assigned
                .iter()
                .filter_map(|mapped| {
                    by_uid
                        .get(&mapped.uid)
                        .map(|item| message_record(mapped.uid, item))
                })
                .collect();
            let flags: Vec<(u32, Vec<String>)> = records
                .iter()
                .map(|record| (record.uid, record.flags.clone()))
                .collect();
            let highest_uid = assigned
                .iter()
                .map(|mapped| mapped.uid)
                .max()
                .unwrap_or(0)
                .max(cursor.highest_uid);
            service.begin_backfill(&target, false)?;
            service.store_message_batch(&target, &records, highest_uid)?;
            service.apply_flags(&target, &flags)?;
            service.finish_backfill(&target, false)?;
            // Only messages whose body is still missing need a GetItem round.
            let mut missing = Vec::new();
            for mapped in &assigned {
                if let Some(message_id) =
                    service.ews_message_id(&account_id, &folder, UID_VALIDITY, mapped.uid)?
                    && !service.ews_body_ready(&message_id)?
                {
                    missing.push((
                        message_id,
                        mapped.item_id.clone(),
                        mapped.change_key.clone(),
                    ));
                }
            }
            Ok(missing)
        },
    )
    .await?;

    for (message_id, item_id, change_key) in pending {
        let raw = client
            .mime(&item_id, change_key.as_deref())
            .await
            .map_err(ews_failure)?;
        let account_id = token.account_id.clone();
        with_store(shared, token, move |service| {
            service.ews_cache_body(&message_id, &raw)?;
            let _ = &account_id;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Applies local read/starred changes through UpdateItem. The IMAP flag
/// writer is not started for EWS accounts, so this is the only path.
async fn write_back(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    client: &mut EwsClient,
) -> Result<(), SyncError> {
    let account_id = token.account_id.clone();
    let changes = with_store(shared, token, move |service| {
        service.pending_flag_changes(&account_id, 64)
    })
    .await?;
    for change in changes {
        let account_id = token.account_id.clone();
        let found = with_store(shared, token, move |service| {
            service.ews_item(&account_id, &change.remote_name, change.uid)
        })
        .await?;
        let Some((item_id, change_key)) = found else {
            continue;
        };
        let (read, flagged) = match change.flag {
            Flag::Seen => (Some(change.wanted), None),
            Flag::Flagged => (None, Some(change.wanted)),
        };
        let result = client
            .set_flags(&item_id, change_key.as_deref(), read, flagged)
            .await;
        let message_id = change.message_id.clone();
        let flag = change.flag;
        let sent = result.is_ok();
        with_store(shared, token, move |service| {
            if sent {
                service.finish_flag_change(&message_id, flag, true)?;
            } else {
                service.retry_flag_change(&message_id, flag)?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

fn message_record(uid: u32, item: &Item) -> MessageRecord {
    let mut flags = Vec::new();
    if item.is_read {
        flags.push("\\Seen".to_owned());
    }
    if item.is_flagged {
        flags.push("\\Flagged".to_owned());
    }
    MessageRecord {
        uid,
        message_id_header: item.message_id.clone(),
        original_message_id: None,
        sent_identity_policy: "default:v2",
        in_reply_to: item.in_reply_to.clone(),
        references_header: item.references.clone(),
        subject: item.subject.clone(),
        from_name: item.from.as_ref().and_then(|party| party.name.clone()),
        from_address: item
            .from
            .as_ref()
            .map(|party| party.address.clone())
            .unwrap_or_default(),
        to_addresses: parties(&item.to),
        cc_addresses: parties(&item.cc),
        reply_to_addresses: parties(&item.reply_to),
        received_at: utc(item.received_at.as_deref()),
        sent_at: item.sent_at.as_deref().map(|value| utc(Some(value))),
        size: item.size,
        is_read: item.is_read,
        is_flagged: item.is_flagged,
        flags,
        has_attachment: item.has_attachments,
    }
}

fn parties(values: &[Party]) -> Vec<MailAddress> {
    values
        .iter()
        .map(|party| MailAddress {
            name: party.name.clone(),
            address: party.address.clone(),
        })
        .collect()
}

fn utc(value: Option<&str>) -> String {
    value
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|date| date.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn role_of(display_name: &str) -> FolderRole {
    let name = display_name.to_ascii_lowercase();
    if name == "inbox" || name.contains("收件") {
        FolderRole::Inbox
    } else if name.contains("sent") || name.contains("已发送") || name.contains("已发") {
        FolderRole::Sent
    } else if name.contains("draft") || name.contains("草稿") {
        FolderRole::Drafts
    } else if name.contains("trash") || name.contains("deleted") || name.contains("已删除") {
        FolderRole::Trash
    } else if name.contains("junk") || name.contains("spam") || name.contains("垃圾") {
        FolderRole::Junk
    } else if name.contains("archive") || name.contains("存档") {
        FolderRole::Archive
    } else {
        FolderRole::Other
    }
}

fn ews_failure(error: EwsError) -> SyncError {
    match error {
        EwsError::Http(HttpError::Rejected) => SyncError::Imap(ImapError::Authentication(
            "Exchange refused the sign-in".into(),
        )),
        EwsError::Http(HttpError::NoNtlm) => SyncError::Imap(ImapError::ProviderPolicy),
        _ => SyncError::Imap(ImapError::Protocol("EWS request failed".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_folder_names_get_roles() {
        assert_eq!(role_of("Inbox"), FolderRole::Inbox);
        assert_eq!(role_of("Sent Items"), FolderRole::Sent);
        assert_eq!(role_of("已发送邮件"), FolderRole::Sent);
        assert_eq!(role_of("Drafts"), FolderRole::Drafts);
        assert_eq!(role_of("Deleted Items"), FolderRole::Trash);
        assert_eq!(role_of("Junk Email"), FolderRole::Junk);
        assert_eq!(role_of("Archive"), FolderRole::Archive);
        assert_eq!(role_of("Projects"), FolderRole::Other);
    }

    #[test]
    fn ews_dates_become_the_same_format_as_imap_headers() {
        let value = utc(Some("2026-11-12T08:30:00Z"));
        assert_eq!(value, "2026-11-12T08:30:00.000Z");
        assert!(utc(None).ends_with('Z'));
        assert!(utc(Some("not a date")).ends_with('Z'));
    }

    #[test]
    fn a_rejection_is_not_retried_as_a_network_error() {
        let error = ews_failure(EwsError::Http(HttpError::Rejected));
        assert!(matches!(
            error,
            SyncError::Imap(ImapError::Authentication(_))
        ));
    }
}
