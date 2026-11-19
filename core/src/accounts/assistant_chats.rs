//! Saved assistant conversations. A thin pass-through to storage, so the
//! assistant uses the same service lock as every other writer.
use super::AccountService;
use crate::{
    AppError,
    storage::assistant_chats::{Append, StoredConversation, StoredSummary},
};

impl AccountService {
    pub(crate) fn assistant_chat_append(&mut self, write: &Append<'_>) -> Result<bool, AppError> {
        self.storage.assistant_chat_append(write)
    }

    pub(crate) fn assistant_chat_list(
        &self,
        limit: usize,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<StoredSummary>, AppError> {
        self.storage.assistant_chat_list(limit, after)
    }

    pub(crate) fn assistant_chat_exists(&self, id: &str) -> Result<bool, AppError> {
        self.storage.assistant_chat_exists(id)
    }

    pub(crate) fn assistant_chat_get(
        &self,
        id: &str,
    ) -> Result<Option<StoredConversation>, AppError> {
        self.storage.assistant_chat_get(id)
    }

    pub(crate) fn assistant_chat_delete(&mut self, id: &str) -> Result<bool, AppError> {
        self.storage.assistant_chat_delete(id)
    }

    pub(crate) fn assistant_chat_clear(&mut self) -> Result<(), AppError> {
        self.storage.assistant_chat_clear()
    }
}

impl AccountService {
    pub(crate) fn summary_put(
        &mut self,
        row: &crate::storage::summaries::NewSummary<'_>,
    ) -> Result<String, AppError> {
        self.storage.summary_put(row)
    }

    pub(crate) fn summary_get(
        &self,
        key: &crate::storage::summaries::SummaryKey<'_>,
        style_language: Option<(&str, &str)>,
    ) -> Result<Option<crate::storage::summaries::StoredSummary>, AppError> {
        self.storage.summary_get(key, style_language)
    }

    pub(crate) fn summary_delete_all(&mut self) -> Result<u64, AppError> {
        self.storage.summary_delete_all()
    }
}
