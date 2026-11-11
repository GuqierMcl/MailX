//! Conversation threading over cached headers.
//!
//! A conversation is an account-scoped set of messages linked by References /
//! In-Reply-To, with a conservative subject fallback for messages that carry no
//! threading headers. Messages that cannot be linked simply stay without a
//! `conversation_id` and display alone.
use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value};
use uuid::Uuid;

use super::Storage;
use crate::AppError;

/// Subject fallback only joins messages this close in time.
const SUBJECT_WINDOW_DAYS: f64 = 30.0;
/// A subject shared by more messages than this is a mailing pattern, not a thread.
const SUBJECT_GROUP_LIMIT: usize = 20;

struct Row {
    id: String,
    account_id: String,
    message_id: Option<String>,
    subject: String,
    received_at: String,
    in_reply_to: Option<String>,
    references: Option<String>,
    conversation_id: Option<String>,
}

fn load(db: &Connection, id: &str) -> Result<Option<Row>, AppError> {
    Ok(db
        .query_row(
            "SELECT id, account_id, message_id_header, subject, received_at, in_reply_to,
                    references_header, conversation_id FROM messages WHERE id = ?1",
            [id],
            |row| {
                Ok(Row {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    message_id: row.get(2)?,
                    subject: row.get(3)?,
                    received_at: row.get(4)?,
                    in_reply_to: row.get(5)?,
                    references: row.get(6)?,
                    conversation_id: row.get(7)?,
                })
            },
        )
        .optional()?)
}

/// Subject without reply / forward prefixes, trimmed and lower-cased.
pub(crate) fn normalize_subject(subject: &str) -> String {
    strip_prefixes(subject).to_lowercase()
}

fn strip_prefixes(subject: &str) -> &str {
    let mut rest = subject.trim();
    'again: loop {
        for prefix in ["re", "fwd", "fw", "回复", "答复", "转发"] {
            let Some(head) = rest.get(..prefix.len()) else {
                continue;
            };
            if !head.eq_ignore_ascii_case(prefix) {
                continue;
            }
            let tail = rest[prefix.len()..].trim_start();
            if let Some(after) = tail.strip_prefix(':').or_else(|| tail.strip_prefix('：')) {
                rest = after.trim_start();
                continue 'again;
            }
        }
        return rest.trim();
    }
}

/// Matched rows (oldest first) as `(id, conversation_id)`.
type Matched = Vec<(String, Option<String>)>;

