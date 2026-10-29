//! Partial-fetch list previews, kept apart from the guarded full-body path.
//!
//! A preview only ever fills a row whose body has not been downloaded: once
//! `body_state` is `ready`, the preview written by `body_commit` is final.
use super::{
    Storage,
    bodies::{BodyIdentity, current, read_identity},
};
use crate::{AppError, mail::PreviewState};

/// One finished partial fetch. `preview` is `None` when the message parsed but
/// had nothing to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewOutcome {
    pub(crate) identity: BodyIdentity,
    pub(crate) preview: Option<String>,
}

/// What changed for the UI after a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewWritten {
    pub(crate) message_id: String,
    pub(crate) preview: String,
    pub(crate) state: PreviewState,
}

impl Storage {
    /// The subset of `message_ids` that still wants a partial fetch, with the
    /// identity needed to fetch it. Unknown, stale, vanished or already
    /// previewed rows are skipped.
    pub(crate) fn preview_targets(
        &self,
        message_ids: &[String],
    ) -> Result<Vec<BodyIdentity>, AppError> {
        let mut targets = Vec::new();
        for id in message_ids {
            let wanted: bool = self.db.query_row(
                "SELECT EXISTS (SELECT 1 FROM messages
                 WHERE id = ?1 AND preview_state = 'missing' AND body_state != 'ready')",
                [id],
                |row| row.get(0),
            )?;
            if wanted && let Some(identity) = read_identity(&self.db, id)? {
                targets.push(identity);
            }
        }
        Ok(targets)
    }

    /// Persist finished fetches in one transaction. Rows whose identity moved
    /// on (UIDVALIDITY reset, account edit, removal) or whose body arrived in
    /// the meantime are left alone and left out of the result.
    pub(crate) fn preview_commit(
        &mut self,
        outcomes: &[PreviewOutcome],
    ) -> Result<Vec<PreviewWritten>, AppError> {
        let tx = self.db.transaction()?;
        let mut written = Vec::new();
        for outcome in outcomes {
            if !current(&tx, &outcome.identity)? {
                continue;
            }
            let (preview, state, value) = match &outcome.preview {
                Some(text) => (text.as_str(), PreviewState::Ready, "ready"),
                None => ("", PreviewState::Failed, "failed"),
            };
            let changed = tx.execute(
                "UPDATE messages SET preview = ?2, preview_state = ?3
                 WHERE id = ?1 AND body_state != 'ready'",
                rusqlite::params![outcome.identity.message_id, preview, value],
            )?;
            if changed > 0 {
                written.push(PreviewWritten {
                    message_id: outcome.identity.message_id.clone(),
                    preview: preview.to_owned(),
                    state,
                });
            }
        }
        tx.commit()?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests;
