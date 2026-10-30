//! Local full-text index (storage-search.md 5.1).
//!
//! Every write path that changes indexed text calls [`refresh_message`] or
//! [`refresh_local`] inside its own transaction; deletions (including folder
//! and account cascades) are handled by SQL triggers from migration 19. The
//! index is derived data: [`Storage::ensure_search_index_version`] rebuilds it
//! when [`INDEX_VERSION`] changes, and [`Storage::search_index_backfill_step`]
//! fills it for rows that predate it.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value as Json;

use super::Storage;
use crate::error::AppError;
use crate::mail::SnippetSegment;

/// Bump when tokenization, folding or the indexed columns change.
pub(crate) const INDEX_VERSION: i64 = 1;
/// Plain text cached in the index per message.
pub(crate) const BODY_LIMIT: usize = 64 * 1024;
pub(crate) const BACKFILL_BATCH: usize = 300;
pub(crate) const MAX_TERMS: usize = 16;
pub(crate) const MAX_QUERY_CHARS: usize = 256;
/// Private-use markers around `snippet()` hits; stripped from indexed text.
const MARK_START: char = '\u{E000}';
const MARK_END: char = '\u{E001}';
use crate::mail::LOCAL_SENT_PREFIX as LOCAL_PREFIX;
const SNIPPET_TOKENS: usize = 64;
const SHORT_CONTEXT: usize = 40;

/// Full-width ASCII (U+FF01..=U+FF5E) becomes ASCII and U+3000 a space.
pub(crate) fn fold_width(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ => c,
        })
        .collect()
}

fn index_text(text: &str) -> String {
    fold_width(text)
        .chars()
        .filter(|c| *c != MARK_START && *c != MARK_END)
        .collect()
}

fn truncate_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ---------------------------------------------------------------- query

/// Terms of a validated search text, already width-folded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchTerms {
    /// At least three Unicode scalar values: served by the trigram index.
    pub long: Vec<String>,
    /// Shorter terms: served by LIKE over the indexed columns.
    pub short: Vec<String>,
}

/// Splits on whitespace, treating double-quoted segments as one term.
/// `Ok(None)` means the text is blank and behaves as no text filter.
pub(crate) fn parse_query(text: &str) -> Result<Option<SearchTerms>, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::validation("text"));
    }
    let folded = index_text(text);
    let mut raw: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut flush = |current: &mut String| {
        let term = current.trim();
        if !term.is_empty() {
            raw.push(term.to_owned());
        }
        current.clear();
    };
    for c in folded.chars() {
        match c {
            '"' => {
                flush(&mut current);
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => flush(&mut current),
            c => current.push(c),
        }
    }
    flush(&mut current);
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.len() > MAX_TERMS {
        return Err(AppError::validation("text"));
    }
    let mut terms = SearchTerms::default();
    for term in raw {
        if term.chars().count() >= 3 {
            terms.long.push(term);
        } else {
            terms.short.push(term);
        }
    }
    Ok(Some(terms))
}

