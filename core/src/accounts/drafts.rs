//! Draft operations on the account store: validation, idempotency, revisions.
use chrono::{FixedOffset, Local, Offset};
use uuid::Uuid;

use super::{AccountService, new_id, now, validate_uuid};
use crate::{
    AppError, AttachmentOrigin, ErrorCode,
    drafts::{
        attachments::ForwardedFile,
        compose::{self, Composed, SourceHeaders},
        types::*,
        validate::{self, DraftContent},
    },
    mail::{BodyState, EmailDetail},
    storage::{
        attachments::NewAttachment,
        bodies::{BlobStore, StoredBlob},
        drafts::NewDraft,
    },
};

/// Everything `create_draft_from_source` builds the draft from, read while the
/// store lock was released.
pub(crate) struct SourceMessage<'a> {
    pub detail: &'a EmailDetail,
    pub account_address: &'a str,
    pub headers: &'a SourceHeaders,
    /// What a forward carries over from the message, already in the blob store;
    /// empty for replies and for messages without files.
    pub forwarded: &'a [ForwardedFile],
}

/// What `create_draft_from` needs from the store before it touches any file.
pub(crate) enum DraftSource {
    /// The mutation id already created a draft for this very request.
    Existing(Draft),
    Fresh {
        detail: EmailDetail,
        account_address: String,
        message_id: Option<String>,
        raw: Option<(BlobStore, StoredBlob)>,
    },
}

fn canonical(value: serde_json::Value) -> Result<String, AppError> {
    serde_json::to_string(&value).map_err(|_| AppError::new(ErrorCode::Internal))
}

fn content_json(content: &DraftContent) -> serde_json::Value {
    serde_json::json!({
        "to": content.to, "cc": content.cc, "bcc": content.bcc,
        "subject": content.subject, "bodyHtml": content.body_html,
    })
}

impl AccountService {
    fn check_account(&self, account_id: &str) -> Result<(), AppError> {
        validate_uuid(account_id, "accountId")?;
        if self.storage.account_exists(account_id)? {
            Ok(())
        } else {
            Err(AppError::new(ErrorCode::NotFound))
        }
    }

