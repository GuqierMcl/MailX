//! Cached AI summaries of messages. The text is untrusted model output; the
//! interface renders it as Markdown without raw HTML.
use rusqlite::{OptionalExtension, params};

use super::Storage;
use crate::AppError;

/// What one finished summary writes.
pub(crate) struct NewSummary<'a> {
    pub message_id: &'a str,
    pub style: &'a str,
    pub language: &'a str,
    pub source_sha256: &'a str,
    pub prompt_version: u32,
    pub text: &'a str,
}

/// Which source a stored summary must belong to in order to count.
pub(crate) struct SummaryKey<'a> {
    pub message_id: &'a str,
    pub source_sha256: &'a str,
    pub prompt_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredSummary {
    pub style: String,
    pub language: String,
    pub text: String,
    pub created_at: String,
}

impl Storage {
    pub(crate) fn summary_put(&mut self, row: &NewSummary<'_>) -> Result<String, AppError> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.db.execute(
            "INSERT OR REPLACE INTO message_summaries(message_id, style, language, source_sha256,
               prompt_version, text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.message_id,
                row.style,
                row.language,
                row.source_sha256,
                row.prompt_version,
                row.text,
                now,
            ],
        )?;
        Ok(now)
    }

    /// The summary in `style` and `language`, or with both `None` the newest
    /// one of any style and language. `None` when absent or stale.
    pub(crate) fn summary_get(
        &self,
        key: &SummaryKey<'_>,
        style_language: Option<(&str, &str)>,
    ) -> Result<Option<StoredSummary>, AppError> {
        let map = |row: &rusqlite::Row<'_>| {
            Ok(StoredSummary {
                style: row.get(0)?,
                language: row.get(1)?,
                text: row.get(2)?,
                created_at: row.get(3)?,
            })
        };
        let found = match style_language {
            Some((style, language)) => self
                .db
                .query_row(
                    "SELECT style, language, text, created_at FROM message_summaries
                     WHERE message_id = ?1 AND source_sha256 = ?2 AND prompt_version = ?3
                       AND style = ?4 AND language = ?5",
                    params![
                        key.message_id,
                        key.source_sha256,
                        key.prompt_version,
                        style,
                        language
                    ],
                    map,
                )
                .optional()?,
            None => self
                .db
                .query_row(
                    "SELECT style, language, text, created_at FROM message_summaries
                     WHERE message_id = ?1 AND source_sha256 = ?2 AND prompt_version = ?3
                     ORDER BY created_at DESC LIMIT 1",
                    params![key.message_id, key.source_sha256, key.prompt_version],
                    map,
                )
                .optional()?,
        };
        Ok(found)
    }

    pub(crate) fn summary_delete_all(&mut self) -> Result<u64, AppError> {
        let removed = self.db.execute("DELETE FROM message_summaries", [])?;
        Ok(removed as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_with_message(id: &str) -> (TempDir, Storage) {
        let directory = TempDir::new().unwrap();
        let store = Storage::open(directory.path()).unwrap();
        store
            .db
            .execute_batch(&format!(
                "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
                    VALUES ('account', 1, 'Test', 'test@example.invalid', 'source', 't', 't');
                 INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
                    attributes, unread_count, total_count, synced_at)
                    VALUES ('folder', 'account', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 1, 1, 't');
                 INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
                    subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
                    received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
                    VALUES ('{id}', 'account', 'source', 'folder', 10, 1, 'Hello',
                    'a@example.invalid', '[]', '[]', '[]', 't', 0, 0, '[]', 0, 'present', 't');"
            ))
            .unwrap();
        (directory, store)
    }

    #[test]
    fn summaries_are_found_by_source_and_replaced_per_style_and_language() {
        let (_directory, mut storage) = open_with_message("m");
        let key = SummaryKey {
            message_id: "m",
            source_sha256: "h1",
            prompt_version: 1,
        };
        assert_eq!(storage.summary_get(&key, None).unwrap(), None);
        let put = |storage: &mut Storage, style: &str, text: &str, hash: &str| {
            storage
                .summary_put(&NewSummary {
                    message_id: "m",
                    style,
                    language: "en",
                    source_sha256: hash,
                    prompt_version: 1,
                    text,
                })
                .unwrap()
        };
        put(&mut storage, "brief", "One.", "h1");
        put(&mut storage, "points", "- Two.", "h1");
        put(&mut storage, "brief", "Again.", "h1");
        let brief = storage
            .summary_get(&key, Some(("brief", "en")))
            .unwrap()
            .unwrap();
        assert_eq!(brief.text, "Again.");
        assert!(
            storage
                .summary_get(&key, Some(("detailed", "en")))
                .unwrap()
                .is_none()
        );
        // The newest of any style.
        assert_eq!(
            storage.summary_get(&key, None).unwrap().unwrap().text,
            "Again."
        );
        // Another body or prompt makes it stale.
        let stale = SummaryKey {
            source_sha256: "h2",
            ..key
        };
        assert!(storage.summary_get(&stale, None).unwrap().is_none());
        let newer_prompt = SummaryKey {
            prompt_version: 2,
            ..key
        };
        assert!(storage.summary_get(&newer_prompt, None).unwrap().is_none());
        assert_eq!(storage.summary_delete_all().unwrap(), 2);
        assert!(storage.summary_get(&key, None).unwrap().is_none());
    }
}
