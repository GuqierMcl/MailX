//! Send queue operations on the account store (docs/architecture/sending.md
//! sections 5, 6 and 10). The worker and the host-facing methods of
//! `MailxCore` both go through here, under the store lock, in short
//! transactions; no network I/O happens in this file.
use chrono::{DateTime, Utc};

use super::{AccountService, new_id, validate_uuid};
use crate::{
    AppError, Draft, ErrorCode,
    accounts::sync_plan::PlanError,
    send::types::*,
    smtp::{
        SmtpPlan,
        attachment::AttachmentData,
        message::{BuiltMessage, OutgoingMessage, build, local_now, new_message_id},
    },
    storage::{
        attachments::AttachmentFile,
        bodies::{BlobStore, StoredBlob},
        outbox::{Changed, Claim, ClaimedJob, FrozenDraft, NewOutbox},
    },
};

/// What the worker found when it looked at the queue.
pub(crate) enum Next {
    /// The account is gone.
    Gone,
    /// Nothing is due now; look again at this time, if anything is queued.
    Wait(Option<DateTime<Utc>>),
    /// An item moved to `sending`; attempt it with this plan.
    Job {
        job: Box<ClaimedJob>,
        plan: Result<SmtpPlan, PlanError>,
    },
}

pub(crate) struct Step {
    /// State changes made while looking, for the host.
    pub changed: Vec<Changed>,
    pub next: Next,
}

fn canonical(request: &QueueSendRequest) -> Result<String, AppError> {
    serde_json::to_string(&serde_json::json!({
        "op": "queueSend", "draftId": request.draft_id, "revision": request.expected_revision,
    }))
    .map_err(|_| AppError::new(ErrorCode::Internal))
}

pub(crate) const UNDO_CHOICES: [u32; 5] = [0, 5, 10, 20, 30];

/// Phase one of queueing, read under the store lock.
pub(crate) enum SendPlan {
    /// The same request already queued an item.
    Replay(Box<OutboxItem>),
    Build(Box<SendBuild>),
}

/// Everything needed to build the snapshot without holding the store lock.
pub(crate) struct SendBuild {
    draft: Draft,
    address: String,
    name: Option<String>,
    message_id: String,
    canonical: String,
    files: Vec<AttachmentFile>,
    blobs: BlobStore,
}

/// Phase two: the MIME bytes, already written to the blob store.
pub(crate) struct Frozen {
    built: BuiltMessage,
    snapshot: StoredBlob,
}

