//! The notable things in the Now page's numbers, found here so that the model
//! only has to pick one and say it well. Interest comes from what is noticed,
//! not from how it is worded, and a number the model has to derive is a number
//! it can get wrong.
//!
//! Every observation is a plain English sentence with its figures, produced
//! only when its condition really holds. Anything computed across all accounts
//! is skipped when any account is hidden from the assistant, exactly as the
//! facts are.
use chrono::Duration;

use super::{
    MAX_SENDER_CHARS, clip,
    moment::{Moment, ago, weekday_name},
};
use crate::MailOverview;

/// Most observations listed, most interesting first.
pub(super) const MAX_OBSERVATIONS: usize = 5;
/// Away at least this long counts as "back after a while".
pub(super) const AWAY_HOURS: i64 = 24;
/// Today is busy at this many messages or more...
pub(super) const BUSY_MIN: u32 = 5;
/// ...and at least this many times the daily average of the previous 13 days.
pub(super) const BUSY_FACTOR: u32 = 2;
/// Week over week, received mail needs at least this many in both weeks.
pub(super) const RECEIVED_WEEK_MIN: u32 = 5;
/// Sent mail is rarer, so its weeks need fewer.
pub(super) const SENT_WEEK_MIN: u32 = 3;
/// A rise of this percent or more is worth saying (+50%)...
pub(super) const WEEK_UP_PERCENT: u64 = 50;
/// ...and so is a fall to this percent of the week before or less (-33%).
pub(super) const WEEK_DOWN_TO_PERCENT: u64 = 67;
/// The person with the most messages this week needs at least this many.
pub(super) const CONTACT_MIN: u32 = 3;
/// The busiest of the last days needs at least this many messages...
pub(super) const BUSIEST_DAY_MIN: u32 = 5;
/// ...and 3/2 of the runner-up.
const BUSIEST_LEAD: (u32, u32) = (3, 2);

/// What the observations are computed from.
pub(super) struct Inputs<'a> {
    pub overview: &'a MailOverview,
    /// Some account is hidden from the assistant.
    pub any_hidden: bool,
    /// New mail since the user last looked, visible accounts only; `None`
    /// when the overview was not asked for any.
    pub new_total: Option<u32>,
    /// How long ago the user last looked.
    pub away: Option<Duration>,
    pub moment: Option<&'a Moment>,
}

/// The observations that hold, most interesting first, at most
/// [`MAX_OBSERVATIONS`].
pub(super) fn observe(inputs: &Inputs) -> Vec<String> {
    let rules: [fn(&Inputs) -> Option<String>; 10] = [
        away,
        busy_today,
        quiet,
        received_week,
        sent_week,
        top_contact,
        busiest_day,
        starred_unread,
        all_read_today,
        late_night_or_weekend,
    ];
    rules
        .iter()
        .filter_map(|rule| rule(inputs))
        .take(MAX_OBSERVATIONS)
        .collect()
}

fn plural(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The activity section, unless an account is hidden (its figures cannot be
/// split by account).
fn activity<'a>(inputs: &Inputs<'a>) -> Option<&'a crate::OverviewActivity> {
    if inputs.any_hidden {
        return None;
    }
    inputs.overview.activity.as_ref()
}

fn away(inputs: &Inputs) -> Option<String> {
    let gap = inputs
        .away
        .filter(|gap| *gap >= Duration::hours(AWAY_HOURS))?;
    let new = inputs.new_total?;
    let meanwhile = if new == 0 {
        "no new mail arrived meanwhile".to_owned()
    } else {
        format!(
            "{} arrived meanwhile",
            plural(new, "new message", "new messages")
        )
    };
    Some(format!(
        "The user is back after a while: they last looked {}, and {meanwhile}.",
        ago(gap)
    ))
}

/// The previous 13 days' received mail, when the history can be trusted.
fn previous_days_total(inputs: &Inputs) -> Option<u32> {
    let activity = activity(inputs)?;
    if !activity.previous_week_complete || activity.days.len() != 14 {
        return None;
    }
    Some(activity.days[..13].iter().map(|day| day.received).sum())
}

