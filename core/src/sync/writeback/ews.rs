//! EWS executes the existing flag/deletion queues and reports their normal outcomes.
use super::*;
use crate::{
    accounts::sync_plan::EwsPlan,
    ews::{EwsClient, EwsError},
    storage::{ews::EwsItem, messages::SyncTarget},
};

pub(super) async fn run(
    service: &Arc<Mutex<AccountService>>,
    account: &str,
    plan: EwsPlan,
    changes: Vec<PendingChange>,
    deletions: Vec<PendingDeletion>,
    report: WriteBackReport,
) -> RoundEnd {
    let queued = changes.len() + deletions.len();
    let mut client =
        match EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password).await {
            Ok(client) => client,
            Err(error) => return failed(error, queued),
        };
    push(
        service,
        account,
        &plan.mailbox_source_id,
        &mut client,
        changes,
        deletions,
        report,
    )
    .await
}

async fn push(
    service: &Arc<Mutex<AccountService>>,
    account: &str,
    source: &str,
    client: &mut EwsClient,
    changes: Vec<PendingChange>,
    deletions: Vec<PendingDeletion>,
    mut report: WriteBackReport,
) -> RoundEnd {
    let mut groups: BTreeMap<(String, u32), Vec<PendingChange>> = BTreeMap::new();
    for change in changes {
        groups
            .entry((change.folder_id.clone(), change.uid))
            .or_default()
            .push(change);
    }
    for (_, changes) in groups {
        let first = &changes[0];
        let target = SyncTarget {
            account_id: first.account_id.clone(),
            mailbox_source_id: first.mailbox_source_id.clone(),
            folder_id: first.folder_id.clone(),
            uid_validity: first.uid_validity,
        };
        let item = match lookup(service, target, first.uid).await {
            Ok(Some(item)) => item,
            // A stale/missing mapping must never become a write against an IMAP uid.
            _ => {
                for change in &changes {
                    let _ = service.retry(change);
                }
                report.waiting += changes.len();
                continue;
            }
        };
        let read = changes
            .iter()
            .find(|change| change.flag == Flag::Seen)
            .map(|change| change.wanted);
        let starred = changes
            .iter()
            .find(|change| change.flag == Flag::Flagged)
            .map(|change| change.wanted);
        match client
            .set_flags(&item.item_id, item.change_key.as_deref(), read, starred)
            .await
        {
            Ok(()) => {
                for change in &changes {
                    // Queue::finish confirms the exact value sent; a newer
                    // toggle made during this request stays queued.
                    if service.finish(change).is_ok() {
                        report.sent += 1;
                    } else {
                        report.waiting += 1;
                    }
                }
            }
            Err(EwsError::Authentication | EwsError::AuthUnsupported) => return RoundEnd::Blocked,
            Err(_) => {
                for change in &changes {
                    let _ = service.retry(change);
                }
                report.waiting += changes.len();
            }
        }
    }
    // Keep the shared undo deadline, claims, retry cap and tombstone cleanup.
    for deletion in deletions {
        let ids = vec![deletion.message_id.clone()];
        let target = SyncTarget {
            account_id: account.to_owned(),
            mailbox_source_id: source.to_owned(),
            folder_id: deletion.folder_id.clone(),
            uid_validity: deletion.uid_validity,
        };
        let item = match lookup(service, target, deletion.uid).await {
            Ok(Some(item)) => item,
            Ok(None) => {
                deletion_impossible(service.as_ref(), &ids, &mut report);
                continue;
            }
            Err(_) => {
                report.waiting += 1;
                continue;
            }
        };
        match client
            .delete(
                &item.item_id,
                item.change_key.as_deref(),
                deletion.target_remote_name.is_none(),
            )
            .await
        {
            Ok(()) | Err(EwsError::NotFound) => match service.deleted(&ids) {
                Ok(()) => report.deleted += 1,
                Err(_) => report.waiting += 1,
            },
            Err(EwsError::Authentication | EwsError::AuthUnsupported) => return RoundEnd::Blocked,
            Err(_) => deletion_refused(service.as_ref(), &ids, &mut report),
        }
    }
    report.next_deletion_in = next_deletion_in(service, account).await;
    if report.waiting > 0 {
        RoundEnd::Retry(report)
    } else {
        RoundEnd::Done(report)
    }
}

async fn lookup(
    service: &Arc<Mutex<AccountService>>,
    target: SyncTarget,
    uid: u32,
) -> Result<Option<EwsItem>, AppError> {
    let service = service.clone();
    tokio::task::spawn_blocking(move || {
        service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .ews_item(&target, uid)
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal))?
}

fn failed(error: EwsError, waiting: usize) -> RoundEnd {
    match error {
        EwsError::Authentication | EwsError::AuthUnsupported => RoundEnd::Blocked,
        error => failed_round(crate::sync::ews::error(error), waiting),
    }
}

#[cfg(test)]
mod tests;
