//! Bounded reads behind the Now page overview (now-page.md section 4).
//!
//! Every query goes through [`VISIBLE_MESSAGES`], so it sees exactly the rows
//! the mail list shows: the account's current mailbox source, the folder's
//! current UIDVALIDITY generation, present on the server, not deleted locally.
//! Inbox queries also leave out mail archived on this device, as the inbox
//! list does; junk and trash are other folder roles and never match.
use rusqlite::{OptionalExtension, params};

use super::{LOCAL_COPIES, SUMMARY_COLUMNS, VISIBLE_MESSAGES, archived_sql, body_state, summary};
use crate::{
    AppError,
    mail::{BodyState, EmailSummary, MailAddress},
    storage::Storage,
};

/// The inbox rows of the overview, newest first.
const INBOX: &str = concat!("f.role = 'inbox' AND NOT ", archived_sql!());

/// Every inbox row, locally archived mail included: activity counts mail as
/// received whatever happened to it on this device afterwards.
/// The folder restriction is repeated as `m.folder_id IN (...)` so SQLite,
/// which has no table statistics here, reads `messages_by_folder` by
/// `received_at` range instead of scanning every message.
const RECEIVED: &str =
    "f.role = 'inbox' AND m.folder_id IN (SELECT id FROM folders WHERE role = 'inbox')";
/// Sent-role rows, with the same planner hint.
const SENT: &str =
    "f.role = 'sent' AND m.folder_id IN (SELECT id FROM folders WHERE role = 'sent')";

/// Per-account counts of new inbox mail and the newest of those messages.
pub(crate) type NewMailRows = (Vec<(String, u32)>, Vec<EmailSummary>);

/// Most attachment names returned for one message.
const MAX_FILE_NAMES: u32 = 20;

/// One recipient of one message in the unified Sent view. `message` keys
/// the message (a server row id or a `local-sent:` id) so a recipient listed
/// twice is counted once; `address` is lowercased like every address match.
pub(crate) struct SentRecipient {
    pub message: String,
    pub address: String,
    /// The display name stored with the recipient; local Sent copies keep
    /// plain addresses only.
    pub name: Option<String>,
    pub at: String,
}

/// Inbox mail from one sender in a range: lowercased address, message count
/// and the newest `received_at`.
pub(crate) type SenderCount = (String, u32, String);

