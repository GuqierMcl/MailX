//! Translation cache operations. Nothing here sees a service key or mail text
//! that has not already been stored.
use super::AccountService;
use crate::{
    AppError,
    storage::{
        bodies::{BodyIdentity, StoredBlob},
        translations::{NewTranslation, StoredTranslation, TranslationKey, TranslationStats},
    },
};

impl AccountService {
    pub(crate) fn translation_subject(&self, email_id: &str) -> Result<Option<String>, AppError> {
        self.storage.translation_subject(email_id)
    }

    pub(crate) fn translation_get(
        &self,
        key: &TranslationKey<'_>,
    ) -> Result<Option<StoredTranslation>, AppError> {
        self.storage.translation_get(key)
    }

    /// Writes only while the message still has the body the translation was
    /// made from; `false` means it changed in the meantime and nothing was written.
    pub(crate) fn translation_put(
        &mut self,
        identity: &BodyIdentity,
        expected: &(StoredBlob, u32),
        row: &NewTranslation<'_>,
    ) -> Result<bool, AppError> {
        if self.storage.body_stamp(identity)?.as_ref() != Some(expected) {
            return Ok(false);
        }
        self.storage.translation_put(row)?;
        Ok(true)
    }

    pub(crate) fn translation_delete_message(&mut self, email_id: &str) -> Result<(), AppError> {
        super::validate_email_id(email_id)?;
        self.storage.translation_delete_message(email_id)
    }

    pub(crate) fn translation_delete_all(&mut self) -> Result<(), AppError> {
        self.storage.translation_delete_all()
    }

    pub(crate) fn translation_stats(&self) -> Result<TranslationStats, AppError> {
        self.storage.translation_stats()
    }
}