fn match_expression(long: &[String]) -> String {
    long.iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn like_pattern(term: &str) -> String {
    let mut pattern = String::from("%");
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

const LIKE_COLUMNS: [&str; 5] = ["subject", "people", "preview", "body", "attachments"];

impl SearchTerms {
    /// `SELECT email_id ...` subquery over the index: MATCH narrows first and
    /// each short term post-filters through LIKE over all five columns.
    pub(crate) fn email_ids_subquery(&self) -> (String, Vec<Value>) {
        let mut conditions = Vec::new();
        let mut values = Vec::new();
        if !self.long.is_empty() {
            conditions.push("search_fts MATCH ?".to_owned());
            values.push(Value::from(match_expression(&self.long)));
        }
        for term in &self.short {
            let any = LIKE_COLUMNS
                .iter()
                .map(|column| format!("search_fts.{column} LIKE ? ESCAPE '\\'"))
                .collect::<Vec<_>>()
                .join(" OR ");
            conditions.push(format!("({any})"));
            for _ in LIKE_COLUMNS {
                values.push(Value::from(like_pattern(term)));
            }
        }
        (
            format!(
                "SELECT d.email_id FROM search_fts JOIN search_docs d ON d.rowid = search_fts.rowid WHERE {}",
                conditions.join(" AND ")
            ),
            values,
        )
    }
}

// ---------------------------------------------------------------- documents

#[derive(Default)]
struct Doc {
    subject: String,
    people: String,
    preview: String,
    body: String,
    attachments: String,
    has_body: bool,
}

fn addresses_text(json: Option<String>, out: &mut Vec<String>) {
    let Some(Json::Array(items)) = json.and_then(|raw| serde_json::from_str(&raw).ok()) else {
        return;
    };
    for item in items {
        for key in ["name", "address"] {
            if let Some(value) = item.get(key).and_then(Json::as_str) {
                out.push(value.to_owned());
            }
        }
    }
}

fn write_doc(db: &Connection, email_id: &str, doc: &Doc) -> rusqlite::Result<()> {
    let existing: Option<i64> = db
        .query_row(
            "SELECT rowid FROM search_docs WHERE email_id = ?1",
            [email_id],
            |row| row.get(0),
        )
        .optional()?;
    let rowid = match existing {
        Some(rowid) => {
            db.execute("DELETE FROM search_fts WHERE rowid = ?1", [rowid])?;
            db.execute(
                "UPDATE search_docs SET has_body = ?2 WHERE rowid = ?1",
                params![rowid, doc.has_body],
            )?;
            rowid
        }
        None => {
            db.execute(
                "INSERT INTO search_docs (email_id, has_body) VALUES (?1, ?2)",
                params![email_id, doc.has_body],
            )?;
            db.last_insert_rowid()
        }
    };
    db.execute(
        "INSERT INTO search_fts (rowid, subject, people, preview, body, attachments)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            rowid,
            index_text(&doc.subject),
            index_text(&doc.people),
            index_text(&doc.preview),
            index_text(truncate_utf8(&doc.body, BODY_LIMIT)),
            index_text(&doc.attachments),
        ],
    )?;
    Ok(())
}

fn remove_email(db: &Connection, email_id: &str) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM search_fts WHERE rowid IN (SELECT rowid FROM search_docs WHERE email_id = ?1)",
        [email_id],
    )?;
    db.execute("DELETE FROM search_docs WHERE email_id = ?1", [email_id])?;
    Ok(())
}

