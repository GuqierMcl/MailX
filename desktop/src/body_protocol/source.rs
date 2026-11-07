//! Both reader sources share token lifetime, CSP, resource URLs and link grants.
//! Their core bindings remain distinct; a draft can never fetch remote images.
use mailx_core::{DraftBodyBinding, mail::render::InlineImage};

use super::{AppError, BodyBinding, MailxCore, missing};

#[derive(Clone)]
pub(super) enum BodySource {
    Email(BodyBinding),
    Draft(DraftBodyBinding),
}

impl From<BodyBinding> for BodySource {
    fn from(binding: BodyBinding) -> Self {
        Self::Email(binding)
    }
}

impl BodySource {
    pub(super) fn email_id(&self) -> &str {
        match self {
            Self::Email(binding) => binding.email_id(),
            Self::Draft(binding) => binding.draft_id(),
        }
    }

    pub(super) fn account_id(&self) -> &str {
        match self {
            Self::Email(binding) => binding.account_id(),
            Self::Draft(binding) => binding.account_id(),
        }
    }

    pub(super) fn release_remote_images(&self) {
        if let Self::Email(binding) = self {
            binding.release_remote_images();
        }
    }

    pub(super) fn email(&self) -> Result<BodyBinding, AppError> {
        match self {
            Self::Email(binding) => Ok(binding.clone()),
            Self::Draft(_) => Err(missing()),
        }
    }

    pub(super) async fn document(&self, core: &MailxCore) -> Result<String, AppError> {
        match self {
            Self::Email(binding) => Ok(core.body_document_bound(binding.clone()).await?.html),
            Self::Draft(binding) => core.draft_body_document(binding.clone()).await,
        }
    }

    pub(super) async fn links(&self, core: &MailxCore) -> Result<Vec<String>, AppError> {
        match self {
            Self::Email(binding) => Ok(core
                .body_links(binding.email_id().to_owned(), Some(binding.clone()))
                .await?
                .links),
            Self::Draft(binding) => core.draft_body_links(binding.clone()).await,
        }
    }

    pub(super) async fn link(&self, core: &MailxCore, index: usize) -> Result<String, AppError> {
        match self {
            Self::Email(binding) => {
                core.body_link(binding.email_id().to_owned(), index, Some(binding.clone()))
                    .await
            }
            Self::Draft(_) => self
                .links(core)
                .await?
                .get(index)
                .cloned()
                .ok_or_else(missing),
        }
    }

    pub(super) async fn part(
        &self,
        core: &MailxCore,
        cid: String,
    ) -> Result<InlineImage, AppError> {
        match self {
            Self::Email(binding) => core.body_part(binding.clone(), cid).await,
            Self::Draft(binding) => core.draft_body_part(binding.clone(), cid).await,
        }
    }
}