fn busy_today(inputs: &Inputs) -> Option<String> {
    let today = activity(inputs)?.today.received;
    let before = previous_days_total(inputs)?;
    if today < BUSY_MIN || u64::from(today) * 13 < u64::from(BUSY_FACTOR) * u64::from(before) {
        return None;
    }
    let usual = (before + 6) / 13;
    let usual = if usual == 0 {
        "fewer than one a day".to_owned()
    } else {
        format!("about {usual} a day")
    };
    Some(format!(
        "Today is busier than usual: {today} messages received so far, against {usual} over the previous 13 days."
    ))
}

fn quiet(inputs: &Inputs) -> Option<String> {
    let activity = activity(inputs)?;
    if inputs.new_total? != 0 || activity.today.received != 0 {
        return None;
    }
    // After a long absence the absence is the story, and says the same.
    if inputs
        .away
        .is_some_and(|gap| gap >= Duration::hours(AWAY_HOURS))
    {
        return None;
    }
    Some(
        "It is quiet: nothing new since the user last looked, and nothing received today so far."
            .to_owned(),
    )
}

/// The change from `before` to `now` as a percent, when it is a rise of
/// [`WEEK_UP_PERCENT`] or more or a fall to [`WEEK_DOWN_TO_PERCENT`] or less.
fn notable_change(now: u32, before: u32) -> Option<(&'static str, u64)> {
    let (now, before) = (u64::from(now), u64::from(before));
    let percent = |difference: u64| (difference * 100 + before / 2) / before;
    if now * 100 >= before * (100 + WEEK_UP_PERCENT) {
        Some(("up", percent(now - before)))
    } else if now * 100 <= before * WEEK_DOWN_TO_PERCENT {
        Some(("down", percent(before - now)))
    } else {
        None
    }
}

fn received_week(inputs: &Inputs) -> Option<String> {
    let activity = activity(inputs)?;
    let (now, before) = (activity.received_last_week, activity.received_previous_week);
    if !activity.previous_week_complete || now < RECEIVED_WEEK_MIN || before < RECEIVED_WEEK_MIN {
        return None;
    }
    let (direction, percent) = notable_change(now, before)?;
    Some(format!(
        "Mail received in the last 7 days is {direction} {percent}% on the 7 days before ({now} against {before})."
    ))
}

fn sent_week(inputs: &Inputs) -> Option<String> {
    let activity = activity(inputs)?;
    let (now, before) = (activity.sent_last_week, activity.sent_previous_week);
    if !activity.previous_week_complete || before < SENT_WEEK_MIN {
        return None;
    }
    if now == 0 {
        return Some(format!(
            "The user sent nothing in the last 7 days; in the 7 days before they sent {before}."
        ));
    }
    if now < SENT_WEEK_MIN {
        return None;
    }
    let (direction, percent) = notable_change(now, before)?;
    Some(format!(
        "Mail sent in the last 7 days is {direction} {percent}% on the 7 days before ({now} against {before})."
    ))
}

fn top_contact(inputs: &Inputs) -> Option<String> {
    let contact = activity(inputs)?.top_contacts.first()?;
    if contact.count < CONTACT_MIN {
        return None;
    }
    let name = contact
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&contact.address);
    Some(format!(
        "The most back-and-forth in the last 7 days is with \"{}\": {} counting both directions.",
        clip(name, MAX_SENDER_CHARS),
        plural(contact.count, "message", "messages"),
    ))
}

/// The busiest of the six full days before today, when it clearly stands out
/// and today (still going) is not busier.
fn busiest_day(inputs: &Inputs) -> Option<String> {
    let activity = activity(inputs)?;
    let moment = inputs.moment?;
    if activity.days.len() != 14 {
        return None;
    }
    let full = &activity.days[7..13];
    let (index, top) = full
        .iter()
        .enumerate()
        .max_by_key(|(_, day)| day.received)?;
    let top_count = top.received;
    let runner_up = full
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .map(|(_, day)| day.received)
        .max()
        .unwrap_or(0);
    if top_count < BUSIEST_DAY_MIN
        || top_count * BUSIEST_LEAD.1 < runner_up * BUSIEST_LEAD.0
        || top_count <= runner_up
        || top_count < activity.today.received
    {
        return None;
    }
    let name = if index == full.len() - 1 {
        "Yesterday".to_owned()
    } else {
        weekday_name(moment.weekday_of_day(&top.start)?).to_owned()
    };
    Some(format!(
        "{name} was the busiest of the last 7 days: {} received.",
        plural(top_count, "message", "messages")
    ))
}

