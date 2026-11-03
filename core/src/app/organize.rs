//! Batched local mail changes shared by UI and tool callers.
use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use super::{FlagChange, MailxCore};
use crate::events::{CoreEvent, MailChangeSource, MailStateChangeKind, MailStateChanged};
use crate::storage::archives::{ArchiveEffects, ArchiveEntry, ArchiveKey};
use crate::{AppError, ErrorCode};

const UNDO_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_UNDO_RECEIPTS: usize = 100;

/// Each requested field has its own result: read can be queued while a star is
/// local-only, and a later field's failure does not erase earlier success.
#[derive(Clone, Debug)]
pub struct EmailFlagResult {
    pub email_id: String,
    pub is_read: Option<Result<FlagChange, AppError>>,
    pub is_starred: Option<Result<FlagChange, AppError>>,
}

#[derive(Clone, Debug, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOutcome {
    pub processed_count: u32,
    pub changed_count: u32,
    /// For the trusted host UI only. Tool results must not expose this token.
    pub undo_token: Option<String>,
}

struct UndoReceipt {
    token: String,
    created: Instant,
    keys: BTreeSet<ArchiveKey>,
    entries: Vec<ArchiveEntry>,
}

#[derive(Default)]
pub(super) struct ArchiveUndoRegistry {
    receipts: VecDeque<UndoReceipt>,
}

impl ArchiveUndoRegistry {
    fn prune(&mut self) {
        self.receipts
            .retain(|r| r.created.elapsed() < UNDO_LIFETIME);
    }

    fn invalidate(&mut self, keys: &BTreeSet<ArchiveKey>) {
        self.prune();
        self.receipts.retain(|r| r.keys.is_disjoint(keys));
    }

    fn add(&mut self, entries: Vec<ArchiveEntry>) -> String {
        while self.receipts.len() >= MAX_UNDO_RECEIPTS {
            self.receipts.pop_front();
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.receipts.push_back(UndoReceipt {
            token: token.clone(),
            created: Instant::now(),
            keys: entries.iter().map(|entry| entry.key.clone()).collect(),
            entries,
        });
        token
    }
}

impl From<crate::storage::LocalChange> for FlagChange {
    fn from(change: crate::storage::LocalChange) -> Self {
        match change {
            crate::storage::LocalChange::Unchanged => Self::Unchanged,
            crate::storage::LocalChange::Queued => Self::Queued,
            crate::storage::LocalChange::LocalOnly => Self::LocalOnly,
        }
    }
}

impl MailxCore {
    /// Each field is committed independently, with bounded per-item results.
    /// The account service lock is held for the batch; notifications and
    /// background write-back wakeups occur only after it is released.
    pub fn set_email_flags(
        &self,
        email_ids: &[String],
        is_read: Option<bool>,
        is_starred: Option<bool>,
        source: MailChangeSource,
    ) -> Result<Vec<EmailFlagResult>, AppError> {
        if email_ids.is_empty()
            || email_ids.len() > 500
            || (is_read.is_none() && is_starred.is_none())
        {
            return Err(AppError::new(ErrorCode::Validation));
        }
        let mut service = self.accounts()?;
        let mut results = Vec::with_capacity(email_ids.len());
        let mut affected = ArchiveEffects::default();
        let mut wake = BTreeSet::new();
        for id in email_ids {
            let mut result = EmailFlagResult {
                email_id: id.clone(),
                is_read: None,
                is_starred: None,
            };
            let summary = service.get_email(id).map(|detail| detail.summary);
            let mut changed = false;
            for (wanted, flag, output) in [
                (is_read, crate::storage::Flag::Seen, &mut result.is_read),
                (
                    is_starred,
                    crate::storage::Flag::Flagged,
                    &mut result.is_starred,
                ),
            ] {
                let Some(wanted) = wanted else {
                    continue;
                };
                let outcome = match &summary {
                    Ok(summary) if !summary.local_only => service
                        .set_message_flag(id, flag, wanted)
                        .map(FlagChange::from),
                    Ok(_) => Err(AppError::new(ErrorCode::NotFound)),
                    Err(error) => Err(error.clone()),
                };
                if let (Ok(summary), Ok(outcome)) = (&summary, &outcome) {
                    if *outcome == FlagChange::Queued {
                        wake.insert(summary.account_id.clone());
                    }
                    if *outcome != FlagChange::Unchanged {
                        changed = true;
                    }
                }
                *output = Some(outcome);
            }
            if changed {
                let summary = summary.as_ref().expect("a successful change had a summary");
                if !affected.account_ids.contains(&summary.account_id) {
                    affected.account_ids.push(summary.account_id.clone());
                }
                if !affected.email_ids.contains(id) {
                    affected.email_ids.push(id.clone());
                    affected.changed_count += 1;
                }
            }
            results.push(result);
        }
        drop(service);
        for account in wake {
            self.sync.changes_waiting(&account);
        }
        self.emit_mail_state(affected, source, MailStateChangeKind::Flags, None);
        Ok(results)
    }

    /// Change all archive targets atomically. The receipt counts visible
    /// messages changed, including copies sharing the same archive key.
    pub fn change_email_archive(
        &self,
        email_ids: &[String],
        archive: bool,
        source: MailChangeSource,
    ) -> Result<ArchiveOutcome, AppError> {
        // Every archive and restoration path takes these locks in this order.
        let mut service = self.accounts()?;
        let mut registry = self
            .archive_undo
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let changed = service.change_archive(email_ids, archive)?;
        // A later explicit operation, even a no-op, supersedes old undo intent.
        registry.invalidate(&changed.touched_keys);
        let undo_token = if !changed.entries.is_empty() {
            Some(registry.add(changed.entries))
        } else {
            None
        };
        let outcome = ArchiveOutcome {
            processed_count: changed.processed_count,
            changed_count: changed.effects.changed_count,
            undo_token: undo_token.clone(),
        };
        drop(registry);
        drop(service);
        self.emit_mail_state(
            changed.effects,
            source,
            if archive {
                MailStateChangeKind::Archive
            } else {
                MailStateChangeKind::Unarchive
            },
            undo_token,
        );
        Ok(outcome)
    }

    /// Trusted host action: restores only this operation's changed keys. A
    /// later operation on any of them invalidates the entire short-lived token.
    pub fn undo_archive(&self, token: &str) -> Result<(), AppError> {
        if uuid::Uuid::parse_str(token).is_err() {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        let mut service = self.accounts()?;
        let mut registry = self
            .archive_undo
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        registry.prune();
        let receipt = registry
            .receipts
            .iter()
            .find(|r| r.token == token)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let affected = service.restore_archive(&receipt.entries)?;
        let keys = receipt.keys.clone();
        registry.invalidate(&keys);
        drop(registry);
        drop(service);
        self.emit_mail_state(
            affected,
            MailChangeSource::User,
            MailStateChangeKind::UndoArchive,
            None,
        );
        Ok(())
    }

    fn emit_mail_state(
        &self,
        effects: ArchiveEffects,
        source: MailChangeSource,
        kind: MailStateChangeKind,
        undo_token: Option<String>,
    ) {
        if effects.changed_count != 0 {
            self.events
                .emit(&CoreEvent::MailStateChanged(MailStateChanged {
                    account_ids: effects.account_ids,
                    email_ids: effects.email_ids,
                    source,
                    kind,
                    changed_count: effects.changed_count,
                    undo_token,
                }));
        }
    }
}

#[cfg(test)]
mod tests;
