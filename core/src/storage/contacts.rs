//! Contact settings stored on this device: which senders are aggregated into
//! one list row. Rules are global (every account) and keyed by the sender's
//! address, ASCII-lowercased the way SQLite's `lower()` does.
//!
//! Also the reads behind recipient suggestions (sending.md section 17.4),
//! built from mail already cached: no contact table is kept in step.

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, params_from_iter};

use super::{
    Storage,
    messages::{LOCAL_COPIES, VISIBLE_MESSAGES},
};
use crate::{AppError, mail::ContactAggregation};

/// The contact view leaves junk and trash out, so the counts do too.
const OUTSIDE_JUNK: &str = "f.role NOT IN ('junk', 'trash')";

/// Sent-role rows; the folder restriction is repeated as `m.folder_id IN`
/// so SQLite reads `messages_by_folder` instead of scanning every message.
const SENT: &str =
    "f.role = 'sent' AND m.folder_id IN (SELECT id FROM folders WHERE role = 'sent')";

/// What one source (received mail or sent mail) knows about one address:
/// message count, the newest message time and the newest non-empty display
/// name with its time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContactStats {
    pub count: u32,
    pub last: String,
    pub name: Option<(String, String)>,
}

/// `MAX()` over `time TAB name` picks the newest named mention; stored times
/// are fixed-width UTC and sort as text.
fn newest_name(time: &str, name: &str) -> String {
    format!(
        "MAX(CASE WHEN trim(COALESCE({name}, ''), ' ' || char(9, 10, 13)) <> ''
              THEN {time} || char(9) || trim({name}, ' ' || char(9, 10, 13)) END)"
    )
}

/// How [`Storage::contact_candidates`] narrows rows before the exact match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CandidateFilter {
    /// Every address.
    All,
    /// The value contains this ASCII-lowercased text.
    Contains(String),
    /// The whole query, already lowercased, where SQLite's ASCII-only
    /// `lower()` folds it exactly as Rust does: the address or its domain
    /// starts with it, or the name has it at the start of a word. Names with
    /// other whitespace than a plain space, or with CJK text, fall back to
    /// containing it.
    Prefix(String),
}

/// Characters `char::is_whitespace` accepts besides the plain space, as a
/// GLOB class.
const OTHER_WHITESPACE: &str =
    "*[\u{9}-\u{d}\u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}]*";
/// A superset of the CJK ranges the exact match treats as CJK, as a GLOB class.
const CJK: &str =
    "*[\u{1100}-\u{11ff}\u{2e80}-\u{9fff}\u{ac00}-\u{d7af}\u{f900}-\u{faff}\u{20000}-\u{3134f}]*";

impl CandidateFilter {
    /// The condition on one address expression and its name expression.
    /// Parameters: ?1 the text, ?2 the text escaped for LIKE, ?3 and ?4 the
    /// GLOB classes.
    fn sql(&self, address: &str, name: Option<&str>) -> String {
        let address = format!("lower(COALESCE({address}, ''))");
        let name = name.map(|name| format!("lower(COALESCE({name}, ''))"));
        match self {
            Self::All => "1".into(),
            Self::Contains(_) => {
                let mut sql = format!("instr({address}, ?1) > 0");
                if let Some(name) = name {
                    sql.push_str(&format!(" OR instr({name}, ?1) > 0"));
                }
                sql
            }
            Self::Prefix(_) => {
                let mut sql = format!(
                    "{address} LIKE ?2 || '%' ESCAPE '\\' OR {address} LIKE '%@' || ?2 || '%' ESCAPE '\\'"
                );
                if let Some(name) = name {
                    sql.push_str(&format!(
                        " OR (instr({name}, ?1) > 0 AND ({name} LIKE ?2 || '%' ESCAPE '\\'
                           OR {name} LIKE '% ' || ?2 || '%' ESCAPE '\\'
                           OR {name} LIKE '%.' || ?2 || '%' ESCAPE '\\'
                           OR {name} LIKE '%-' || ?2 || '%' ESCAPE '\\'
                           OR {name} LIKE '%\\_' || ?2 || '%' ESCAPE '\\'
                           OR {name} GLOB ?3 OR {name} GLOB ?4))"
                    ));
                }
                sql
            }
        }
    }

    /// Exactly the parameters [`Self::sql`] refers to.
    fn params(&self) -> Vec<String> {
        match self {
            Self::All => vec![],
            Self::Contains(text) => vec![text.clone()],
            Self::Prefix(text) => vec![
                text.clone(),
                text.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_"),
                OTHER_WHITESPACE.into(),
                CJK.into(),
            ],
        }
    }
}

