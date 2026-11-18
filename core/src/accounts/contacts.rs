//! Contact aggregation rules and recipient suggestions (see
//! `storage/contacts.rs`).
use std::{cmp::Reverse, collections::HashMap, collections::HashSet};

use super::AccountService;
use crate::{
    AppError,
    mail::{ContactAggregation, ContactSuggestion},
    storage::contacts::{CandidateFilter, ContactStats},
};

/// Suggestions returned when the caller names no limit.
pub const DEFAULT_CONTACT_SUGGESTIONS: u32 = 8;
/// Most suggestions one query may ask for.
pub const MAX_CONTACT_SUGGESTIONS: u32 = 20;
/// Longest suggestion query, in characters, after trimming.
const MAX_SUGGESTION_QUERY_CHARS: usize = 320;
/// Local parts of addresses nobody writes to.
const NO_REPLY: [&str; 6] = [
    "noreply",
    "no-reply",
    "donotreply",
    "do-not-reply",
    "mailer-daemon",
    "postmaster",
];
/// A message sent to an address weighs this many received ones.
const SENT_WEIGHT: u32 = 3;

/// A sender address as stored and compared: trimmed and ASCII-lowercased.
/// Rejects values that cannot be an address before SQL sees them.
pub(crate) fn normalize_sender_address(value: &str, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > 320
        || !value.contains('@')
        || value.chars().any(char::is_control)
    {
        return Err(AppError::validation(field));
    }
    Ok(value.to_ascii_lowercase())
}

impl AccountService {
    pub fn list_contact_aggregations(&self) -> Result<Vec<ContactAggregation>, AppError> {
        self.storage.list_contact_aggregations()
    }

    /// Idempotent: setting an existing rule or removing a missing one succeeds.
    pub fn set_contact_aggregation(
        &mut self,
        address: &str,
        aggregated: bool,
    ) -> Result<(), AppError> {
        let address = normalize_sender_address(address, "address")?;
        self.storage.set_contact_aggregation(&address, aggregated)
    }

