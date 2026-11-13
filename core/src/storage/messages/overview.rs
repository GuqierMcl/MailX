//! Bounded reads behind the Now page overview (now-page.md section 4).
//!
//! Every query goes through [`VISIBLE_MESSAGES`], so it sees exactly the rows
//! the mail list shows: the account's current mailbox source, the folder's
//! current UIDVALIDITY generation, present on the server, not deleted locally.
//! Inbox queries also leave out mail archived on this device, as the inbox
//! list does; junk and trash are other folder roles and never match.
use rusqlite::params;

use super::{SUMMARY_COLUMNS, VISIBLE_MESSAGES, archived_sql, summary};
use crate::{
    AppError,
    mail::{EmailSummary, MailAddress},
    storage::Storage,
};

/// The inbox rows of the overview, newest first.
const INBOX: &str = concat!("f.role = 'inbox' AND NOT ", archived_sql!());

/// Per-account counts of new inbox mail and the newest of those messages.
pub(crate) type NewMailRows = (Vec<(String, u32)>, Vec<EmailSummary>);

/// Most attachment names returned for one message.
const MAX_FILE_NAMES: u32 = 20;

/// A recent inbox message to run the verification-code detector on. Its
/// `text` is never logged or returned as is.
pub(crate) struct CodeSource {
    pub email_id: String,
    pub account_id: String,
    pub folder_id: String,
    pub from: MailAddress,
    pub subject: String,
    pub received_at: String,
    /// The cached plain-text body when the body is downloaded, otherwise the
    /// stored preview. `None` when the cached body is larger than the
    /// detector accepts, so the message is skipped rather than truncated.
    pub text: Option<String>,
}

impl Storage {
    /// Inbox messages received strictly after `since`, per account (accounts
    /// without any are left out), and the newest `latest` of them.
    pub(crate) fn overview_new_mail(
        &self,
        since: &str,
        latest: u32,
    ) -> Result<NewMailRows, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT m.account_id, COUNT(*) {VISIBLE_MESSAGES} AND {INBOX} AND m.received_at > ?1
             GROUP BY m.account_id ORDER BY m.account_id"
        ))?;
        let counts = statement
            .query_map([since], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut statement = self.db.prepare(&format!(
            "SELECT {SUMMARY_COLUMNS} {VISIBLE_MESSAGES} AND {INBOX} AND m.received_at > ?1
             ORDER BY m.received_at DESC, m.id DESC LIMIT ?2"
        ))?;
        let items = statement
            .query_map(params![since, latest], summary)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((counts, items))
    }

    /// Starred unread messages in the Starred view's scope: every folder but
    /// junk and trash, locally archived mail included. The literal flag values
    /// let SQLite use the partial `messages_starred_unread` index.
    pub(crate) fn overview_starred_unread(&self) -> Result<u32, AppError> {
        Ok(self.db.query_row(
            &format!(
                "SELECT COUNT(*) {VISIBLE_MESSAGES} AND m.is_flagged = 1 AND m.is_read = 0
                 AND f.role <> 'junk' AND f.role <> 'trash'"
            ),
            [],
            |row| row.get(0),
        )?)
    }

    /// At most `limit` inbox messages received at or after `after`, newest
    /// first, with the text to search for a verification code. Never fetches
    /// a body: it reads only what is already stored.
    pub(crate) fn overview_code_sources(
        &self,
        after: &str,
        limit: u32,
        max_text_bytes: usize,
    ) -> Result<Vec<CodeSource>, AppError> {
        let max_text_bytes = i64::try_from(max_text_bytes).unwrap_or(i64::MAX);
        let mut statement = self.db.prepare(&format!(
            "SELECT m.id, m.account_id, m.folder_id, m.subject, m.from_name, m.from_address,
                    m.received_at, m.body_state = 'ready', m.preview,
                    (SELECT length(CAST(b.plain_text AS BLOB)) FROM message_bodies b
                      WHERE b.message_id = m.id),
                    (SELECT b.plain_text FROM message_bodies b
                      WHERE b.message_id = m.id AND length(CAST(b.plain_text AS BLOB)) <= ?3)
             {VISIBLE_MESSAGES} AND {INBOX} AND m.received_at >= ?1
             ORDER BY m.received_at DESC, m.id DESC LIMIT ?2"
        ))?;
        let rows = statement
            .query_map(params![after, limit, max_text_bytes], |row| {
                let ready: bool = row.get(7)?;
                let preview: String = row.get(8)?;
                let body_size: Option<i64> = row.get(9)?;
                let body: Option<String> = row.get(10)?;
                let text = match (ready, body_size) {
                    // A downloaded body with plain text: use it, or skip the
                    // message when it exceeds the detector's budget.
                    (true, Some(_)) => body,
                    _ => Some(preview),
                };
                Ok(CodeSource {
                    email_id: row.get(0)?,
                    account_id: row.get(1)?,
                    folder_id: row.get(2)?,
                    subject: row.get(3)?,
                    from: MailAddress {
                        name: row.get(4)?,
                        address: row.get(5)?,
                    },
                    received_at: row.get(6)?,
                    text,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// At most `limit` inbox messages with attachments received at or after
    /// `after`, newest first, each with the names of its non-inline parts
    /// when its body has been downloaded (empty otherwise).
    pub(crate) fn overview_attachments(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<Vec<(EmailSummary, Vec<String>)>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT {SUMMARY_COLUMNS} {VISIBLE_MESSAGES} AND {INBOX} AND m.has_attachment = 1
             AND m.received_at >= ?1 ORDER BY m.received_at DESC, m.id DESC LIMIT ?2"
        ))?;
        let items = statement
            .query_map(params![after, limit], summary)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut names = self.db.prepare_cached(
            "SELECT filename FROM message_parts
             WHERE message_id = ?1 AND is_inline = 0 AND filename IS NOT NULL AND filename <> ''
             ORDER BY part_index LIMIT ?2",
        )?;
        items
            .into_iter()
            .map(|item| {
                let files = names
                    .query_map(params![item.id, MAX_FILE_NAMES], |row| row.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok((item, files))
            })
            .collect()
    }
}