fn stats(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, ContactStats)> {
    let count: i64 = row.get(1)?;
    let name: Option<String> = row.get(3)?;
    Ok((
        row.get(0)?,
        ContactStats {
            count: u32::try_from(count).unwrap_or(u32::MAX),
            last: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            name: name.and_then(|value| {
                let (time, name) = value.split_once('\t')?;
                Some((time.to_owned(), name.to_owned()))
            }),
        },
    ))
}

impl Storage {
    /// Adds or removes the rule for a normalized address; idempotent.
    pub(crate) fn set_contact_aggregation(
        &mut self,
        address: &str,
        aggregated: bool,
    ) -> Result<(), AppError> {
        if aggregated {
            self.db.execute(
                "INSERT OR IGNORE INTO contact_aggregations(address, created_at) VALUES (?1, ?2)",
                params![
                    address,
                    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
                ],
            )?;
        } else {
            self.db.execute(
                "DELETE FROM contact_aggregations WHERE address = ?1",
                [address],
            )?;
        }
        Ok(())
    }

    /// Every rule with the newest visible sender name and the visible message
    /// count outside junk and trash (what the contact view lists), ordered by
    /// name (or address) without regard to case.
    pub(crate) fn list_contact_aggregations(&self) -> Result<Vec<ContactAggregation>, AppError> {
        let sql = format!(
            "SELECT r.address, r.created_at,
                (SELECT COUNT(*) {VISIBLE_MESSAGES} AND lower(m.from_address) = r.address AND {OUTSIDE_JUNK}),
                (SELECT m.from_name {VISIBLE_MESSAGES} AND lower(m.from_address) = r.address AND {OUTSIDE_JUNK}
                    ORDER BY m.received_at DESC, m.id DESC LIMIT 1),
                (SELECT MAX(m.received_at) {VISIBLE_MESSAGES} AND lower(m.from_address) = r.address AND {OUTSIDE_JUNK})
             FROM contact_aggregations r"
        );
        let mut statement = self.db.prepare(&sql)?;
        let mut items = statement
            .query_map(params_from_iter(std::iter::empty::<i64>()), |row| {
                let count: i64 = row.get(2)?;
                let name: Option<String> = row.get(3)?;
                Ok(ContactAggregation {
                    address: row.get(0)?,
                    name: name
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty()),
                    message_count: u32::try_from(count).unwrap_or(u32::MAX),
                    latest_received_at: row.get(4)?,
                    created_at: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by_cached_key(|item| {
            (
                item.name.as_deref().unwrap_or(&item.address).to_lowercase(),
                item.address.clone(),
            )
        });
        Ok(items)
    }

    /// Lowercased addresses that may match a suggestion query: senders, and
    /// recipients of sent mail (sent-role rows and accepted local copies),
    /// whose address or display name passes `filter`. A superset checked on
    /// raw rows of any folder; visibility and the exact match come later.
    pub(crate) fn contact_candidates(
        &self,
        filter: &CandidateFilter,
    ) -> Result<Vec<String>, AppError> {
        let address = "json_extract(j.value, '$.address')";
        let name = "json_extract(j.value, '$.name')";
        // Both sides are deduplicated before the filter runs: distinct
        // (address, name) pairs straight from `messages_by_sender_name`, and
        // distinct recipient lists of sent-role rows.
        let sql = format!(
            "WITH p AS MATERIALIZED (SELECT DISTINCT from_address AS address, from_name AS name
                                       FROM messages),
                  u AS MATERIALIZED (SELECT DISTINCT to_addresses AS to_list, cc_addresses AS cc_list
                                       FROM messages
                                      WHERE folder_id IN (SELECT id FROM folders WHERE role = 'sent')),
                  t AS (SELECT to_list AS list FROM u UNION SELECT cc_list FROM u)
             SELECT lower(p.address) FROM p WHERE p.address <> '' AND ({})
             UNION
             SELECT lower({address}) FROM t, json_each(t.list) j WHERE ({})
             UNION
             SELECT lower(j.value) FROM outbox o, json_each(o.envelope, '$.recipients') j
              WHERE o.state = 'accepted' AND ({})",
            filter.sql("p.address", Some("p.name")),
            filter.sql(address, Some(name)),
            filter.sql("j.value", None),
        );
        let mut statement = self.db.prepare(&sql)?;
        let rows = statement
            .query_map(params_from_iter(filter.params()), |row| {
                row.get::<_, Option<String>>(0)
            })?
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|address| !address.is_empty())
            .collect())
    }

