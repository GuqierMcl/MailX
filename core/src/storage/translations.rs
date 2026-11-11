//! Cached message translations. The text is untrusted service output stored as
//! wire strings; it only becomes HTML through `translate::apply`.
use rusqlite::{OptionalExtension, params};

use super::{Storage, decode, encode};
use crate::{AppError, ErrorCode, settings::TranslationProviderId};

/// What one finished translation writes.
pub(crate) struct NewTranslation<'a> {
    pub message_id: &'a str,
    pub target_language: &'a str,
    pub include_quotes: bool,
    pub raw_sha256: &'a str,
    pub sanitizer_version: u32,
    pub segmenter_version: u32,
    pub provider: TranslationProviderId,
    pub source_language: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub segments: &'a [Option<String>],
    pub partial: bool,
}

/// Which body a stored translation must belong to in order to count.
pub(crate) struct TranslationKey<'a> {
    pub message_id: &'a str,
    pub target_language: &'a str,
    pub include_quotes: bool,
    pub raw_sha256: &'a str,
    pub sanitizer_version: u32,
    pub segmenter_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredTranslation {
    pub provider: TranslationProviderId,
    pub source_language: Option<String>,
    pub subject: Option<String>,
    pub segments: Vec<Option<String>>,
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranslationStats {
    pub count: u64,
    pub bytes: u64,
}

fn provider_from(name: &str) -> Option<TranslationProviderId> {
    TranslationProviderId::ALL
        .into_iter()
        .find(|provider| provider.as_str() == name)
}

impl Storage {
    pub(crate) fn translation_put(&mut self, row: &NewTranslation<'_>) -> Result<(), AppError> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.db.execute(
            "INSERT OR REPLACE INTO message_translations(message_id, target_language, include_quotes,
               raw_sha256, sanitizer_version, segmenter_version, provider, source_language, subject,
               segments, partial, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.message_id,
                row.target_language,
                row.include_quotes,
                row.raw_sha256,
                row.sanitizer_version,
                row.segmenter_version,
                row.provider.as_str(),
                row.source_language,
                row.subject,
                encode(&row.segments)?,
                row.partial,
                now,
            ],
        )?;
        Ok(())
    }

    /// `None` when absent or stale (another body, sanitizer or segmenter version).
    pub(crate) fn translation_get(
        &self,
        key: &TranslationKey<'_>,
    ) -> Result<Option<StoredTranslation>, AppError> {
        let row = self
            .db
            .query_row(
                "SELECT provider, source_language, subject, segments, partial
                 FROM message_translations
                 WHERE message_id = ?1 AND target_language = ?2 AND include_quotes = ?3
                   AND raw_sha256 = ?4 AND sanitizer_version = ?5 AND segmenter_version = ?6",
                params![
                    key.message_id,
                    key.target_language,
                    key.include_quotes,
                    key.raw_sha256,
                    key.sanitizer_version,
                    key.segmenter_version,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((provider, source_language, subject, segments, partial)) = row else {
            return Ok(None);
        };
        let Some(provider) = provider_from(&provider) else {
            return Ok(None);
        };
        Ok(Some(StoredTranslation {
            provider,
            source_language,
            subject,
            segments: decode(&segments)?,
            partial,
        }))
    }

    pub(crate) fn translation_delete_message(&mut self, message_id: &str) -> Result<(), AppError> {
        self.db.execute(
            "DELETE FROM message_translations WHERE message_id = ?1",
            [message_id],
        )?;
        Ok(())
    }

    pub(crate) fn translation_delete_all(&mut self) -> Result<(), AppError> {
        self.db.execute("DELETE FROM message_translations", [])?;
        Ok(())
    }

    pub(crate) fn translation_stats(&self) -> Result<TranslationStats, AppError> {
        let (count, bytes): (i64, i64) = self.db.query_row(
            "SELECT count(*), COALESCE(SUM(length(CAST(segments AS BLOB))
                  + COALESCE(length(CAST(subject AS BLOB)), 0)), 0)
             FROM message_translations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let unsigned = |value: i64| {
            u64::try_from(value).map_err(|_| AppError::new(ErrorCode::StorageIncompatible))
        };
        Ok(TranslationStats {
            count: unsigned(count)?,
            bytes: unsigned(bytes)?,
        })
    }

    pub(crate) fn translation_subject(&self, message_id: &str) -> Result<Option<String>, AppError> {
        Ok(self
            .db
            .query_row(
                "SELECT subject FROM messages WHERE id = ?1",
                [message_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod tests;