/// Re-reads a server message from the database and rewrites its index row, or
/// drops the row when the message is gone or no longer `present`.
pub(crate) fn refresh_message(db: &Connection, message_id: &str) -> rusqlite::Result<()> {
    let row = db
        .query_row(
            "SELECT m.subject, m.from_name, m.from_address, m.to_addresses, m.cc_addresses,
                    m.preview, m.remote_state, m.body_state,
                    substr(b.plain_text, 1, ?2)
             FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id
             WHERE m.id = ?1",
            params![message_id, BODY_LIMIT as i64],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((subject, from_name, from_address, to, cc, preview, remote_state, body_state, body)) =
        row
    else {
        return remove_email(db, message_id);
    };
    if remote_state != "present" {
        return remove_email(db, message_id);
    }
    let mut people = Vec::new();
    people.extend(from_name);
    people.extend(from_address);
    addresses_text(to, &mut people);
    addresses_text(cc, &mut people);
    let mut statement = db.prepare_cached(
        "SELECT filename FROM message_parts
         WHERE message_id = ?1 AND is_inline = 0 AND filename IS NOT NULL ORDER BY part_index",
    )?;
    let attachments = statement
        .query_map([message_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    write_doc(
        db,
        message_id,
        &Doc {
            subject: subject.unwrap_or_default(),
            people: people.join(" "),
            preview: preview.unwrap_or_default(),
            body: body.unwrap_or_default(),
            attachments: attachments.join(" "),
            has_body: body_state == "ready",
        },
    )
}

pub(crate) fn refresh_by_uid(
    db: &Connection,
    folder_id: &str,
    uid_validity: u32,
    uid: u32,
) -> rusqlite::Result<()> {
    let id: Option<String> = db
        .query_row(
            "SELECT id FROM messages WHERE folder_id = ?1 AND uid_validity = ?2 AND uid = ?3",
            params![folder_id, uid_validity, uid],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => refresh_message(db, &id),
        None => Ok(()),
    }
}

pub(crate) fn remove_folder(db: &Connection, folder_id: &str) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM search_fts WHERE rowid IN (
            SELECT d.rowid FROM search_docs d JOIN messages m ON m.id = d.email_id
            WHERE m.folder_id = ?1)",
        [folder_id],
    )?;
    db.execute(
        "DELETE FROM search_docs WHERE email_id IN (SELECT id FROM messages WHERE folder_id = ?1)",
        [folder_id],
    )?;
    Ok(())
}

/// Indexes a local sent copy when it qualifies for the sent list (accepted,
/// not matched to a server copy, not removed, snapshot still held); otherwise
/// drops its row. Attachment names are not recorded for local copies.
pub(crate) fn refresh_local(db: &Connection, outbox_id: &str) -> rusqlite::Result<()> {
    let email_id = format!("{LOCAL_PREFIX}{outbox_id}");
    let row = db
        .query_row(
            "SELECT subject, recipients_preview, preview FROM outbox
             WHERE id = ?1 AND state = 'accepted' AND server_copy_message_id IS NULL
               AND deleted_locally_at IS NULL AND snapshot_released_at IS NULL
               AND snapshot_hash IS NOT NULL",
            [outbox_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((subject, recipients, preview)) = row else {
        return remove_email(db, &email_id);
    };
    let recipients: Vec<String> = serde_json::from_str(&recipients).unwrap_or_default();
    write_doc(
        db,
        &email_id,
        &Doc {
            subject: subject.unwrap_or_default(),
            people: recipients.join(" "),
            preview: preview.unwrap_or_default(),
            ..Doc::default()
        },
    )
}

pub(crate) fn remove_local(db: &Connection, outbox_id: &str) -> rusqlite::Result<()> {
    remove_email(db, &format!("{LOCAL_PREFIX}{outbox_id}"))
}

// ---------------------------------------------------------------- state

/// Result of the optional consistency check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IndexConsistency {
    /// Present messages without an index row.
    pub missing: u64,
    /// Index rows whose message no longer exists or is not present.
    pub orphans: u64,
}

impl Storage {
    /// Drops the index when the stored version differs from the code's so the
    /// background backfill rebuilds it.
    pub(crate) fn ensure_search_index_version(&mut self) -> Result<(), AppError> {
        let stored: i64 = self.db.query_row(
            "SELECT version FROM search_index_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        if stored == INDEX_VERSION {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM search_fts", [])?;
        tx.execute("DELETE FROM search_docs", [])?;
        tx.execute(
            "UPDATE search_index_state
             SET version = ?1, backfill_done = 0, messages_cursor = '', local_done = 0 WHERE id = 1",
            [INDEX_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Indexes one batch of existing rows in one transaction and returns
    /// whether the backfill is finished. Resumable through the state table.
    pub(crate) fn search_index_backfill_step(&mut self, batch: usize) -> Result<bool, AppError> {
        let (done, cursor, local_done): (bool, String, bool) = self.db.query_row(
            "SELECT backfill_done, messages_cursor, local_done FROM search_index_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if done {
            return Ok(true);
        }
        let tx = self.db.transaction()?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT id FROM messages WHERE remote_state = 'present' AND id > ?1
                 ORDER BY id LIMIT ?2",
            )?;
            statement
                .query_map(params![cursor, batch as i64], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if let Some(last) = ids.last() {
            for id in &ids {
                refresh_message(&tx, id)?;
            }
            tx.execute(
                "UPDATE search_index_state SET messages_cursor = ?1 WHERE id = 1",
                [last],
            )?;
        } else if !local_done {
            let ids = {
                let mut statement = tx.prepare(
                    "SELECT id FROM outbox WHERE state = 'accepted' AND server_copy_message_id IS NULL
                       AND deleted_locally_at IS NULL AND snapshot_released_at IS NULL
                       AND snapshot_hash IS NOT NULL",
                )?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for id in &ids {
                refresh_local(&tx, id)?;
            }
            tx.execute(
                "UPDATE search_index_state SET local_done = 1 WHERE id = 1",
                [],
            )?;
        } else {
            tx.execute(
                "UPDATE search_index_state SET backfill_done = 1 WHERE id = 1",
                [],
            )?;
            tx.commit()?;
            return Ok(true);
        }
        tx.commit()?;
        Ok(false)
    }

    /// Counts rows that disagree with `messages`. Scans both tables, so call
    /// it on demand rather than at startup.
    #[allow(dead_code)]
    pub(crate) fn search_index_consistency(&self) -> Result<IndexConsistency, AppError> {
        let missing: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM messages m WHERE m.remote_state = 'present'
               AND NOT EXISTS (SELECT 1 FROM search_docs d WHERE d.email_id = m.id)",
            [],
            |row| row.get(0),
        )?;
        let orphans: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM search_docs d WHERE d.email_id NOT LIKE 'local-sent:%'
               AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.id = d.email_id AND m.remote_state = 'present')",
            [],
            |row| row.get(0),
        )?;
        Ok(IndexConsistency {
            missing: missing as u64,
            orphans: orphans as u64,
        })
    }

    /// Snippets for one page of results. Only hits inside the body or the
    /// attachment names produce a snippet; subject and people hits do not.
    pub(crate) fn search_snippets(
        &self,
        terms: &SearchTerms,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<SnippetSegment>>, AppError> {
        let mut result = std::collections::HashMap::new();
        if ids.is_empty() {
            return Ok(result);
        }
        let marks = vec!["?"; ids.len()].join(",");
        let mut statement = self.db.prepare(&format!(
            "SELECT d.email_id, d.rowid FROM search_docs d WHERE d.email_id IN ({marks})"
        ))?;
        let rowids = statement
            .query_map(rusqlite::params_from_iter(ids), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rowids.is_empty() {
            return Ok(result);
        }
        let list = rowids
            .iter()
            .map(|(_, rowid)| rowid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let by_rowid: std::collections::HashMap<i64, &String> =
            rowids.iter().map(|(id, rowid)| (*rowid, id)).collect();
        let (start, end) = (MARK_START.to_string(), MARK_END.to_string());
        if !terms.long.is_empty() {
            let mut statement = self.db.prepare(&format!(
                "SELECT rowid, snippet(search_fts, 3, ?2, ?3, '…', {SNIPPET_TOKENS}),
                        snippet(search_fts, 4, ?2, ?3, '…', {SNIPPET_TOKENS})
                 FROM search_fts WHERE search_fts MATCH ?1 AND rowid IN ({list})"
            ))?;
            let rows =
                statement.query_map(params![match_expression(&terms.long), start, end], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
            for row in rows {
                let (rowid, body, attachments) = row?;
                let hit = [body, attachments]
                    .into_iter()
                    .find(|snippet| snippet.contains(MARK_START));
                if let (Some(snippet), Some(id)) = (hit, by_rowid.get(&rowid)) {
                    result.insert((*id).clone(), split_snippet(&snippet));
                }
            }
        }
        if !terms.short.is_empty() {
            let mut statement = self.db.prepare(&format!(
                "SELECT rowid, body, attachments FROM search_fts WHERE rowid IN ({list})"
            ))?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (rowid, body, attachments) = row?;
                let Some(id) = by_rowid.get(&rowid) else {
                    continue;
                };
                if result.contains_key(*id) {
                    continue;
                }
                let found = [body, attachments]
                    .iter()
                    .find_map(|text| short_snippet(text, &terms.short));
                if let Some(segments) = found {
                    result.insert((*id).clone(), segments);
                }
            }
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------- snippets

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

/// Splits `snippet()` output on the private-use markers into plain segments.
pub(crate) fn split_snippet(snippet: &str) -> Vec<SnippetSegment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut hit = false;
    let mut push = |text: &mut String, hit: bool| {
        if !text.is_empty() {
            segments.push(SnippetSegment {
                text: collapse_whitespace_keep_edges(text),
                hit,
            });
            text.clear();
        }
    };
    for c in snippet.chars() {
        if c == MARK_START {
            push(&mut text, hit);
            hit = true;
        } else if c == MARK_END {
            push(&mut text, hit);
            hit = false;
        } else {
            text.push(c);
        }
    }
    push(&mut text, hit);
    segments
}

fn collapse_whitespace_keep_edges(text: &str) -> String {
    let lead = text.starts_with(char::is_whitespace);
    let trail = text.ends_with(char::is_whitespace);
    let mut out = collapse_whitespace(text);
    if lead {
        out.insert(0, ' ');
    }
    if trail && !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

fn fold_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Snippet around the first occurrence of any short term, +-40 characters.
fn short_snippet(text: &str, terms: &[String]) -> Option<Vec<SnippetSegment>> {
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars.iter().map(|c| fold_char(*c)).collect();
    let mut best: Option<(usize, usize)> = None;
    for term in terms {
        let needle: Vec<char> = term.chars().map(fold_char).collect();
        if needle.is_empty() || needle.len() > folded.len() {
            continue;
        }
        let found = folded
            .windows(needle.len())
            .position(|w| w == needle.as_slice());
        if let Some(at) = found
            && best.is_none_or(|(start, _)| at < start)
        {
            best = Some((at, needle.len()));
        }
    }
    let (at, len) = best?;
    let from = at.saturating_sub(SHORT_CONTEXT);
    let to = (at + len + SHORT_CONTEXT).min(chars.len());
    let mut segments = Vec::new();
    let mut before = String::new();
    if from > 0 {
        before.push('…');
    }
    before.extend(&chars[from..at]);
    let mut after: String = chars[at + len..to].iter().collect();
    if to < chars.len() {
        after.push('…');
    }
    if !before.is_empty() {
        segments.push(SnippetSegment {
            text: collapse_whitespace_keep_edges(&before),
            hit: false,
        });
    }
    segments.push(SnippetSegment {
        text: chars[at..at + len].iter().collect(),
        hit: true,
    });
    if !after.is_empty() {
        segments.push(SnippetSegment {
            text: collapse_whitespace_keep_edges(&after),
            hit: false,
        });
    }
    Some(segments)
}

#[cfg(test)]
mod tests;
