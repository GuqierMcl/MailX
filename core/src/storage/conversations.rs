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
/// At most this many referenced IDs (the most recent ones) are looked up.
const REFERENCE_LIMIT: usize = 20;
/// Bumped whenever the threading rules change in a way that needs a rebuild:
/// `conversation_id` is derived data, and rows below this version are
/// re-threaded in the background (see [`Storage::rethread_step`]).
pub(crate) const THREADING_VERSION: i64 = 1;

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
    if wanted.len() > REFERENCE_LIMIT {
        wanted.drain(..wanted.len() - REFERENCE_LIMIT);
    }
    // Rows are related when they are one of the referenced messages or when
    // they reference one of them too (siblings of an uncached root).
    let marks = vec!["?"; wanted.len()].join(",");
    let tokens = vec![
        "instr(' ' || COALESCE(references_header, '') || ' ', ' ' || ? || ' ') > 0";
        wanted.len()
    ]
    .join(" OR ");
    let sql = format!(
        "SELECT id, conversation_id FROM messages
         WHERE account_id = ? AND id <> ?
           AND (message_id_header IN ({marks}) OR in_reply_to IN ({marks}) OR {tokens})
         ORDER BY received_at ASC, id ASC"
    );
    let mut values: Vec<Value> = vec![message.account_id.clone().into(), message.id.clone().into()];
    for _ in 0..3 {
        values.extend(wanted.iter().map(|id| Value::from((*id).to_owned())));
    }
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

/// Lower-cased addresses of a JSON `[{name, address}]` column.
fn addresses(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<serde_json::Value>>(json)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.get("address")?.as_str().map(str::to_ascii_lowercase))
        .collect()
}

/// Who the message was exchanged with: the recipients of a message the
/// account sent, otherwise its sender.
fn counterpart(me: &str, from: &str, to: &str, cc: &str) -> (bool, HashSet<String>) {
    let from = from.to_ascii_lowercase();
    if from == me {
        (
            true,
            addresses(to).into_iter().chain(addresses(cc)).collect(),
        )
    } else {
        (false, HashSet::from([from]))
    }
}

/// Subject fallback joins only a real two-way exchange with one counterpart:
/// one-way notifications sharing a subject are not a conversation.
fn by_subject(tx: &Connection, message: &Row) -> Result<Option<String>, AppError> {
    let subject = normalize_subject(&message.subject);
    if subject.is_empty() {
        return Ok(None);
    }
    let me: String = tx.query_row(
        "SELECT lower(email_address) FROM accounts WHERE id = ?1",
        [&message.account_id],
        |row| row.get(0),
    )?;
    let (own_sent, own_peers) = tx.query_row(
        "SELECT from_address, to_addresses, cc_addresses FROM messages WHERE id = ?1",
        [&message.id],
        |row| {
            Ok(counterpart(
                &me,
                &row.get::<_, String>(0)?,
                &row.get::<_, String>(1)?,
                &row.get::<_, String>(2)?,
            ))
        },
    )?;
    // SQLite folds only ASCII case, so the pre-filter works on the stripped
    // subject with the same folding; the exact comparison happens below.
    let prefilter = strip_prefixes(&message.subject).to_ascii_lowercase();
    let mut statement = tx.prepare(
        "SELECT id, conversation_id, subject, from_address, to_addresses, cc_addresses FROM messages
         WHERE account_id = ?1 AND id <> ?2 AND in_reply_to IS NULL AND references_header IS NULL
           AND abs(julianday(received_at) - julianday(?3)) <= ?4
           AND instr(lower(subject), ?5) > 0
         ORDER BY received_at ASC, id ASC",
    )?;
    let candidates = statement.query_map(
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
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        },
    )?;
    let (mut matched, mut any_sent, mut any_received) = (Matched::new(), own_sent, !own_sent);
    for candidate in candidates {
        let (id, cid, other, from, to, cc) = candidate?;
        if normalize_subject(&other) != subject {
            continue;
        }
        let (sent, peers) = counterpart(&me, &from, &to, &cc);
        if peers.is_disjoint(&own_peers) {
            continue;
        }
        any_sent |= sent;
        any_received |= !sent;
        matched.push((id, cid));
    }
    if matched.is_empty() || matched.len() + 1 > SUBJECT_GROUP_LIMIT || !(any_sent && any_received)
    {
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
        if let Some(row) = load(&tx, id)? {
            messages.push(row);
        }
    }
    // Oldest first, so a reply in the same batch finds its parent already linked.
    messages.sort_by(|a, b| a.received_at.cmp(&b.received_at).then(a.id.cmp(&b.id)));
    let mut assigned: Vec<(Row, String)> = Vec::new();
    for message in messages {
        // Already linked (stored earlier, or pulled in by an earlier message
        // of this batch): nothing to derive, only the version to record.
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
    // Every processed row records the rules it was threaded with, linked or not.
    for id in new_message_ids {
        tx.execute(
            "UPDATE messages SET threading_version = ?1 WHERE id = ?2 AND threading_version < ?1",
            params![THREADING_VERSION, id],
        )?;
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
    /// Threads up to `limit` present messages still on older rules, oldest
    /// first so parents precede replies, and returns how many it processed.
    pub(crate) fn rethread_step(&mut self, limit: usize) -> Result<usize, AppError> {
        // The literal lets SQLite use the partial `messages_pending_rethread` index.
        let ids: Vec<String> = self
            .db
            .prepare(&format!(
                "SELECT id FROM messages WHERE threading_version < {THREADING_VERSION}
                 AND remote_state = 'present' ORDER BY received_at ASC, id ASC LIMIT ?1"
            ))?
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        assign_conversations(self, &ids)?;
        Ok(ids.len())
    }

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