    /// Recipient suggestions for a composer address field (sending.md
    /// section 17.4): people written to first, then by weighted count, the
    /// newest exchange and the address. Reads only mail already cached.
    pub fn suggest_contacts(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<ContactSuggestion>, AppError> {
        self.suggest_contacts_scoped(query, limit, None)
    }

    pub(crate) fn suggest_contacts_scoped(
        &self,
        query: &str,
        limit: Option<u32>,
        account_ids: Option<&[String]>,
    ) -> Result<Vec<ContactSuggestion>, AppError> {
        let limit = limit.unwrap_or(DEFAULT_CONTACT_SUGGESTIONS);
        if !(1..=MAX_CONTACT_SUGGESTIONS).contains(&limit) {
            return Err(AppError::validation("limit"));
        }
        let query = query.trim();
        if query.chars().count() > MAX_SUGGESTION_QUERY_CHARS {
            return Err(AppError::validation("query"));
        }
        if query.is_empty() {
            return Ok(vec![]);
        }
        let wanted = query.to_lowercase();
        let own: HashSet<String> = self
            .storage
            .contact_own_addresses(account_ids)?
            .into_iter()
            .collect();
        let candidates: Vec<String> = self
            .storage
            .contact_candidates(&prefilter(query), account_ids)?
            .into_iter()
            .filter(|address| !own.contains(address) && !no_reply(address))
            .collect();
        // People written to rank first, so the others are counted only when
        // too few of them match.
        let mut written: HashMap<String, Person> = HashMap::new();
        for (address, stats) in self.storage.contact_sent(&candidates, account_ids)? {
            if stats.count > 0 {
                let person = written.entry(address).or_default();
                person.sent_to = true;
                person.add(stats, SENT_WEIGHT);
            }
        }
        let (known, others): (Vec<String>, Vec<String>) = candidates
            .into_iter()
            .partition(|address| written.contains_key(address));
        for (address, stats) in self.storage.contact_received(&known, account_ids)? {
            written.entry(address).or_default().add(stats, 1);
        }
        let matching = |people: HashMap<String, Person>| {
            people.into_iter().filter(|(address, person)| {
                matches(
                    &wanted,
                    address,
                    person.name.as_ref().map(|(_, name)| name.as_str()),
                )
            })
        };
        let mut ranked: Vec<(String, Person)> = matching(written).collect();
        if ranked.len() < limit as usize {
            let mut received: HashMap<String, Person> = HashMap::new();
            for (address, stats) in self.storage.contact_received(&others, account_ids)? {
                received.entry(address).or_default().add(stats, 1);
            }
            ranked.extend(matching(received));
        }
        ranked.sort_by_cached_key(|(address, person)| {
            (
                !person.sent_to,
                Reverse(person.score),
                Reverse(person.last.clone()),
                address.clone(),
            )
        });
        ranked.truncate(limit as usize);
        Ok(ranked
            .into_iter()
            .map(|(address, person)| ContactSuggestion {
                name: person.name.map(|(_, name)| name),
                address,
            })
            .collect())
    }
}

/// One candidate address with both sources merged.
#[derive(Default)]
struct Person {
    sent_to: bool,
    score: u32,
    last: String,
    /// (time, name) of the newest named mention.
    name: Option<(String, String)>,
}

impl Person {
    fn add(&mut self, stats: ContactStats, weight: u32) {
        self.score = self
            .score
            .saturating_add(stats.count.saturating_mul(weight));
        if self.last < stats.last {
            self.last = stats.last;
        }
        if let Some(named) = stats.name
            && self.name.as_ref().is_none_or(|current| current < &named)
        {
            self.name = Some(named);
        }
    }
}

/// The SQL pre-filter for `query`. SQLite's `lower()` folds ASCII only, so
/// a query of ASCII and caseless characters (such as CJK) is matched by
/// position in SQL; otherwise its longest such run must be contained, and
/// without one every address is a candidate.
fn prefilter(query: &str) -> CandidateFilter {
    let caseless = |c: char| {
        c.is_ascii()
            || (c.to_lowercase().eq(std::iter::once(c)) && c.to_uppercase().eq(std::iter::once(c)))
    };
    if query.chars().all(caseless) {
        return CandidateFilter::Prefix(query.to_ascii_lowercase());
    }
    query
        .split(|c: char| !caseless(c))
        .max_by_key(|run| run.chars().count())
        .filter(|run| !run.is_empty())
        .map_or(CandidateFilter::All, |run| {
            CandidateFilter::Contains(run.to_ascii_lowercase())
        })
}

/// Whether the local part names a mailbox nobody reads.
fn no_reply(address: &str) -> bool {
    let local = address.rsplit_once('@').map_or(address, |(local, _)| local);
    let local = local.to_lowercase();
    NO_REPLY.iter().any(|pattern| local.contains(pattern))
}

/// `wanted` is the lowercased query. Matches the start of the address, the
/// start of its domain, or the start of any word of the name (words split at
/// whitespace, `.`, `-` and `_`); a name with CJK text matches anywhere.
pub(crate) fn matches(wanted: &str, address: &str, name: Option<&str>) -> bool {
    let address = address.to_lowercase();
    if address.starts_with(wanted)
        || address
            .rsplit_once('@')
            .is_some_and(|(_, domain)| domain.starts_with(wanted))
    {
        return true;
    }
    let Some(name) = name.map(str::to_lowercase) else {
        return false;
    };
    if name.chars().any(is_cjk) {
        return name.contains(wanted);
    }
    name.match_indices(wanted).any(|(at, _)| {
        name[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || matches!(c, '.' | '-' | '_'))
    })
}

/// Han, kana and Hangul.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF
        | 0x2E80..=0x2FDF
        | 0x3040..=0x30FF
        | 0x3130..=0x318F
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF
        | 0x20000..=0x3134F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_covers_address_domain_name_words_and_cjk() {
        let m = |q: &str, address: &str, name: Option<&str>| matches(q, address, name);
        // Address and domain prefixes.
        assert!(m("ali", "alice@example.test", None));
        assert!(m("alice@ex", "alice@example.test", None));
        assert!(m("exa", "alice@example.test", None));
        assert!(!m("lice", "alice@example.test", None));
        assert!(!m("ample", "alice@example.test", None));
        // Word starts of the name, split at whitespace, `.`, `-`, `_`.
        for name in [
            "Alice Liu",
            "alice.liu",
            "Alice-Liu",
            "alice_liu",
            "Alice\tLiu",
        ] {
            assert!(m("liu", "a@x.test", Some(name)), "{name}");
        }
        assert!(m("alice l", "a@x.test", Some("Alice Liu")));
        assert!(!m("iu", "a@x.test", Some("Alice Liu")));
        assert!(!m("liu", "a@x.test", Some("AliceLiu")));
        // Case-insensitive beyond ASCII.
        assert!(m("öz", "a@x.test", Some("Mesut Özil")));
        // CJK names match anywhere.
        assert!(m("三", "a@x.test", Some("张三")));
        assert!(m("张", "a@x.test", Some("张三")));
        assert!(m("田中", "a@x.test", Some("山田中子")));
        assert!(m("민", "a@x.test", Some("김민수")));
        assert!(!m("李", "a@x.test", Some("张三")));
        assert!(!m("x", "a@y.test", None));
    }

    #[test]
    fn no_reply_patterns_are_matched_in_the_local_part_only() {
        for address in [
            "noreply@x.test",
            "NoReply@x.test",
            "billing-no-reply@x.test",
            "donotreply@x.test",
            "do-not-reply@x.test",
            "mailer-daemon@x.test",
            "postmaster@x.test",
        ] {
            assert!(no_reply(address), "{address}");
        }
        for address in ["alice@noreply.test", "reply@x.test", "post@x.test"] {
            assert!(!no_reply(address), "{address}");
        }
    }

    #[test]
    fn the_sql_prefilter_never_loses_a_match() {
        use CandidateFilter::{All, Contains, Prefix};
        assert_eq!(prefilter("Alice L"), Prefix("alice l".into()));
        assert_eq!(prefilter("张三"), Prefix("张三".into()));
        // Cased non-ASCII text is folded only in Rust, so the longest run
        // without it must be contained, or nothing is filtered.
        assert_eq!(prefilter("Özil"), Contains("zil".into()));
        assert_eq!(prefilter("josé"), Contains("jos".into()));
        assert_eq!(prefilter("É"), All);
    }
}