    /// An explicit account, else `default_account_id` if it names an account,
    /// else the first account in `list` order.
    fn resolve_account(
        &self,
        requested: Option<&str>,
        default_account_id: &str,
    ) -> Result<String, AppError> {
        if let Some(id) = requested {
            self.check_account(id)?;
            return Ok(id.to_owned());
        }
        let default_account_id = default_account_id.trim();
        if Uuid::parse_str(default_account_id).is_ok()
            && self.storage.account_exists(default_account_id)?
        {
            return Ok(default_account_id.to_owned());
        }
        self.storage
            .account_ids()?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))
    }

    /// Replays `client_mutation_id` if it was used before: the same request
    /// returns the draft as it was created (and edited since); any other
    /// content is an idempotency conflict.
    fn replay(&self, client_mutation_id: &str, canonical: &str) -> Result<Option<Draft>, AppError> {
        match self.storage.draft_by_mutation(client_mutation_id)? {
            None => Ok(None),
            Some((draft, saved)) if saved == canonical => Ok(Some(draft)),
            Some(_) => Err(AppError::new(ErrorCode::IdempotencyConflict)),
        }
    }

    /// `default_account_id` is the `compose.defaultAccountId` setting; the
    /// settings store is not part of this service, so the host passes it in.
    pub fn create_draft(
        &mut self,
        request: CreateDraftRequest,
        default_account_id: &str,
    ) -> Result<Draft, AppError> {
        validate_uuid(&request.client_mutation_id, "clientMutationId")?;
        let content = validate::normalize(DraftContent {
            to: request.to,
            cc: request.cc,
            bcc: request.bcc,
            subject: request.subject,
            body_html: request.body_html,
            body_text: String::new(),
        })?;
        let canonical = canonical(serde_json::json!({
            "op": "create", "accountId": request.account_id, "content": content_json(&content),
        }))?;
        if let Some(draft) = self.replay(&request.client_mutation_id, &canonical)? {
            return Ok(draft);
        }
        let account_id = self.resolve_account(request.account_id.as_deref(), default_account_id)?;
        let id = new_id();
        let now = now();
        self.storage.insert_draft(&NewDraft {
            id: &id,
            account_id: &account_id,
            kind: DraftKind::New,
            source_email_id: None,
            content: &content,
            in_reply_to: None,
            references: None,
            now: &now,
            client_mutation_id: &request.client_mutation_id,
            canonical: &canonical,
        })
    }

    fn from_canonical(request: &CreateDraftFromRequest) -> Result<String, AppError> {
        canonical(serde_json::json!({
            "op": "createFrom", "emailId": request.email_id, "kind": request.kind.as_str(),
        }))
    }

    /// First half of `create_draft_from`: validate, replay, and gather what
    /// must be read from disk. Does no file I/O itself.
    pub(crate) fn draft_source(
        &self,
        request: &CreateDraftFromRequest,
    ) -> Result<DraftSource, AppError> {
        validate_uuid(&request.client_mutation_id, "clientMutationId")?;
        validate_uuid(&request.email_id, "emailId")?;
        if request.kind == DraftKind::New {
            return Err(AppError::validation("kind"));
        }
        if let Some(draft) =
            self.replay(&request.client_mutation_id, &Self::from_canonical(request)?)?
        {
            return Ok(DraftSource::Existing(draft));
        }
        // Visible messages only: a deleted, stale or unselectable one is NotFound.
        let detail = self.storage.get_email(&request.email_id)?;
        if detail.summary.body_state != BodyState::Ready || detail.body.is_none() {
            return Err(AppError::new(ErrorCode::BodyNotCached));
        }
        let account_address = self.storage.account_address(&detail.summary.account_id)?;
        let identity = self.storage.body_identity(&request.email_id)?;
        let raw = self
            .storage
            .body_stamp(&identity)?
            .map(|(blob, _)| (self.storage.blobs.clone(), blob));
        Ok(DraftSource::Fresh {
            message_id: self.storage.message_id_header(&request.email_id)?,
            account_address,
            detail,
            raw,
        })
    }

    /// Second half: build the content (no I/O) and store it, with its
    /// attachments in the same transaction.
    pub(crate) fn create_draft_from_source(
        &mut self,
        request: &CreateDraftFromRequest,
        source: &SourceMessage<'_>,
        offset: FixedOffset,
        locale: crate::SupportedLocale,
    ) -> Result<Draft, AppError> {
        let SourceMessage {
            detail,
            account_address,
            headers,
            forwarded,
        } = source;
        let text = detail
            .body
            .as_ref()
            .and_then(|body| body.text.as_deref())
            .unwrap_or_default();
        let identity = self.storage.body_identity(&request.email_id)?;
        let cached = self.storage.body_cache(&identity)?;
        let original = crate::drafts::body::quote_cached_source(
            cached
                .as_ref()
                .filter(|_| headers.has_html)
                .and_then(|body| body.rendered.sanitized_html.as_deref()),
            cached
                .as_ref()
                .map(|body| body.rendered.links.as_slice())
                .unwrap_or_default(),
            text,
            locale,
        );
        let Composed {
            content,
            in_reply_to,
            references,
        } = compose::compose(
            detail,
            &original,
            request.kind,
            account_address,
            headers,
            offset,
            locale,
        );
        let content = validate::normalize(content)?;
        let canonical = Self::from_canonical(request)?;
        // The lock was released while the raw MIME was read: an identical
        // request may have created the draft meanwhile.
        if let Some(draft) = self.replay(&request.client_mutation_id, &canonical)? {
            return Ok(draft);
        }
        // The message may have been removed while the raw MIME was read.
        self.storage.get_email(&request.email_id)?;
        let id = new_id();
        let now = now();
        let ids: Vec<String> = forwarded.iter().map(|_| new_id()).collect();
        let attachments: Vec<NewAttachment<'_>> = forwarded
            .iter()
            .zip(&ids)
            .map(|(file, id)| NewAttachment {
                id,
                content_id: None,
                file_name: &file.file_name,
                media_type: &file.media_type,
                blob: &file.blob,
                origin: AttachmentOrigin::Forwarded,
            })
            .collect();
        self.storage.insert_draft_with(
            &NewDraft {
                id: &id,
                account_id: &detail.summary.account_id,
                kind: request.kind,
                source_email_id: Some(&request.email_id),
                content: &content,
                in_reply_to: in_reply_to.as_deref(),
                references: references.as_deref(),
                now: &now,
                client_mutation_id: &request.client_mutation_id,
                canonical: &canonical,
            },
            &attachments,
        )
    }

    pub fn get_draft(&self, id: &str) -> Result<Draft, AppError> {
        validate_uuid(id, "draftId")?;
        self.storage.get_draft(id)
    }

    pub fn list_drafts(&self, account_id: Option<&str>) -> Result<Vec<DraftSummary>, AppError> {
        if let Some(account_id) = account_id {
            validate_uuid(account_id, "accountId")?;
        }
        self.storage.list_drafts(account_id)
    }

    pub fn update_draft(&mut self, request: UpdateDraftRequest) -> Result<Draft, AppError> {
        validate_uuid(&request.id, "draftId")?;
        let current = self.storage.get_draft(&request.id)?;
        let unchanged_body = request.body_html == current.body_html;
        let content = DraftContent {
            to: request.to,
            cc: request.cc,
            bcc: request.bcc,
            subject: request.subject,
            body_html: request.body_html,
            body_text: current.body_text,
        };
        let content = if unchanged_body {
            validate::normalize_headers(content)?
        } else {
            validate::normalize(content)?
        };
        if let Some(account_id) = &request.account_id {
            self.check_account(account_id)?;
        }
        // A draft held by the send queue is the snapshot's source; editing it
        // now would suggest the queued mail changes.
        if self.storage.draft_locked(&request.id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        self.storage.update_draft(
            &request.id,
            request.expected_revision,
            request.account_id.as_deref(),
            &content,
            &now(),
        )
    }

    /// Deletes blank drafts that no live queue item holds; run at startup.
    pub fn purge_blank_drafts(&mut self) -> Result<(), AppError> {
        self.storage.purge_blank_drafts()
    }

    pub fn delete_draft(&mut self, id: &str) -> Result<(), AppError> {
        validate_uuid(id, "draftId")?;
        if self.storage.draft_locked(id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        self.storage.delete_draft(id)
    }
}

/// The offset of the machine's local time zone right now.
pub(crate) fn local_offset() -> FixedOffset {
    Local::now().offset().fix()
}