impl SendBuild {
    /// Reads the attachment blobs (hash verified), builds the MIME and stores
    /// it as a blob. Touches no database, so it runs outside the store lock.
    pub(crate) fn freeze(&self) -> Result<Frozen, AppError> {
        let attachments = self
            .files
            .iter()
            .map(|file| {
                Ok(AttachmentData {
                    file_name: file.summary.file_name.clone(),
                    media_type: file.summary.media_type.clone(),
                    bytes: self.blobs.read(&file.blob)?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let built = build(&OutgoingMessage {
            sender_address: &self.address,
            sender_name: self.name.as_deref(),
            draft: &self.draft,
            message_id: &self.message_id,
            date: local_now(),
            attachments: &attachments,
        })?;
        let snapshot = self.blobs.put(&built.mime)?;
        Ok(Frozen { built, snapshot })
    }
}

impl AccountService {
    /// Snapshots the draft into the queue and locks it, as [`Core::queue_send`]
    /// does in three phases. Returns the item and
    /// whether this call created it (a replay creates nothing).
    #[cfg(test)]
    pub(crate) fn queue_send(
        &mut self,
        request: &QueueSendRequest,
        undo_send_seconds: u32,
        now: DateTime<Utc>,
    ) -> Result<(OutboxItem, Option<DraftChanged>), AppError> {
        match self.prepare_send(request, undo_send_seconds)? {
            SendPlan::Replay(item) => Ok((*item, None)),
            SendPlan::Build(build) => {
                let frozen = build.freeze()?;
                self.commit_send(request, undo_send_seconds, &build, frozen, now)
            }
        }
    }

    /// Validates the request and gathers what the snapshot is built from.
    pub(crate) fn prepare_send(
        &self,
        request: &QueueSendRequest,
        undo_send_seconds: u32,
    ) -> Result<SendPlan, AppError> {
        validate_uuid(&request.draft_id, "draftId")?;
        validate_uuid(&request.client_mutation_id, "clientMutationId")?;
        if !UNDO_CHOICES.contains(&undo_send_seconds) {
            return Err(AppError::validation("undoSendSeconds"));
        }
        let canonical = canonical(request)?;
        if let Some((item, saved)) = self
            .storage
            .outbox_by_mutation(&request.client_mutation_id)?
        {
            return if saved == canonical {
                Ok(SendPlan::Replay(Box::new(item)))
            } else {
                Err(AppError::new(ErrorCode::IdempotencyConflict))
            };
        }
        let draft = self.storage.get_draft(&request.draft_id)?;
        if draft.revision != request.expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        if self.storage.draft_locked(&draft.id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        let (address, name) = self.storage.account_sender(&draft.account_id)?;
        let message_id = new_message_id(&address)?;
        let files = self.storage.attachment_files(&draft.id)?;
        Ok(SendPlan::Build(Box::new(SendBuild {
            draft,
            address,
            name,
            message_id,
            canonical,
            files,
            blobs: self.storage.blobs.clone(),
        })))
    }

    /// Inserts the queue item and locks the draft in one transaction, which
    /// fails with `conflict` if the draft changed since [`Self::prepare_send`].
    pub(crate) fn commit_send(
        &mut self,
        request: &QueueSendRequest,
        undo_send_seconds: u32,
        build: &SendBuild,
        frozen: Frozen,
        now: DateTime<Utc>,
    ) -> Result<(OutboxItem, Option<DraftChanged>), AppError> {
        let Frozen { built, snapshot } = frozen;
        let draft = &build.draft;
        let preview: Vec<String> = built.envelope.recipients.iter().take(3).cloned().collect();
        let id = new_id();
        let item = self.storage.insert_outbox(&NewOutbox {
            id: &id,
            draft_id: Some(&draft.id),
            account_id: &draft.account_id,
            client_mutation_id: &request.client_mutation_id,
            canonical: &build.canonical,
            message_id: &built.message_id,
            snapshot: &snapshot,
            frozen: Some(&FrozenDraft {
                revision: draft.revision,
                attachment_ids: build.files.iter().map(|f| f.summary.id.clone()).collect(),
            }),
            envelope: &built.envelope,
            subject: &draft.subject,
            recipients_preview: &preview,
            not_before: now + chrono::Duration::seconds(i64::from(undo_send_seconds)),
            now,
        })?;
        let locked = DraftChanged {
            draft_id: draft.id.clone(),
            account_id: item.account_id.clone(),
            change: DraftChange::Locked,
        };
        Ok((item, Some(locked)))
    }

    pub(crate) fn get_outbox(&self, id: &str) -> Result<OutboxItem, AppError> {
        validate_uuid(id, "outboxId")?;
        self.storage.get_outbox(id)
    }

    pub(crate) fn list_outbox(
        &self,
        account_id: Option<&str>,
        include_all: bool,
    ) -> Result<Vec<OutboxItem>, AppError> {
        if let Some(account_id) = account_id {
            validate_uuid(account_id, "accountId")?;
        }
        self.storage.list_outbox(account_id, include_all)
    }

    pub(crate) fn cancel_outbox(&self, id: &str, now: DateTime<Utc>) -> Result<Changed, AppError> {
        validate_uuid(id, "outboxId")?;
        self.storage.cancel_queued(id, now)
    }

    pub(crate) fn retry_outbox_now(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutboxItem, AppError> {
        validate_uuid(id, "outboxId")?;
        self.storage.retry_now(id, now)
    }

    /// Settles an `unknown` item. Replaying a mutation id returns the same
    /// result; the same id for another choice is a conflict.
    pub(crate) fn resolve_unknown(
        &mut self,
        request: &ResolveUnknownRequest,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        validate_uuid(&request.id, "outboxId")?;
        validate_uuid(&request.client_mutation_id, "clientMutationId")?;
        let wanted = match request.resolution {
            UnknownResolution::MarkSent => OutboxResolution::MarkedSent,
            UnknownResolution::Resend => OutboxResolution::Resent,
        };
        let current = self.storage.get_outbox(&request.id)?;
        if let Some(settled) = current.resolution {
            return match self
                .storage
                .resolution_replay(&request.id, &request.client_mutation_id)?
            {
                Some(_) if settled == wanted => Ok(None),
                _ => Err(AppError::new(ErrorCode::Conflict)),
            };
        }
        let changed = match request.resolution {
            UnknownResolution::MarkSent => {
                self.storage
                    .mark_sent(&request.id, &request.client_mutation_id, now)?
            }
            UnknownResolution::Resend => self.storage.resend_unknown(
                &request.id,
                &request.client_mutation_id,
                &new_id(),
                &canonical_resend(&request.id)?,
                now,
            )?,
        };
        Ok(Some(changed))
    }

    /// The item a settled resolution led to, for replays.
    pub(crate) fn resolved_item(
        &self,
        request: &ResolveUnknownRequest,
    ) -> Result<OutboxItem, AppError> {
        self.storage
            .resolution_replay(&request.id, &request.client_mutation_id)?
            .ok_or_else(|| AppError::new(ErrorCode::Conflict))
    }

    pub(crate) fn recover_outbox(&mut self, now: DateTime<Utc>) -> Result<Vec<Changed>, AppError> {
        self.storage.recover_outbox(now)
    }

    /// One look at an account's queue: fail items whose sender changed, then
    /// take the next due one and resolve its login plan.
    pub(crate) fn outbox_step(
        &mut self,
        account_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Step, AppError> {
        let address = match self.storage.account_sender(account_id) {
            Ok((address, _)) => address,
            Err(error) if error.code == ErrorCode::NotFound => {
                return Ok(Step {
                    changed: Vec::new(),
                    next: Next::Gone,
                });
            }
            Err(error) => return Err(error),
        };
        let mut changed = self
            .storage
            .fail_changed_sender(account_id, &address, now)?;
        let (claim, claimed) = self.storage.claim_next(account_id, now)?;
        changed.extend(claimed);
        let next = match claim {
            Claim::Wait(at) => Next::Wait(at),
            Claim::Job(job) => {
                let plan = self.smtp_plan(account_id);
                Next::Job { job, plan }
            }
        };
        Ok(Step { changed, next })
    }

    pub(crate) fn outbox_data_started(&self, id: &str, now: DateTime<Utc>) -> Result<(), AppError> {
        self.storage.mark_data_started(id, now)
    }

    pub(crate) fn outbox_requeue(
        &self,
        id: &str,
        at: DateTime<Utc>,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.storage.requeue(id, at, error, now)
    }

    pub(crate) fn outbox_fail(
        &self,
        id: &str,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.storage.fail(id, error, now)
    }

    pub(crate) fn outbox_unknown(
        &self,
        id: &str,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.storage.mark_unknown(id, error, now)
    }

    pub(crate) fn outbox_accept(
        &self,
        id: &str,
        reply: &ServerReplyInfo,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.storage.accept(id, reply, now)
    }
}

fn canonical_resend(id: &str) -> Result<String, AppError> {
    serde_json::to_string(&serde_json::json!({"op": "resend", "outboxId": id}))
        .map_err(|_| AppError::new(ErrorCode::Internal))
}

#[cfg(test)]
impl AccountService {
    pub(crate) fn outbox_mutation_id(&self, id: &str) -> Result<String, AppError> {
        Ok(self.storage.db.query_row(
            "SELECT client_mutation_id FROM outbox WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?)
    }

    /// Puts queued items into `sending` as a crashed worker would have left
    /// them; the flag says whether the body start mark was written.
    pub(crate) fn force_sending_for_tests(
        &mut self,
        items: &[(&str, bool)],
        now: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let stamp = crate::send::clock::stamp(now);
        for (id, started) in items {
            self.storage.db.execute(
                "UPDATE outbox SET state = 'sending', attempts = 1,
                    data_started_at = CASE WHEN ?2 THEN ?3 END WHERE id = ?1",
                rusqlite::params![id, started, stamp],
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn outbox_mime(&self, id: &str) -> Result<Vec<u8>, AppError> {
        let (hash, size): (String, i64) = self.storage.db.query_row(
            "SELECT snapshot_hash, snapshot_size FROM outbox WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        self.storage.blobs.read(&StoredBlob {
            sha256: hash,
            size: u64::try_from(size).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeZone;
    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::accounts::sync_plan::tests::config;
    use crate::drafts::types::Draft;
    use crate::secrets::test_support::MemorySecrets;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    fn secs(seconds: i64) -> chrono::Duration {
        chrono::Duration::seconds(seconds)
    }

    fn id() -> String {
        Uuid::new_v4().to_string()
    }

    fn service(directory: &TempDir) -> AccountService {
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap()
    }

    fn account(service: &mut AccountService, email: &str) -> String {
        let mut value = config(json!({"kind":"direct"}));
        value["emailAddress"] = json!(email);
        service
            .create(serde_json::from_value(json!({"requestId": id(), "config": value})).unwrap())
            .unwrap()
            .id
    }

    fn draft(service: &mut AccountService, account_id: &str) -> Draft {
        service
            .create_draft(
                serde_json::from_value(json!({
                    "accountId": account_id,
                    "to": [{"name": null, "address": "li@example.org"}],
                    "cc": [], "bcc": [], "subject": "hi", "bodyHtml": "hello",
                    "clientMutationId": id(),
                }))
                .unwrap(),
                "",
            )
            .unwrap()
    }

    fn request(draft: &Draft) -> QueueSendRequest {
        QueueSendRequest {
            draft_id: draft.id.clone(),
            expected_revision: draft.revision,
            client_mutation_id: id(),
        }
    }

    fn is_job(step: &Step) -> bool {
        matches!(step.next, Next::Job { .. })
    }

    #[test]
    fn cancel_and_start_cannot_both_win() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");

        // The worker claims first: cancel loses.
        let first = draft(&mut service, &account_id);
        let (item, _) = service.queue_send(&request(&first), 0, t0()).unwrap();
        assert!(is_job(&service.outbox_step(&account_id, t0()).unwrap()));
        assert_eq!(
            service.cancel_outbox(&item.id, t0()).unwrap_err().code,
            ErrorCode::AlreadySending
        );
        assert_eq!(
            service.get_outbox(&item.id).unwrap().state,
            OutboxState::Sending
        );
        assert_eq!(
            service.retry_outbox_now(&item.id, t0()).unwrap_err().code,
            ErrorCode::AlreadySending
        );
    }

    #[test]
    fn a_canceled_item_is_never_claimed() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let draft = draft(&mut service, &account_id);
        let (item, _) = service.queue_send(&request(&draft), 0, t0()).unwrap();
        let changed = service.cancel_outbox(&item.id, t0()).unwrap();
        assert_eq!(changed.item.state, OutboxState::Canceled);
        assert!(matches!(
            service.outbox_step(&account_id, t0()).unwrap().next,
            Next::Wait(None)
        ));
        // Canceling again, or retrying a canceled item, is a conflict.
        assert_eq!(
            service.cancel_outbox(&item.id, t0()).unwrap_err().code,
            ErrorCode::Conflict
        );
    }

    #[test]
    fn a_locked_draft_cannot_be_edited_or_deleted_until_released() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let draft = draft(&mut service, &account_id);
        let (item, _) = service.queue_send(&request(&draft), 30, t0()).unwrap();
        let edit = || {
            serde_json::from_value::<crate::drafts::types::UpdateDraftRequest>(json!({
                "id": draft.id, "expectedRevision": draft.revision, "accountId": null,
                "to": [{"name": null, "address": "li@example.org"}], "cc": [], "bcc": [],
                "subject": "changed", "bodyHtml": "x",
            }))
            .unwrap()
        };
        assert_eq!(
            service.update_draft(edit()).unwrap_err().code,
            ErrorCode::DraftLocked
        );
        assert_eq!(
            service.delete_draft(&draft.id).unwrap_err().code,
            ErrorCode::DraftLocked
        );
        // A second queue item for the same draft is refused as well.
        assert_eq!(
            service
                .queue_send(&request(&draft), 0, t0())
                .unwrap_err()
                .code,
            ErrorCode::DraftLocked
        );
        let changed = service.cancel_outbox(&item.id, t0()).unwrap();
        assert_eq!(changed.draft.unwrap().change, DraftChange::Unlocked);
        assert_eq!(service.update_draft(edit()).unwrap().revision, 2);
    }

    #[test]
    fn recovery_requeues_without_a_body_mark_and_marks_unknown_with_one() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let (a, b) = (
            draft(&mut service, &account_id),
            draft(&mut service, &account_id),
        );
        let (not_started, _) = service.queue_send(&request(&a), 0, t0()).unwrap();
        let (started, _) = service.queue_send(&request(&b), 0, t0() + secs(1)).unwrap();
        for _ in 0..2 {
            assert!(is_job(
                &service.outbox_step(&account_id, t0() + secs(2)).unwrap()
            ));
        }
        service
            .outbox_data_started(&started.id, t0() + secs(3))
            .unwrap();

        let changed = service.recover_outbox(t0() + secs(10)).unwrap();
        assert_eq!(changed.len(), 2);
        let queued = service.get_outbox(&not_started.id).unwrap();
        assert_eq!(queued.state, OutboxState::Queued);
        assert_eq!(queued.attempts, 0, "the interrupted try is not counted");
        assert_eq!(queued.data_started_at, None);
        let unknown = service.get_outbox(&started.id).unwrap();
        assert_eq!(unknown.state, OutboxState::Unknown);
        assert_eq!(unknown.attempts, 1);
        assert!(unknown.data_started_at.is_some());
        assert_eq!(unknown.error.unwrap().reason, "interrupted");
        // Idempotent.
        assert!(service.recover_outbox(t0() + secs(20)).unwrap().is_empty());
        // The unknown item still locks its draft.
        assert!(service.storage.draft_locked(&b.id).unwrap());
    }

    #[test]
    fn deleting_the_account_removes_its_queue_and_drafts() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let draft = draft(&mut service, &account_id);
        service.queue_send(&request(&draft), 0, t0()).unwrap();
        // Account removal is not an operation yet; the connection rows have no
        // cascade of their own, so clear them as a future delete would.
        let db = &service.storage.db;
        for table in [
            "create_requests",
            "credential_journal",
            "mail_connections",
            "network_routes",
            "auth_profiles",
        ] {
            db.execute(
                &format!("DELETE FROM {table} WHERE account_id = ?1"),
                [&account_id],
            )
            .unwrap();
        }
        db.execute("DELETE FROM accounts WHERE id = ?1", [&account_id])
            .unwrap();
        let count: i64 = service
            .storage
            .db
            .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(matches!(
            service.outbox_step(&account_id, t0()).unwrap().next,
            Next::Gone
        ));
    }

    #[test]
    fn the_outbox_row_survives_its_draft() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let draft = draft(&mut service, &account_id);
        let (item, _) = service.queue_send(&request(&draft), 0, t0()).unwrap();
        assert!(is_job(&service.outbox_step(&account_id, t0()).unwrap()));
        let reply = ServerReplyInfo {
            code: 250,
            text: "ok".into(),
        };
        let changed = service
            .outbox_accept(&item.id, &reply, t0() + secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(changed.draft.unwrap().change, DraftChange::Deleted);
        let kept = service.get_outbox(&item.id).unwrap();
        assert_eq!(kept.state, OutboxState::Accepted);
        assert_eq!(kept.draft_id, None, "the draft pointer is cleared with it");
        assert!(kept.accepted_at.is_some());
        assert_eq!(kept.server_reply, Some(reply));
        assert!(!service.outbox_mime(&item.id).unwrap().is_empty());
        assert_eq!(
            service.get_draft(&draft.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn queue_send_checks_revision_recipients_idempotency_and_undo_choices() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let draft = draft(&mut service, &account_id);

        let stale = QueueSendRequest {
            expected_revision: 9,
            ..request(&draft)
        };
        assert_eq!(
            service.queue_send(&stale, 0, t0()).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            service
                .queue_send(&request(&draft), 7, t0())
                .unwrap_err()
                .field
                .as_deref(),
            Some("undoSendSeconds")
        );

        let ask = request(&draft);
        let (first, locked) = service.queue_send(&ask, 10, t0()).unwrap();
        assert!(locked.is_some());
        assert_eq!(first.state, OutboxState::Queued);
        assert_eq!(first.not_before, "2026-10-23T12:00:10.000Z");
        assert_eq!(first.recipients, ["li@example.org"]);
        let (again, locked) = service.queue_send(&ask, 10, t0() + secs(5)).unwrap();
        assert_eq!(again.id, first.id);
        assert!(locked.is_none(), "a replay changes nothing");
        // The same mutation id for another draft or revision is a conflict.
        let other = {
            let mut other = self::draft(&mut service, &account_id);
            other.revision = 1;
            other
        };
        let reused = QueueSendRequest {
            client_mutation_id: ask.client_mutation_id.clone(),
            ..request(&other)
        };
        assert_eq!(
            service.queue_send(&reused, 0, t0()).unwrap_err().code,
            ErrorCode::IdempotencyConflict
        );
        let rows: i64 = service
            .storage
            .db
            .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_draft_without_recipients_is_not_queued() {
        let directory = TempDir::new().unwrap();
        let mut service = service(&directory);
        let account_id = account(&mut service, "lin@example.com");
        let empty = service
            .create_draft(
                serde_json::from_value(json!({
                    "accountId": account_id, "to": [], "cc": [], "bcc": [],
                    "subject": "", "bodyHtml": "", "clientMutationId": id(),
                }))
                .unwrap(),
                "",
            )
            .unwrap();
        let error = service.queue_send(&request(&empty), 0, t0()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some("draft.recipients"));
        assert!(!service.storage.draft_locked(&empty.id).unwrap());
    }
}