fn starred_unread(inputs: &Inputs) -> Option<String> {
    if inputs.any_hidden || inputs.overview.starred_unread == 0 {
        return None;
    }
    let count = inputs.overview.starred_unread;
    Some(format!(
        "{} still unread.",
        plural(count, "starred message is", "starred messages are")
    ))
}

fn all_read_today(inputs: &Inputs) -> Option<String> {
    let today = &activity(inputs)?.today;
    if today.received == 0 || today.unread != 0 {
        return None;
    }
    Some(format!(
        "Everything received today ({}) has been read.",
        plural(today.received, "message", "messages")
    ))
}

fn late_night_or_weekend(inputs: &Inputs) -> Option<String> {
    let moment = inputs.moment?;
    let today = activity(inputs).map_or(0, |activity| activity.today.received);
    if inputs.new_total.unwrap_or(0) == 0 && today == 0 {
        return None;
    }
    if moment.is_late_night() {
        Some("It is late at night and mail is still arriving.".to_owned())
    } else if moment.is_weekend() {
        Some("It is the weekend and mail is still arriving.".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DayActivity, NewMailOverview, OverviewActivity, OverviewContact, TodayActivity};

    /// Shanghai, Wednesday 2026-11-18, 14:00 local.
    fn moment(now: &str) -> Moment {
        Moment::derive("2026-11-17T16:00:00.000Z", now).unwrap()
    }

    fn day_start(index: usize) -> String {
        // Local midnights in Shanghai, day 13 being 2026-11-18.
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-17T16:00:00Z").unwrap()
            - Duration::days(13 - index as i64);
        start
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    fn activity_with(days: [u32; 14]) -> OverviewActivity {
        OverviewActivity {
            days: days
                .iter()
                .enumerate()
                .map(|(i, received)| DayActivity {
                    start: day_start(i),
                    received: *received,
                })
                .collect(),
            received_last_week: days[7..].iter().sum(),
            received_previous_week: days[..7].iter().sum(),
            sent_last_week: 0,
            sent_previous_week: 0,
            previous_week_complete: true,
            top_contacts: vec![],
            today: TodayActivity {
                received: days[13],
                unread: days[13],
            },
        }
    }

    fn overview(activity: Option<OverviewActivity>) -> MailOverview {
        MailOverview {
            new_mail: NewMailOverview::default(),
            starred_unread: 0,
            unread_senders: vec![],
            verification_codes: vec![],
            code_candidates: vec![],
            recent_attachments: vec![],
            activity,
            generated_at: "2026-11-18T06:00:00.000Z".into(),
        }
    }

    struct Case {
        overview: MailOverview,
        any_hidden: bool,
        new_total: Option<u32>,
        away: Option<Duration>,
        moment: Option<Moment>,
    }

    impl Case {
        fn new(activity: Option<OverviewActivity>) -> Self {
            Self {
                overview: overview(activity),
                any_hidden: false,
                new_total: Some(0),
                away: Some(Duration::hours(2)),
                moment: Some(moment("2026-11-18T06:00:00.000Z")),
            }
        }

        fn quiet_week() -> Self {
            let mut case = Self::new(Some(activity_with([3; 14])));
            case.overview.activity.as_mut().unwrap().today.unread = 3;
            case
        }

        fn run(&self) -> Vec<String> {
            observe(&Inputs {
                overview: &self.overview,
                any_hidden: self.any_hidden,
                new_total: self.new_total,
                away: self.away,
                moment: self.moment.as_ref(),
            })
        }

        fn activity(&mut self) -> &mut OverviewActivity {
            self.overview.activity.as_mut().unwrap()
        }
    }

    fn has(found: &[String], needle: &str) -> bool {
        found.iter().any(|line| line.contains(needle))
    }

    #[test]
    fn an_ordinary_day_has_nothing_to_notice() {
        let found = Case::quiet_week().run();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn coming_back_after_a_day_or_more_says_how_much_arrived() {
        let mut case = Case::quiet_week();
        case.away = Some(Duration::hours(24) - Duration::minutes(1));
        case.new_total = Some(4);
        assert!(!has(&case.run(), "back after a while"));
        case.away = Some(Duration::hours(24));
        assert_eq!(
            case.run()[0],
            "The user is back after a while: they last looked 1 day ago, and 4 new messages arrived meanwhile."
        );
        case.new_total = Some(1);
        case.away = Some(Duration::days(3));
        assert!(has(&case.run(), "3 days ago, and 1 new message arrived"));
        case.new_total = Some(0);
        assert!(has(&case.run(), "no new mail arrived meanwhile"));
        // No new-mail section, no claim about it.
        case.new_total = None;
        assert!(!has(&case.run(), "back after a while"));
        // An absence works without the activity section too.
        let mut bare = Case::new(None);
        bare.away = Some(Duration::days(2));
        bare.new_total = Some(7);
        assert!(has(&bare.run(), "7 new messages arrived"));
    }

    #[test]
    fn a_quiet_stretch_needs_no_new_mail_and_nothing_today() {
        let mut case = Case::new(Some(activity_with([
            3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0,
        ])));
        assert!(has(&case.run(), "It is quiet"));
        case.new_total = Some(1);
        assert!(!has(&case.run(), "It is quiet"));
        case.new_total = Some(0);
        case.activity().today.received = 1;
        assert!(!has(&case.run(), "It is quiet"));
        case.activity().today.received = 0;
        // Away for long: the absence line says it instead.
        case.away = Some(Duration::days(2));
        let found = case.run();
        assert!(!has(&found, "It is quiet") && has(&found, "no new mail arrived"));
        // Without a since there is no "since the last look".
        case.away = None;
        case.new_total = None;
        assert!(!has(&case.run(), "It is quiet"));
    }

    #[test]
    fn a_busy_day_is_at_least_double_the_usual_and_at_least_five() {
        // Previous 13 days: 26 in all, an average of 2.
        let mut days = [2; 14];
        days[13] = 4;
        assert!(!has(&Case::new(Some(activity_with(days))).run(), "busier"));
        days[13] = 5;
        let found = Case::new(Some(activity_with(days))).run();
        assert!(
            has(
                &found,
                "Today is busier than usual: 5 messages received so far, against about 2 a day over the previous 13 days."
            ),
            "{found:?}"
        );
        // Five is not enough when the usual is higher: 13 days of 3 doubled is 6.
        let mut days = [3; 14];
        days[13] = 5;
        assert!(!has(&Case::new(Some(activity_with(days))).run(), "busier"));
        days[13] = 6;
        assert!(has(&Case::new(Some(activity_with(days))).run(), "busier"));
        // Almost no earlier mail.
        let mut days = [0; 14];
        days[13] = 5;
        assert!(has(
            &Case::new(Some(activity_with(days))).run(),
            "fewer than one a day"
        ));
        // Incomplete history says nothing about "usual".
        let mut case = Case::new(Some(activity_with(days)));
        case.activity().previous_week_complete = false;
        assert!(!has(&case.run(), "busier"));
    }

    #[test]
    fn received_mail_week_over_week_needs_a_real_change_in_real_numbers() {
        let mut case = Case::quiet_week();
        let set = |case: &mut Case, last: u32, before: u32| {
            let activity = case.activity();
            activity.received_last_week = last;
            activity.received_previous_week = before;
        };
        set(&mut case, 15, 10);
        assert!(has(
            &case.run(),
            "is up 50% on the 7 days before (15 against 10)"
        ));
        set(&mut case, 14, 10);
        assert!(!has(&case.run(), "Mail received"));
        set(&mut case, 67, 100);
        assert!(has(
            &case.run(),
            "is down 33% on the 7 days before (67 against 100)"
        ));
        set(&mut case, 68, 100);
        assert!(!has(&case.run(), "Mail received"));
        // Either week under five: no comparison.
        set(&mut case, 4, 20);
        assert!(!has(&case.run(), "Mail received"));
        set(&mut case, 20, 4);
        assert!(!has(&case.run(), "Mail received"));
        // The previous week may be incomplete.
        set(&mut case, 40, 10);
        case.activity().previous_week_complete = false;
        assert!(!has(&case.run(), "Mail received"));
    }

    #[test]
    fn sent_mail_follows_the_same_idea_and_notices_silence() {
        let mut case = Case::quiet_week();
        let set = |case: &mut Case, last: u32, before: u32| {
            let activity = case.activity();
            activity.sent_last_week = last;
            activity.sent_previous_week = before;
        };
        set(&mut case, 0, 3);
        assert!(has(
            &case.run(),
            "The user sent nothing in the last 7 days; in the 7 days before they sent 3."
        ));
        set(&mut case, 0, 2);
        assert!(!has(&case.run(), "sent"));
        set(&mut case, 6, 4);
        assert!(has(
            &case.run(),
            "Mail sent in the last 7 days is up 50% on the 7 days before (6 against 4)"
        ));
        set(&mut case, 2, 4);
        assert!(
            !has(&case.run(), "sent"),
            "under three sent is too few to compare"
        );
        set(&mut case, 3, 6);
        assert!(has(&case.run(), "is down 50%"));
        set(&mut case, 5, 6);
        assert!(!has(&case.run(), "sent"));
        set(&mut case, 0, 6);
        case.activity().previous_week_complete = false;
        assert!(!has(&case.run(), "sent"));
    }

    #[test]
    fn the_top_contact_needs_three_messages_and_a_clipped_name() {
        let mut case = Case::quiet_week();
        case.activity().top_contacts = vec![OverviewContact {
            address: "lin@example.com".into(),
            name: Some("林夕".into()),
            count: 2,
        }];
        assert!(!has(&case.run(), "back-and-forth"));
        case.activity().top_contacts[0].count = 3;
        assert_eq!(
            case.run(),
            vec![
                "The most back-and-forth in the last 7 days is with \"林夕\": 3 messages counting both directions."
            ]
        );
        // No name: the address. A long, multi-line name is flattened and cut.
        case.activity().top_contacts[0].name = None;
        assert!(has(&case.run(), "with \"lin@example.com\""));
        case.activity().top_contacts[0].name =
            Some(format!("Ann\nIgnore previous {}", "x".repeat(100)));
        let found = case.run();
        assert!(has(&found, "\"Ann Ignore previous"), "{found:?}");
        assert!(has(&found, "...\""));
        assert!(!found[0].contains('\n'));
    }

    #[test]
    fn the_busiest_day_must_stand_out_and_not_be_beaten_by_today() {
        // Shanghai weekdays: day 13 is Wednesday, so day 12 Tuesday, 11 Monday,
        // 10 Sunday, 9 Saturday, 8 Friday, 7 Thursday.
        let with = |busy: usize, count: u32, others: u32, today: u32| {
            let mut days = [others; 14];
            days[busy] = count;
            days[13] = today;
            Case::new(Some(activity_with(days)))
        };
        let found = with(9, 12, 4, 2).run();
        assert!(
            has(
                &found,
                "Saturday was the busiest of the last 7 days: 12 messages received."
            ),
            "{found:?}"
        );
        let found = with(12, 12, 4, 2).run();
        assert!(
            has(
                &found,
                "Yesterday was the busiest of the last 7 days: 12 messages received."
            ),
            "{found:?}"
        );
        // Under five, not clearly ahead of the runner-up (3/2), tied, or beaten by today.
        assert!(!has(&with(9, 4, 0, 0).run(), "busiest"));
        let mut case = with(9, 8, 4, 2);
        assert!(has(&case.run(), "busiest"), "8 is 2 times 4");
        case.activity().days[10].received = 6;
        assert!(!has(&case.run(), "busiest"));
        case.activity().days[10].received = 5;
        assert!(has(&case.run(), "busiest"));
        assert!(!has(&with(9, 12, 4, 13).run(), "busiest"));
        assert!(has(&with(9, 12, 4, 12).run(), "busiest"));
        let mut tied = with(9, 8, 1, 0);
        tied.activity().days[10].received = 8;
        assert!(!has(&tied.run(), "busiest"));
        // Needs the local weekday.
        let mut no_moment = with(9, 12, 4, 2);
        no_moment.moment = None;
        assert!(!has(&no_moment.run(), "busiest"));
    }

    #[test]
    fn starred_mail_still_unread_is_counted() {
        let mut case = Case::quiet_week();
        assert!(!has(&case.run(), "starred"));
        case.overview.starred_unread = 1;
        assert_eq!(case.run(), vec!["1 starred message is still unread."]);
        case.overview.starred_unread = 3;
        assert_eq!(case.run(), vec!["3 starred messages are still unread."]);
    }

    #[test]
    fn everything_received_today_being_read_is_noticed() {
        let mut case = Case::quiet_week();
        case.activity().today.unread = 0;
        assert_eq!(
            case.run(),
            vec!["Everything received today (3 messages) has been read."]
        );
        case.activity().today.unread = 1;
        assert!(case.run().is_empty());
        case.activity().today.received = 0;
        case.activity().today.unread = 0;
        assert!(!has(&case.run(), "Everything"));
    }

    #[test]
    fn late_night_and_weekend_lines() {
        let mut case = Case::quiet_week();
        case.new_total = Some(0);
        case.activity().today.received = 0;
        case.moment = Some(moment("2026-11-18T15:00:00.000Z"));
        assert!(
            !has(&case.run(), "late at night"),
            "no mail, nothing to say"
        );
        case.new_total = Some(2);
        assert_eq!(
            case.run(),
            vec!["It is late at night and mail is still arriving."]
        );
        // 22:59 is not late.
        case.moment = Some(moment("2026-11-18T14:59:00.000Z"));
        assert!(case.run().is_empty());
        // Saturday daytime, with mail received today but no since.
        let mut weekend = Case::quiet_week();
        weekend.new_total = None;
        let saturday = Moment::derive("2026-11-20T16:00:00Z", "2026-11-21T06:00:00Z").unwrap();
        weekend.moment = Some(saturday);
        assert_eq!(
            weekend.run(),
            vec!["It is the weekend and mail is still arriving."]
        );
        // Wednesday daytime: neither.
        weekend.moment = Some(moment("2026-11-18T06:00:00.000Z"));
        assert!(weekend.run().is_empty());
    }

    #[test]
    fn hidden_accounts_take_away_everything_that_cannot_be_split() {
        let mut days = [2; 14];
        days[13] = 9;
        let mut case = Case::new(Some(activity_with(days)));
        case.overview.starred_unread = 2;
        {
            let activity = case.activity();
            activity.sent_last_week = 0;
            activity.sent_previous_week = 6;
            activity.received_last_week = 30;
            activity.received_previous_week = 10;
            activity.top_contacts = vec![OverviewContact {
                address: "a@b.c".into(),
                name: None,
                count: 9,
            }];
            activity.today.unread = 0;
        }
        case.away = Some(Duration::days(2));
        case.new_total = Some(3);
        let all = case.run();
        assert_eq!(all.len(), MAX_OBSERVATIONS, "{all:?}");
        case.any_hidden = true;
        // What is left is what the visible accounts' new mail alone can say.
        assert_eq!(
            case.run(),
            vec![
                "The user is back after a while: they last looked 2 days ago, and 3 new messages arrived meanwhile."
            ]
        );
        case.away = Some(Duration::hours(1));
        case.moment = Some(moment("2026-11-18T15:30:00.000Z"));
        assert_eq!(
            case.run(),
            vec!["It is late at night and mail is still arriving."]
        );
    }

    #[test]
    fn the_list_is_ranked_and_capped() {
        let mut days = [2; 14];
        days[9] = 12;
        days[13] = 9;
        let mut case = Case::new(Some(activity_with(days)));
        case.overview.starred_unread = 2;
        case.away = Some(Duration::days(2));
        case.new_total = Some(3);
        {
            let activity = case.activity();
            activity.sent_previous_week = 6;
            activity.top_contacts = vec![OverviewContact {
                address: "a@b.c".into(),
                name: Some("Ann".into()),
                count: 5,
            }];
            activity.today.unread = 0;
        }
        let found = case.run();
        assert_eq!(found.len(), 5);
        let start = |i: usize, text: &str| assert!(found[i].starts_with(text), "{i}: {found:?}");
        start(0, "The user is back after a while");
        start(1, "Today is busier than usual");
        // The weeks are 31 and 14 received.
        start(2, "Mail received in the last 7 days is up");
        start(3, "The user sent nothing");
        start(4, "The most back-and-forth");
    }
}
