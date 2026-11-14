//! The two-week activity section of the overview: mail received and sent per
//! local day, today's inbox and the people corresponded with most.
//!
//! The host sends the local day starts, so the core never needs the user's
//! time zone and days stay correct across daylight-saving changes.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use ts_rs::TS;

use super::timestamp;
use crate::{AppError, accounts::AccountService, accounts::messages::normalized_time};

/// Day starts a query must send: the previous seven days and the last seven.
pub const ACTIVITY_DAYS: usize = 14;
/// Most contacts listed.
pub const OVERVIEW_CONTACTS: usize = 3;
/// The first day of the last seven.
const LAST_WEEK: usize = ACTIVITY_DAYS - 7;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OverviewActivity {
    /// One entry per requested day start, oldest first: inbox mail received
    /// that day, locally archived mail included. Junk, Trash and Sent never
    /// count. The last day ends at `generatedAt`.
    pub days: Vec<DayActivity>,
    /// Sum of the last seven days.
    pub received_last_week: u32,
    /// Sum of the seven days before them.
    pub received_previous_week: u32,
    /// Messages of the unified Sent view (sent-role folders and local Sent
    /// copies, a copy matched to its server copy counted once) in the last
    /// seven days.
    pub sent_last_week: u32,
    /// The same for the seven days before them.
    pub sent_previous_week: u32,
    /// Whether the local cache can be trusted to hold both weeks, so the
    /// week-over-week comparison means something. True only when every
    /// account has an inbox and every current inbox and sent-role folder
    /// generation has finished a sync round, is not capped by the per-round
    /// limit, and started its sync window at least one UTC day before the
    /// first day start (the margin covers servers that read the window's
    /// date in their own time zone).
    pub previous_week_complete: bool,
    /// At most three addresses with the most messages in the last seven days:
    /// inbox mail received from them (archived included) plus Sent view
    /// messages addressed to them. Only addresses the Sent view holds at least
    /// one message to qualify, and the accounts' own addresses never do.
    /// Most messages first, then the most recent.
    pub top_contacts: Vec<OverviewContact>,
    pub today: TodayActivity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DayActivity {
    /// RFC 3339 UTC; the requested day start, normalized.
    pub start: String,
    pub received: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OverviewContact {
    /// Lowercased.
    pub address: String,
    /// The most recent non-empty display name among the counted messages.
    /// Untrusted text.
    pub name: Option<String>,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TodayActivity {
    /// Inbox mail received since the last day start, as the inbox lists it:
    /// locally archived mail is left out.
    pub received: u32,
    /// How many of those are unread.
    pub unread: u32,
}

/// Exactly [`ACTIVITY_DAYS`] valid, strictly ascending instants, the last not
/// after `now`; returned normalized.
pub(super) fn validate_day_starts(
    starts: &[String],
    now: DateTime<Utc>,
) -> Result<Vec<String>, AppError> {
    const FIELD: &str = "dayStarts";
    if starts.len() != ACTIVITY_DAYS {
        return Err(AppError::validation(FIELD));
    }
    let starts = starts
        .iter()
        .map(|start| normalized_time(start, FIELD))
        .collect::<Result<Vec<_>, _>>()?;
    // Normalized values sort lexically in time order.
    let ascending = starts.windows(2).all(|pair| pair[0] < pair[1]);
    if !ascending || starts[ACTIVITY_DAYS - 1] > timestamp(now) {
        return Err(AppError::validation(FIELD));
    }
    Ok(starts)
}

#[derive(Default)]
struct Tally {
    count: u32,
    last: String,
    /// (time, name) of the newest named mention.
    name: Option<(String, String)>,
    sent_to: bool,
}

impl Tally {
    fn seen(&mut self, at: &str) {
        if self.last.as_str() < at {
            at.clone_into(&mut self.last);
        }
    }

    fn named(&mut self, at: &str, name: Option<&str>) {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return;
        };
        if self
            .name
            .as_ref()
            .is_none_or(|(when, _)| when.as_str() < at)
        {
            self.name = Some((at.to_owned(), name.to_owned()));
        }
    }
}

/// The activity as of `now`; `starts` is already validated.
pub(super) fn activity_at(
    service: &AccountService,
    starts: &[String],
    now: DateTime<Utc>,
) -> Result<OverviewActivity, AppError> {
    // Stored times have millisecond precision, so `< end` means `<= now`.
    let end = timestamp(now + Duration::milliseconds(1));
    let mut bounds = starts.to_vec();
    bounds.push(end.clone());
    let received = service.overview_received(&bounds)?;
    let sum = |counts: &[u32]| counts.iter().fold(0u32, |sum, n| sum.saturating_add(*n));
    let last_week = &starts[LAST_WEEK];
    let today_start = &starts[ACTIVITY_DAYS - 1];
    let (today_received, today_unread) = service.overview_inbox_unread(today_start, &end)?;
    let first = DateTime::parse_from_rfc3339(&starts[0])
        .map_err(|_| AppError::validation("dayStarts"))?
        .with_timezone(&Utc);
    let window_start_by = (first.date_naive() - Duration::days(1)).to_string();
    Ok(OverviewActivity {
        days: starts
            .iter()
            .zip(&received)
            .map(|(start, received)| DayActivity {
                start: start.clone(),
                received: *received,
            })
            .collect(),
        received_last_week: sum(&received[LAST_WEEK..]),
        received_previous_week: sum(&received[..LAST_WEEK]),
        sent_last_week: service.overview_sent(last_week, &end)?,
        sent_previous_week: service.overview_sent(&starts[0], last_week)?,
        previous_week_complete: service.overview_history_complete(&window_start_by)?,
        top_contacts: top_contacts(service, last_week, &end)?,
        today: TodayActivity {
            received: today_received,
            unread: today_unread,
        },
    })
}

fn top_contacts(
    service: &AccountService,
    from: &str,
    to: &str,
) -> Result<Vec<OverviewContact>, AppError> {
    let mut tally: HashMap<String, Tally> = HashMap::new();
    for (address, count, last) in service.overview_senders(from, to)? {
        let entry = tally.entry(address).or_default();
        entry.count = entry.count.saturating_add(count);
        entry.seen(&last);
    }
    let mut counted = HashSet::new();
    for recipient in service.overview_sent_recipients(from, to)? {
        // A recipient listed in both To and Cc counts once per message.
        if !counted.insert((recipient.message, recipient.address.clone())) {
            continue;
        }
        let entry = tally.entry(recipient.address).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.sent_to = true;
        entry.seen(&recipient.at);
        entry.named(&recipient.at, recipient.name.as_deref());
    }
    for own in service.overview_own_addresses()? {
        tally.remove(&own);
    }
    // Recipients in range already qualify; senders need a message sent to
    // them at any time.
    let unknown: Vec<String> = tally
        .iter()
        .filter(|(_, entry)| !entry.sent_to)
        .map(|(address, _)| address.clone())
        .collect();
    let sent_to: HashSet<String> = service.overview_sent_to(&unknown)?.into_iter().collect();
    let mut ranked: Vec<(String, Tally)> = tally
        .into_iter()
        .filter(|(address, entry)| entry.sent_to || sent_to.contains(address))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| {
        y.count
            .cmp(&x.count)
            .then_with(|| y.last.cmp(&x.last))
            .then_with(|| a.cmp(b))
    });
    ranked.truncate(OVERVIEW_CONTACTS);
    ranked
        .into_iter()
        .map(|(address, mut entry)| {
            if let Some((name, at)) = service.overview_sender_name(&address, from, to)? {
                entry.named(&at, Some(&name));
            }
            Ok(OverviewContact {
                name: entry.name.map(|(_, name)| name),
                address,
                count: entry.count,
            })
        })
        .collect()
}