    /// Received mail from each of `addresses` (lowercased): visible mail
    /// outside junk and trash, what the contact view counts. Served by
    /// `messages_by_sender`.
    pub(crate) fn contact_received(
        &self,
        addresses: &[String],
    ) -> Result<Vec<(String, ContactStats)>, AppError> {
        if addresses.is_empty() {
            return Ok(vec![]);
        }
        let sql = format!(
            "SELECT lower(m.from_address), COUNT(*), MAX(m.received_at), {}
             {VISIBLE_MESSAGES} AND {OUTSIDE_JUNK}
               AND lower(m.from_address) IN (SELECT value FROM json_each(?1))
             GROUP BY lower(m.from_address)",
            newest_name("m.received_at", "m.from_name"),
        );
        Ok(self
            .db
            .prepare(&sql)?
            .query_map([keys(addresses)?], stats)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Mail of the unified Sent view addressed to each of `addresses`
    /// (lowercased): To and Cc of sent-role rows, the SMTP recipients (Bcc
    /// included) of local Sent copies. A recipient listed twice on one
    /// message counts once.
    pub(crate) fn contact_sent(
        &self,
        addresses: &[String],
    ) -> Result<Vec<(String, ContactStats)>, AppError> {
        if addresses.is_empty() {
            return Ok(vec![]);
        }
        let sql = format!(
            "WITH k AS (SELECT value AS address FROM json_each(?1)),
                  s AS (SELECT m.id, m.received_at, m.to_addresses, m.cc_addresses
                        {VISIBLE_MESSAGES} AND {SENT}),
                  l AS (SELECT o.id, o.accepted_at, o.envelope {LOCAL_COPIES}),
                  r(message, address, name, at) AS (
                    SELECT s.id, lower(json_extract(j.value, '$.address')),
                           json_extract(j.value, '$.name'), s.received_at
                      FROM s, json_each(s.to_addresses) j
                    UNION ALL
                    SELECT s.id, lower(json_extract(j.value, '$.address')),
                           json_extract(j.value, '$.name'), s.received_at
                      FROM s, json_each(s.cc_addresses) j
                    UNION ALL
                    SELECT 'local-sent:' || l.id, lower(j.value), NULL, l.accepted_at
                      FROM l, json_each(l.envelope, '$.recipients') j)
             SELECT r.address, COUNT(DISTINCT r.message), MAX(r.at), {}
               FROM r WHERE r.address IN (SELECT address FROM k)
              GROUP BY r.address",
            newest_name("r.at", "r.name"),
        );
        Ok(self
            .db
            .prepare(&sql)?
            .query_map([keys(addresses)?], stats)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

fn keys(addresses: &[String]) -> Result<String, AppError> {
    serde_json::to_string(addresses).map_err(|_| AppError::new(crate::ErrorCode::Internal))
}