/// A recent inbox message to run the verification-code detector on. Its
/// `text` is never logged or returned as is.
pub(crate) struct CodeSource {
    pub email_id: String,
    pub account_id: String,
    pub folder_id: String,
    pub from: MailAddress,
    pub subject: String,
    pub received_at: String,
    pub body_state: BodyState,
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
                    m.received_at, m.body_state, m.preview,
                    (SELECT length(CAST(b.plain_text AS BLOB)) FROM message_bodies b
                      WHERE b.message_id = m.id),
                    (SELECT b.plain_text FROM message_bodies b
                      WHERE b.message_id = m.id AND length(CAST(b.plain_text AS BLOB)) <= ?3)
             {VISIBLE_MESSAGES} AND {INBOX} AND m.received_at >= ?1
             ORDER BY m.received_at DESC, m.id DESC LIMIT ?2"
        ))?;
        let rows = statement
            .query_map(params![after, limit, max_text_bytes], |row| {
                let body_state = body_state(&row.get::<_, String>(7)?)?;
                let ready = body_state == BodyState::Ready;
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
                    body_state,
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

    /// Inbox mail received in each half-open range `[bounds[i], bounds[i + 1])`,
    /// locally archived mail included. Each count is a range scan on
    /// `messages_by_folder`.
    pub(crate) fn overview_received(&self, bounds: &[String]) -> Result<Vec<u32>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT COUNT(*) {VISIBLE_MESSAGES} AND {RECEIVED}
             AND m.received_at >= ?1 AND m.received_at < ?2"
        ))?;
        bounds
            .windows(2)
            .map(|range| Ok(statement.query_row(params![range[0], range[1]], |row| row.get(0))?))
            .collect()
    }

    /// Inbox mail received in `[from, to)` as the inbox lists it (locally
    /// archived mail left out), and how many of those are unread.
    pub(crate) fn overview_inbox_unread(
        &self,
        from: &str,
        to: &str,
    ) -> Result<(u32, u32), AppError> {
        Ok(self.db.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(m.is_read = 0), 0) {VISIBLE_MESSAGES} AND {RECEIVED}
                 AND NOT {} AND m.received_at >= ?1 AND m.received_at < ?2",
                archived_sql!()
            ),
            params![from, to],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    /// Messages of the unified Sent view in `[from, to)`: sent-role folder
    /// rows plus local Sent copies. A local copy matched to its server copy
    /// has `server_copy_message_id` set and leaves [`LOCAL_COPIES`], exactly
    /// as in the Sent list, so each sent message counts once.
    pub(crate) fn overview_sent(&self, from: &str, to: &str) -> Result<u32, AppError> {
        Ok(self.db.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) {VISIBLE_MESSAGES} AND {SENT}
                         AND m.received_at >= ?1 AND m.received_at < ?2)
                      + (SELECT COUNT(*) {LOCAL_COPIES}
                         AND o.accepted_at >= ?1 AND o.accepted_at < ?2)"
            ),
            params![from, to],
            |row| row.get(0),
        )?)
    }

    /// Whether the cache holds every inbox and Sent message from
    /// `window_start_by` on. True when every account has a current inbox and
    /// every selectable inbox and sent-role folder of the current mailbox
    /// sources has a generation that started its window on or before
    /// `window_start_by` (`YYYY-MM-DD`), has finished at least one uncapped
    /// sync round and is not capped. Unlike the list's
    /// `Coverage.headersComplete`, a round in progress does not clear it, so
    /// the answer holds steady while folders sync.
    pub(crate) fn overview_history_complete(
        &self,
        window_start_by: &str,
    ) -> Result<bool, AppError> {
        Ok(self.db.query_row(
            "SELECT EXISTS (SELECT 1 FROM accounts)
               AND NOT EXISTS (
                 SELECT 1 FROM accounts a WHERE NOT EXISTS (
                   SELECT 1 FROM folders f WHERE f.account_id = a.id
                     AND f.mailbox_source_id = a.mailbox_source_id
                     AND f.selectable = 1 AND f.role = 'inbox'))
               AND NOT EXISTS (
                 SELECT 1 FROM folders f
                 JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
                 LEFT JOIN folder_sync_cursors c ON c.folder_id = f.id
                 WHERE f.selectable = 1 AND f.role IN ('inbox', 'sent')
                   AND (c.folder_id IS NULL OR c.history_complete = 0 OR c.window_capped = 1
                        OR c.window_start > ?1))",
            [window_start_by],
            |row| row.get(0),
        )?)
    }

    /// Inbox mail received in `[from, to)` per sender, locally archived mail
    /// included.
    pub(crate) fn overview_senders(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<SenderCount>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT lower(m.from_address), COUNT(*), MAX(m.received_at) {VISIBLE_MESSAGES}
             AND {RECEIVED} AND m.received_at >= ?1 AND m.received_at < ?2
             AND m.from_address <> '' GROUP BY lower(m.from_address)"
        ))?;
        let rows = statement
            .query_map(params![from, to], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The newest non-empty display name of inbox mail from `address`
    /// (lowercased) received in `[from, to)`, with its time. Served by
    /// `messages_by_sender`.
    pub(crate) fn overview_sender_name(
        &self,
        address: &str,
        from: &str,
        to: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        let mut statement = self.db.prepare_cached(&format!(
            "SELECT m.from_name, m.received_at {VISIBLE_MESSAGES} AND f.role = 'inbox'
             AND lower(m.from_address) = ?1 AND m.received_at >= ?2 AND m.received_at < ?3
             AND COALESCE(m.from_name, '') <> ''
             ORDER BY m.received_at DESC, m.id DESC LIMIT 1"
        ))?;
        Ok(statement
            .query_row(params![address, from, to], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?)
    }

    /// Every recipient of the Sent view's messages in `[from, to)`: To and Cc
    /// of server rows, the SMTP recipients (Bcc included) of local copies.
    pub(crate) fn overview_sent_recipients(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<SentRecipient>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "WITH s AS (SELECT m.id, m.received_at, m.to_addresses, m.cc_addresses
                        {VISIBLE_MESSAGES} AND {SENT}
                        AND m.received_at >= ?1 AND m.received_at < ?2),
                  l AS (SELECT o.id, o.accepted_at, o.envelope {LOCAL_COPIES}
                        AND o.accepted_at >= ?1 AND o.accepted_at < ?2)
             SELECT s.id, lower(json_extract(j.value, '$.address')), json_extract(j.value, '$.name'), s.received_at
               FROM s, json_each(s.to_addresses) j
             UNION ALL
             SELECT s.id, lower(json_extract(j.value, '$.address')), json_extract(j.value, '$.name'), s.received_at
               FROM s, json_each(s.cc_addresses) j
             UNION ALL
             SELECT 'local-sent:' || l.id, lower(j.value), NULL, l.accepted_at
               FROM l, json_each(l.envelope, '$.recipients') j"
        ))?;
        let rows = statement
            .query_map(params![from, to], |row| {
                Ok(SentRecipient {
                    message: row.get(0)?,
                    address: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    name: row.get(2)?,
                    at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|row| !row.address.is_empty())
            .collect())
    }

    /// Which of `candidates` (lowercased) any message of the Sent view, at
    /// any time, was addressed to.
    pub(crate) fn overview_sent_to(&self, candidates: &[String]) -> Result<Vec<String>, AppError> {
        if candidates.is_empty() {
            return Ok(vec![]);
        }
        let mut statement = self.db.prepare(&format!(
            "WITH k AS (SELECT value AS address FROM json_each(?1)),
                  s AS (SELECT m.to_addresses, m.cc_addresses {VISIBLE_MESSAGES} AND {SENT}),
                  l AS (SELECT o.envelope {LOCAL_COPIES})
             SELECT lower(json_extract(j.value, '$.address')) AS address FROM s, json_each(s.to_addresses) j
             UNION
             SELECT lower(json_extract(j.value, '$.address')) FROM s, json_each(s.cc_addresses) j
             UNION
             SELECT lower(j.value) FROM l, json_each(l.envelope, '$.recipients') j
             INTERSECT SELECT address FROM k"
        ))?;
        let candidates = serde_json::to_string(candidates)
            .map_err(|_| AppError::new(crate::ErrorCode::Internal))?;
        let rows = statement
            .query_map([candidates], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every account's own address, lowercased.
    pub(crate) fn overview_own_addresses(&self) -> Result<Vec<String>, AppError> {
        let mut statement = self
            .db
            .prepare("SELECT DISTINCT lower(email_address) FROM accounts")?;
        let rows = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
