//! Contact settings stored on this device: which senders are aggregated into
//! one list row. Rules are global (every account) and keyed by the sender's
//! address, ASCII-lowercased the way SQLite's `lower()` does.

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, params_from_iter};

use super::{Storage, messages::VISIBLE_MESSAGES};
use crate::{AppError, mail::ContactAggregation};

/// The contact view leaves junk and trash out, so the counts do too.
const OUTSIDE_JUNK: &str = "f.role NOT IN ('junk', 'trash')";

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
}