/// The conversation to use for a set of related rows, plus `extra` rows that
/// join it: the earliest existing id, or a new one.
fn adopt(tx: &Connection, matched: &Matched, member_ids: &[&str]) -> Result<String, AppError> {
    let cid = matched
        .iter()
        .find_map(|(_, cid)| cid.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    for id in member_ids
        .iter()
        .copied()
        .chain(matched.iter().map(|(id, _)| id.as_str()))
    {
        tx.execute(
            "UPDATE messages SET conversation_id = ?1 WHERE id = ?2 AND conversation_id IS NULL",
            params![cid, id],
        )?;
    }
    Ok(cid)
}

fn by_references(tx: &Connection, message: &Row) -> Result<Option<String>, AppError> {
    let mut wanted: Vec<&str> = Vec::new();
    let mut seen = HashSet::new();
    for id in message
        .references
        .iter()
        .flat_map(|list| list.split_whitespace())
        .chain(message.in_reply_to.as_deref())
    {
        let id = id.trim_matches(['<', '>']);
        if !id.is_empty() && message.message_id.as_deref() != Some(id) && seen.insert(id) {
            wanted.push(id);
        }
    }
    if wanted.is_empty() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT id, conversation_id FROM messages
         WHERE account_id = ? AND id <> ? AND message_id_header IN ({})
         ORDER BY received_at ASC, id ASC",
        vec!["?"; wanted.len()].join(",")
    );
    let mut values: Vec<Value> = vec![message.account_id.clone().into(), message.id.clone().into()];
    values.extend(wanted.iter().map(|id| Value::from((*id).to_owned())));
    let matched: Matched = tx
        .prepare(&sql)?
        .query_map(params_from_iter(values), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    // Even with no cached relative, the message opens a conversation that
    // later arrivals can join.
    Ok(Some(adopt(tx, &matched, &[&message.id])?))
}

/// A message that arrived after its replies: they already name it.
fn by_children(tx: &Connection, message: &Row) -> Result<Option<String>, AppError> {
    let Some(message_id) = message.message_id.as_deref() else {
        return Ok(None);
    };
    let matched: Matched = tx
        .prepare(
            "SELECT id, conversation_id FROM messages
             WHERE account_id = ?1 AND id <> ?2
               AND (in_reply_to = ?3 OR instr(' ' || COALESCE(references_header, '') || ' ', ' ' || ?3 || ' ') > 0)
             ORDER BY received_at ASC, id ASC",
        )?
        .query_map(params![message.account_id, message.id, message_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    if matched.is_empty() {
        return Ok(None);
    }
    Ok(Some(adopt(tx, &matched, &[&message.id])?))
}

/// Subject matching is only for messages without threading headers, so it
/// never merges into a header-built thread.
fn by_subject_fallback(tx: &Connection, message: &Row) -> Result<Option<String>, AppError> {
    if message.in_reply_to.is_some() || message.references.is_some() {
        return Ok(None);
    }
    by_subject(tx, message)
}

fn by_subject(tx: &Connection, message: &Row) -> Result<Option<String>, AppError> {
    let subject = normalize_subject(&message.subject);
    if subject.is_empty() {
        return Ok(None);
    }
    // SQLite folds only ASCII case, so the pre-filter works on the stripped
    // subject with the same folding; the exact comparison happens below.
    let prefilter = strip_prefixes(&message.subject).to_ascii_lowercase();
    let mut statement = tx.prepare(
        "SELECT id, conversation_id, subject FROM messages
         WHERE account_id = ?1 AND id <> ?2 AND in_reply_to IS NULL AND references_header IS NULL
           AND abs(julianday(received_at) - julianday(?3)) <= ?4
           AND instr(lower(subject), ?5) > 0
         ORDER BY received_at ASC, id ASC",
    )?;
    let matched: Matched = statement
        .query_map(
            params![
                message.account_id,
                message.id,
                message.received_at,
                SUBJECT_WINDOW_DAYS,
                prefilter
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .filter_map(|row| match row {
            Ok((id, cid, other)) => (normalize_subject(&other) == subject).then_some(Ok((id, cid))),
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<_, _>>()?;
    if matched.is_empty() || matched.len() + 1 > SUBJECT_GROUP_LIMIT {
        return Ok(None);
    }
    Ok(Some(adopt(tx, &matched, &[&message.id])?))
}

/// Assigns `conversation_id` to the given just-stored messages. Never fails
/// the caller's sync: the caller decides what to do with an error.
pub(crate) fn assign_conversations(
    storage: &mut Storage,
    new_message_ids: &[String],
) -> Result<(), AppError> {
    if new_message_ids.is_empty() {
        return Ok(());
    }
    let tx = storage.db.transaction()?;
    let mut messages = Vec::new();
    for id in new_message_ids {
        if let Some(row) = load(&tx, id)?.filter(|row| row.conversation_id.is_none()) {
            messages.push(row);
        }
    }
    // Oldest first, so a reply in the same batch finds its parent already linked.
    messages.sort_by(|a, b| a.received_at.cmp(&b.received_at).then(a.id.cmp(&b.id)));
    let mut assigned: Vec<(Row, String)> = Vec::new();
    for message in messages {
        // An earlier message of this batch may have pulled this one in.
        let current = load(&tx, &message.id)?.and_then(|row| row.conversation_id);
        let cid = match current {
            Some(cid) => Some(cid),
            None => match by_references(&tx, &message)? {
                Some(cid) => Some(cid),
                None => match by_children(&tx, &message)? {
                    Some(cid) => Some(cid),
                    None => by_subject_fallback(&tx, &message)?,
                },
            },
        };
        if let Some(cid) = cid {
            assigned.push((message, cid));
        }
    }
    // Older cached messages that arrived before their parent.
    for (message, cid) in &assigned {
        let Some(message_id) = message.message_id.as_deref() else {
            continue;
        };
        tx.execute(
            "UPDATE messages SET conversation_id = ?1
             WHERE account_id = ?2 AND conversation_id IS NULL AND id <> ?3
               AND (in_reply_to = ?4 OR instr(' ' || COALESCE(references_header, '') || ' ', ' ' || ?4 || ' ') > 0)",
            params![cid, message.account_id, message.id, message_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

impl Storage {
    /// Best-effort threading of just-stored rows. Threading is derived data:
    /// a failure here is dropped and the rows are retried by the next batch
    /// that touches them, so it can never fail a sync.
    pub(crate) fn thread_stored(&mut self, folder_id: &str, uid_validity: u32, uids: &[u32]) {
        if uids.is_empty() {
            return;
        }
        let ids = (|| -> Result<Vec<String>, AppError> {
            let sql = format!(
                "SELECT id FROM messages WHERE folder_id = ? AND uid_validity = ? AND conversation_id IS NULL
                 AND uid IN ({})",
                vec!["?"; uids.len()].join(",")
            );
            let mut values: Vec<Value> =
                vec![folder_id.to_owned().into(), i64::from(uid_validity).into()];
            values.extend(uids.iter().map(|uid| Value::from(i64::from(*uid))));
            Ok(self
                .db
                .prepare(&sql)?
                .query_map(params_from_iter(values), |row| row.get(0))?
                .collect::<Result<_, _>>()?)
        })();
        if let Ok(ids) = ids {
            let _ = assign_conversations(self, &ids);
        }
    }
}
