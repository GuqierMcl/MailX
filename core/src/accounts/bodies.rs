//! Internal body persistence operations. No HTML or blob reference is an IPC DTO.
use crate::{
    AppError,
    mail::{BodyFailureCode, render::Rendered},
    storage::{
        bodies::{BlobStore, BodyIdentity, CachedBody, StoredBlob},
        previews::{PreviewOutcome, PreviewWritten},
    },
};

use super::{AccountService, validate_email_id};

impl AccountService {
    pub(crate) fn body_identity(&self, id: &str) -> Result<BodyIdentity, AppError> {
        validate_email_id(id)?;
        self.storage.body_identity(id)
    }

    pub(crate) fn body_current(&self, identity: &BodyIdentity) -> Result<bool, AppError> {
        self.storage.body_identity_current(identity)
    }

    pub(crate) fn preview_targets(&self, ids: &[String]) -> Result<Vec<BodyIdentity>, AppError> {
        self.storage.preview_targets(ids)
    }

    pub(crate) fn preview_commit(
        &mut self,
        outcomes: &[PreviewOutcome],
    ) -> Result<Vec<PreviewWritten>, AppError> {
        self.storage.preview_commit(outcomes)
    }

    pub(crate) fn body_blobs(&self) -> BlobStore {
        self.storage.blobs.clone()
    }

    pub(crate) fn body_begin(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        self.storage.body_begin(identity)
    }

    pub(crate) fn body_rebuild(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        self.storage.body_rebuild(identity)
    }

    pub(crate) fn body_stamp(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<(StoredBlob, u32)>, AppError> {
        self.storage.body_stamp(identity)
    }

    pub(crate) fn body_part(
        &self,
        identity: &BodyIdentity,
        cid: &str,
    ) -> Result<Option<(StoredBlob, crate::mail::render::RenderedPart)>, AppError> {
        self.storage.body_part(identity, cid)
    }

    pub(crate) fn body_attachment(
        &self,
        identity: &BodyIdentity,
        part_index: usize,
    ) -> Result<Option<(StoredBlob, crate::mail::render::RenderedPart)>, AppError> {
        self.storage.body_attachment(identity, part_index)
    }

    pub(crate) fn body_link(
        &self,
        identity: &BodyIdentity,
        index: usize,
    ) -> Result<Option<String>, AppError> {
        self.storage.body_link(identity, index)
    }

    pub(crate) fn body_links(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<Vec<String>>, AppError> {
        self.storage.body_links(identity)
    }

    pub(crate) fn body_fail(
        &mut self,
        identity: &BodyIdentity,
        failure: BodyFailureCode,
    ) -> Result<bool, AppError> {
        self.storage.body_fail(identity, failure)
    }

    pub(crate) fn body_reset(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        self.storage.body_reset(identity)
    }

    pub(crate) fn body_cache(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<CachedBody>, AppError> {
        self.storage.body_cache(identity)
    }

    pub(crate) fn body_commit(
        &mut self,
        identity: &BodyIdentity,
        blob: &StoredBlob,
        rendered: &Rendered,
    ) -> Result<bool, AppError> {
        self.storage.body_commit(identity, blob, rendered)
    }
}
