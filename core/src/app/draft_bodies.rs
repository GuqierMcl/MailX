//! Read-only draft bodies for the isolated reader. No MIME snapshot or network
//! access is needed, and every resource remains bound to one saved revision.
use std::{collections::HashMap, sync::Arc};

use scraper::{Html, Node, node::Text};

use super::MailxCore;
use crate::{
    AccountService, AppError, Draft, ErrorCode,
    drafts::body,
    mail::render::{self, InlineImage},
};

/// Opaque host-only binding; HTML is never a serializable application DTO.
#[derive(Clone)]
pub struct DraftBodyBinding {
    draft_id: String,
    account_id: String,
    account_revision: u32,
    revision: u32,
    html: Arc<String>,
    links: Arc<Vec<String>>,
    images: Arc<HashMap<String, String>>,
}

impl DraftBodyBinding {
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

fn missing() -> AppError {
    AppError::new(ErrorCode::NotFound)
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

fn check(service: &AccountService, binding: &DraftBodyBinding) -> Result<Draft, AppError> {
    let draft = service.get_draft(&binding.draft_id)?;
    if draft.account_id != binding.account_id
        || draft.revision != binding.revision
        || service.revision(&binding.account_id)? != binding.account_revision
    {
        return Err(missing());
    }
    Ok(draft)
}

/// Missing attachment rows keep a readable, escaped label in the document.
/// Existing rows still load independently, so one damaged blob cannot hide text.
fn missing_image_placeholders(html: &str, images: &HashMap<String, String>) -> String {
    let mut document = Html::parse_fragment(html);
    let missing: Vec<_> = document
        .tree
        .nodes()
        .filter_map(|node| {
            let Node::Element(element) = node.value() else {
                return None;
            };
            let cid = element.attr("data-mailx-cid")?;
            if element.name() != "img" || images.contains_key(cid) {
                return None;
            }
            let alt = element
                .attr("alt")
                .filter(|alt| !alt.trim().is_empty())
                .unwrap_or("image");
            Some((node.id(), format!("[{alt}]")))
        })
        .collect();
    for (id, text) in missing {
        *document
            .tree
            .get_mut(id)
            .expect("existing image node")
            .value() = Node::Text(Text { text: text.into() });
    }
    document.root_element().inner_html()
}

impl MailxCore {
    /// Prepare only the revision the main window displayed. Editing never
    /// silently rebinds an old token to different content or link targets.
    pub async fn prepare_draft_body(
        &self,
        draft_id: String,
        expected_revision: u32,
    ) -> Result<DraftBodyBinding, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let (draft, account_revision) = {
                let service = core.accounts()?;
                let draft = service.get_draft(&draft_id)?;
                if draft.revision != expected_revision {
                    return Err(AppError::new(ErrorCode::Conflict));
                }
                let account_revision = service.revision(&draft.account_id)?;
                (draft, account_revision)
            };
            // Reuse the sending projection for editor-only color/alignment
            // attributes, then the mail renderer for passive resource slots.
            let referenced = body::referenced_cids(&draft.body_html);
            let rendered = render::sanitize_document(&body::email_document(&draft.body_html));
            let images = draft
                .attachments
                .iter()
                .filter(|file| file.inline)
                .filter_map(|file| {
                    let cid = file.content_id.as_ref()?;
                    referenced
                        .contains(cid)
                        .then(|| (cid.clone(), file.id.clone()))
                })
                .collect();
            let binding = DraftBodyBinding {
                draft_id,
                account_id: draft.account_id,
                account_revision,
                revision: expected_revision,
                html: Arc::new(missing_image_placeholders(&rendered.html, &images)),
                links: Arc::new(rendered.links),
                images: Arc::new(images),
            };
            check(&*core.accounts()?, &binding)?;
            Ok(binding)
        })
        .await
        .map_err(|_| internal())?
    }

    pub async fn draft_body_document(&self, binding: DraftBodyBinding) -> Result<String, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            check(&*core.accounts()?, &binding)?;
            Ok((*binding.html).clone())
        })
        .await
        .map_err(|_| internal())?
    }

    pub async fn draft_body_links(
        &self,
        binding: DraftBodyBinding,
    ) -> Result<Vec<String>, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            check(&*core.accounts()?, &binding)?;
            Ok((*binding.links).clone())
        })
        .await
        .map_err(|_| internal())?
    }

    pub async fn draft_body_part(
        &self,
        binding: DraftBodyBinding,
        content_id: String,
    ) -> Result<InlineImage, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            check(&service, &binding)?;
            let attachment_id = binding.images.get(&content_id).ok_or_else(missing)?;
            // The service validates and re-encodes raster bytes. The same lock
            // keeps saves/deletes from changing the attachment while it is read.
            let (mime_type, bytes) =
                service.inline_image_preview(&binding.draft_id, attachment_id)?;
            Ok(InlineImage { mime_type, bytes })
        })
        .await
        .map_err(|_| internal())?
    }
}
