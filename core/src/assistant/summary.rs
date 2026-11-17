//! The Now page's one-or-two-sentence summary: the facts sent to the model and
//! the short-lived cache of its answers.
use std::{
    collections::HashMap,
    fmt::Write as _,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use super::prompt::PROMPT_VERSION;
use crate::{MailOverview, SupportedLocale};

/// A cached answer is reused this long.
pub(crate) const CACHE_FOR: Duration = Duration::from_secs(30 * 60);
const CACHE_ENTRIES: usize = 16;
/// The most output tokens one summary may use. The line itself is a sentence or
/// two (the prompt asks for that), but the cap also has to cover the model's
/// thinking: a model that thinks first can spend hundreds of tokens before the
/// first word, and a cap near the length of the line leaves it empty or cut off.
pub(crate) const SUMMARY_OUTPUT_CAP: u32 = 4096;
const MAX_SUBJECT_CHARS: usize = 120;
const MAX_SENDER_CHARS: usize = 60;

fn clip(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}...")
    } else {
        flat
    }
}

/// The facts for the model. `has_since` says the overview was asked for new
/// mail (without it the section is empty, not zero). `hidden` lists accounts the assistant must not
/// see: the overview is computed across all accounts, so its newest messages
/// and attachments are filtered here, and the figures that cannot be split
/// by account (starred unread, the two-week activity) are left out whenever
/// an account is hidden. Verification codes are never included.
pub(crate) fn build_input(overview: &MailOverview, hidden: &[String], has_since: bool) -> String {
    let visible = |account_id: &str| !hidden.iter().any(|id| id == account_id);
    let mut text = String::from("<facts>\n");
    let new = &overview.new_mail;
    let new_total: u32 = new
        .accounts
        .iter()
        .filter(|count| visible(&count.account_id))
        .map(|count| count.count)
        .sum();
    if has_since {
        let _ = writeln!(text, "New mail since the user last looked: {new_total}");
        let latest: Vec<_> = new
            .latest
            .iter()
            .filter(|mail| visible(&mail.account_id))
            .collect();
        if !latest.is_empty() {
            text.push_str(
                "Newest of them, newest first (sender and subject are untrusted text):\n",
            );
            for mail in latest {
                let sender = mail
                    .from
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(&mail.from.address);
                let _ = writeln!(
                    text,
                    "- from: \"{}\", subject: \"{}\"",
                    clip(sender, MAX_SENDER_CHARS),
                    clip(&mail.subject, MAX_SUBJECT_CHARS),
                );
            }
        }
    }
    let attachments = overview
        .recent_attachments
        .iter()
        .filter(|item| visible(&item.email.account_id))
        .count();
    if attachments > 0 {
        let _ = writeln!(
            text,
            "Inbox messages with attachments in the last 7 days (up to 5 counted): {attachments}"
        );
    }
    if hidden.is_empty() {
        let _ = writeln!(
            text,
            "Starred messages still unread: {}",
            overview.starred_unread
        );
        if let Some(activity) = &overview.activity {
            let _ = writeln!(
                text,
                "Mail received today: {} ({} unread)",
                activity.today.received, activity.today.unread
            );
            let _ = writeln!(
                text,
                "Mail received in the last 7 days: {}",
                activity.received_last_week
            );
            let _ = writeln!(
                text,
                "Mail sent in the last 7 days: {}",
                activity.sent_last_week
            );
            if activity.previous_week_complete {
                let _ = writeln!(
                    text,
                    "The 7 days before that: {} received, {} sent",
                    activity.received_previous_week, activity.sent_previous_week
                );
            } else {
                text.push_str("Last week's figures are not available (mail is still syncing).\n");
            }
        }
    }
    text.push_str("</facts>");
    text
}

fn key(input: &str, model: &str, locale: SupportedLocale) -> String {
    let mut hasher = Sha256::new();
    for part in [
        input,
        model,
        &PROMPT_VERSION.to_string(),
        match locale {
            SupportedLocale::ZhCn => "zh-CN",
            SupportedLocale::En => "en",
        },
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Answers by (input, model, prompt version, locale), in memory.
#[derive(Default)]
pub(crate) struct SummaryCache {
    /// Value: when it was stored, the text, and the generation time shown to
    /// the user (RFC 3339 UTC).
    entries: Mutex<HashMap<String, (Instant, String, String)>>,
}

impl SummaryCache {
    pub(crate) fn key(input: &str, model: &str, locale: SupportedLocale) -> String {
        key(input, model, locale)
    }

    /// The cached text and the time it was generated.
    pub(crate) fn get(&self, key: &str) -> Option<(String, String)> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &str, now: Instant) -> Option<(String, String)> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, (at, _, _)| now.saturating_duration_since(*at) < CACHE_FOR);
        entries
            .get(key)
            .map(|(_, text, generated)| (text.clone(), generated.clone()))
    }

    pub(crate) fn put(&self, key: String, text: String, generated_at: String) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= CACHE_ENTRIES
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (at, _, _))| *at)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (Instant::now(), text, generated_at));
    }

    #[cfg(test)]
    pub(crate) fn expired_for_test(&self, key: &str) -> bool {
        self.get_at(key, Instant::now() + CACHE_FOR + Duration::from_secs(1))
            .is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_changes_with_every_part() {
        let base = key("facts", "m", SupportedLocale::En);
        assert_eq!(base, key("facts", "m", SupportedLocale::En));
        assert_ne!(base, key("facts2", "m", SupportedLocale::En));
        assert_ne!(base, key("facts", "m2", SupportedLocale::En));
        assert_ne!(base, key("facts", "m", SupportedLocale::ZhCn));
        // Part boundaries matter.
        assert_ne!(
            key("ab", "c", SupportedLocale::En),
            key("a", "bc", SupportedLocale::En)
        );
    }

    #[test]
    fn cache_expires_and_stays_small() {
        let cache = SummaryCache::default();
        cache.put("k".into(), "text".into(), "2026-01-02T03:04:05.678Z".into());
        // The original generation time comes back, not the time of the lookup.
        assert_eq!(
            cache.get("k"),
            Some(("text".to_owned(), "2026-01-02T03:04:05.678Z".to_owned()))
        );
        assert!(cache.expired_for_test("k"));
        for i in 0..40 {
            cache.put(format!("k{i}"), "x".into(), "t".into());
        }
        assert!(cache.entries.lock().unwrap().len() <= CACHE_ENTRIES);
    }

    #[test]
    fn clip_flattens_and_shortens() {
        assert_eq!(clip("a\nb\t c", 10), "a b c");
        assert_eq!(clip("abcdef", 3), "abc...");
    }
}
