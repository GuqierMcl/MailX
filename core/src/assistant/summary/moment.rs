//! The user's local moment for the Now summary: the time of day and weekday
//! where they are, how long ago they last looked, and what today is.
//!
//! The core has no time zone. The host sends the start of each local day, so
//! the UTC offset can be read back from the last one: local midnight, as a UTC
//! instant, is `00:00 - offset`.
use chrono::{DateTime, Datelike, Duration, FixedOffset, Timelike, Utc, Weekday};

use crate::Occasion;

/// The largest UTC offset east of Greenwich that is read as east. A midnight
/// that far east and one a little west of it look the same (+13 and -11, +14
/// and -10), so the more populated zone wins: from +13 down to the far
/// western ones. The rare +13:45 and +14 zones are read as west.
const MAX_EAST_SECONDS: i64 = 13 * 3600;
const DAY_SECONDS: i64 = 24 * 3600;

/// Parts of the day, with the same hour boundaries as the greeting on the page
/// (`daySegment` in `frontend/src/features/now/greeting.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PartOfDay {
    EarlyMorning,
    Morning,
    Noon,
    Afternoon,
    Evening,
    Night,
}

impl PartOfDay {
    pub(super) fn of_hour(hour: u32) -> Self {
        match hour {
            5..=7 => Self::EarlyMorning,
            8..=10 => Self::Morning,
            11..=12 => Self::Noon,
            13..=16 => Self::Afternoon,
            17..=20 => Self::Evening,
            _ => Self::Night,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::EarlyMorning => "early morning",
            Self::Morning => "morning",
            Self::Noon => "around noon",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Night => "night",
        }
    }
}

pub(super) fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// Where the user is in their day, as far as the day starts tell.
#[derive(Clone, Copy, Debug)]
pub(super) struct Moment {
    offset: FixedOffset,
    local: DateTime<FixedOffset>,
}

impl Moment {
    /// `last_day_start` is the start of the user's local today and `now` the
    /// core's clock (both RFC 3339). Nothing when either does not parse.
    ///
    /// The offset is the one at the start of today; on the day a clock change
    /// happens, the hours after it are off by that change.
    pub(super) fn derive(last_day_start: &str, now: &str) -> Option<Self> {
        let start = DateTime::parse_from_rfc3339(last_day_start)
            .ok()?
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339(now).ok()?.with_timezone(&Utc);
        let time_of_day = i64::from(start.time().num_seconds_from_midnight());
        let mut east = (DAY_SECONDS - time_of_day) % DAY_SECONDS;
        if east > MAX_EAST_SECONDS {
            east -= DAY_SECONDS;
        }
        let offset = FixedOffset::east_opt(i32::try_from(east).ok()?)?;
        Some(Self {
            offset,
            local: now.with_timezone(&offset),
        })
    }

    pub(super) fn hour(&self) -> u32 {
        self.local.hour()
    }

    pub(super) fn weekday(&self) -> Weekday {
        self.local.weekday()
    }

    pub(super) fn part(&self) -> PartOfDay {
        PartOfDay::of_hour(self.hour())
    }

    pub(super) fn is_weekend(&self) -> bool {
        matches!(self.weekday(), Weekday::Sat | Weekday::Sun)
    }

    /// Late at night: 23:00 up to 05:00.
    pub(super) fn is_late_night(&self) -> bool {
        !(5..23).contains(&self.hour())
    }

    /// The local weekday of a day start (RFC 3339). Noon of that day is used,
    /// so a clock change on that day does not move it.
    pub(super) fn weekday_of_day(&self, day_start: &str) -> Option<Weekday> {
        let start = DateTime::parse_from_rfc3339(day_start).ok()?;
        Some(
            (start.with_timezone(&self.offset) + Duration::hours(12))
                .date_naive()
                .weekday(),
        )
    }

    /// For the facts. The hour only, not the minute: the line is cached by its
    /// facts, and a time that changed every minute would never hit.
    pub(super) fn fact(&self) -> String {
        format!(
            "Local time: {} {}, around {:02}:00",
            weekday_name(self.weekday()),
            self.part().text(),
            self.hour(),
        )
    }
}

/// How long ago, coarsely. Under ten minutes is "a few minutes"; then tens of
/// minutes, hours and days. Coarse on purpose: see [`Moment::fact`].
pub(super) fn ago(gap: Duration) -> String {
    let minutes = gap.num_minutes().max(0);
    if minutes < 10 {
        return "a few minutes ago".into();
    }
    if minutes < 60 {
        let tens = (minutes + 5) / 10 * 10;
        return if tens >= 60 {
            "about an hour ago".into()
        } else {
            format!("about {tens} minutes ago")
        };
    }
    let hours = (minutes + 30) / 60;
    if hours < 24 {
        return if hours == 1 {
            "about an hour ago".into()
        } else {
            format!("about {hours} hours ago")
        };
    }
    let days = (hours + 12) / 24;
    if days == 1 {
        "1 day ago".into()
    } else {
        format!("{days} days ago")
    }
}

/// The fact line for what today is: the name in English and, for the
/// Chinese ones, as written in Chinese.
pub(super) fn occasion_fact(occasion: Occasion) -> String {
    use Occasion::*;
    let holiday = |english: &str, chinese: &str| format!("Today is {english} ({chinese})");
    let term =
        |chinese: &str, english: &str| format!("Today is the solar term {chinese} ({english})");
    match occasion {
        NewYear => holiday("New Year's Day", "元旦"),
        SpringFestival => holiday("the Spring Festival, Chinese New Year", "春节"),
        Lantern => holiday("the Lantern Festival", "元宵节"),
        DragonBoat => holiday("the Dragon Boat Festival", "端午节"),
        Qixi => holiday("the Qixi Festival", "七夕"),
        MidAutumn => holiday("the Mid-Autumn Festival", "中秋节"),
        Xiaohan => term("小寒", "Minor Cold"),
        Dahan => term("大寒", "Major Cold"),
        Lichun => term("立春", "Start of Spring"),
        Yushui => term("雨水", "Rain Water"),
        Jingzhe => term("惊蛰", "Awakening of Insects"),
        Chunfen => term("春分", "Spring Equinox"),
        Qingming => term("清明", "Pure Brightness"),
        Guyu => term("谷雨", "Grain Rain"),
        Lixia => term("立夏", "Start of Summer"),
        Xiaoman => term("小满", "Grain Buds"),
        Mangzhong => term("芒种", "Grain in Ear"),
        Xiazhi => term("夏至", "Summer Solstice"),
        Xiaoshu => term("小暑", "Minor Heat"),
        Dashu => term("大暑", "Major Heat"),
        Liqiu => term("立秋", "Start of Autumn"),
        Chushu => term("处暑", "End of Heat"),
        Bailu => term("白露", "White Dew"),
        Qiufen => term("秋分", "Autumn Equinox"),
        Hanlu => term("寒露", "Cold Dew"),
        Shuangjiang => term("霜降", "Frost's Descent"),
        Lidong => term("立冬", "Start of Winter"),
        Xiaoxue => term("小雪", "Minor Snow"),
        Daxue => term("大雪", "Major Snow"),
        Dongzhi => term("冬至", "Winter Solstice"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn east_of_greenwich_reads_the_offset_and_weekday_from_local_midnight() {
        // Asia/Shanghai: Wednesday 2026-11-18 00:00 local is Tuesday 16:00 UTC.
        // 21:40 local is 13:40 UTC the same Wednesday.
        let moment =
            Moment::derive("2026-11-17T16:00:00.000Z", "2026-11-18T13:40:00.000Z").expect("moment");
        assert_eq!(moment.offset.local_minus_utc(), 8 * 3600);
        assert_eq!(moment.hour(), 21);
        assert_eq!(moment.weekday(), Weekday::Wed);
        assert_eq!(moment.part(), PartOfDay::Night);
        assert_eq!(moment.fact(), "Local time: Wednesday night, around 21:00");
        // The weekday comes from the local date, not the UTC one: 01:30 local
        // is still Tuesday 17:30 UTC.
        let early =
            Moment::derive("2026-11-17T16:00:00.000Z", "2026-11-17T17:30:00.000Z").expect("moment");
        assert_eq!(early.weekday(), Weekday::Wed);
        assert_eq!(early.hour(), 1);
        assert!(early.is_late_night());
    }

    #[test]
    fn west_of_greenwich_reads_a_negative_offset() {
        // America/Los_Angeles in November (UTC-8): Wednesday 00:00 local is
        // Wednesday 08:00 UTC; 21:40 local is Thursday 05:40 UTC.
        let moment =
            Moment::derive("2026-11-18T08:00:00.000Z", "2026-11-19T05:40:00.000Z").expect("moment");
        assert_eq!(moment.offset.local_minus_utc(), -8 * 3600);
        assert_eq!(moment.hour(), 21);
        assert_eq!(moment.weekday(), Weekday::Wed);
        // Summer time (UTC-7), early afternoon.
        let summer =
            Moment::derive("2026-07-15T07:00:00.000Z", "2026-07-15T20:05:00.000Z").expect("moment");
        assert_eq!(summer.offset.local_minus_utc(), -7 * 3600);
        assert_eq!(summer.hour(), 13);
        assert_eq!(summer.part(), PartOfDay::Afternoon);
        assert_eq!(summer.weekday(), Weekday::Wed);
    }

    #[test]
    fn zones_on_the_hour_and_the_half_hour_are_read() {
        let utc = Moment::derive("2026-11-18T00:00:00Z", "2026-11-18T09:00:00Z").unwrap();
        assert_eq!(utc.offset.local_minus_utc(), 0);
        let india = Moment::derive("2026-11-17T18:30:00Z", "2026-11-18T05:00:00Z").unwrap();
        assert_eq!(india.offset.local_minus_utc(), 5 * 3600 + 1800);
        assert_eq!(india.hour(), 10);
        // Auckland summer time, +13, reads east; Hawaii, -10, west.
        let auckland = Moment::derive("2026-12-17T11:00:00Z", "2026-12-17T12:00:00Z").unwrap();
        assert_eq!(auckland.offset.local_minus_utc(), 13 * 3600);
        let hawaii = Moment::derive("2026-11-18T10:00:00Z", "2026-11-18T12:00:00Z").unwrap();
        assert_eq!(hawaii.offset.local_minus_utc(), -10 * 3600);
    }

    #[test]
    fn day_starts_that_do_not_parse_give_no_moment() {
        assert!(Moment::derive("soon", "2026-11-18T13:40:00Z").is_none());
        assert!(Moment::derive("2026-11-17T16:00:00Z", "later").is_none());
    }

    #[test]
    fn the_weekday_of_a_day_start_is_the_local_one() {
        let moment =
            Moment::derive("2026-11-17T16:00:00.000Z", "2026-11-18T13:40:00.000Z").expect("moment");
        // Shanghai: Tuesday 16:00 UTC is Wednesday local; the day before is Tuesday.
        assert_eq!(
            moment.weekday_of_day("2026-11-17T16:00:00.000Z"),
            Some(Weekday::Wed)
        );
        assert_eq!(
            moment.weekday_of_day("2026-11-16T16:00:00.000Z"),
            Some(Weekday::Tue)
        );
        // A start an hour off (a clock change) is still the same local day.
        assert_eq!(
            moment.weekday_of_day("2026-11-15T15:00:00.000Z"),
            Some(Weekday::Mon)
        );
    }

    #[test]
    fn parts_of_the_day_follow_the_greeting_segments() {
        use PartOfDay::*;
        for (hour, part) in [
            (0, Night),
            (4, Night),
            (5, EarlyMorning),
            (7, EarlyMorning),
            (8, Morning),
            (10, Morning),
            (11, Noon),
            (12, Noon),
            (13, Afternoon),
            (16, Afternoon),
            (17, Evening),
            (20, Evening),
            (21, Night),
            (23, Night),
        ] {
            assert_eq!(PartOfDay::of_hour(hour), part, "{hour}");
        }
    }

    #[test]
    fn late_night_and_weekend_edges() {
        let at = |now: &str| Moment::derive("2026-11-21T00:00:00Z", now).unwrap();
        assert!(!at("2026-11-21T22:59:00Z").is_late_night());
        assert!(at("2026-11-21T23:00:00Z").is_late_night());
        assert!(at("2026-11-21T04:59:00Z").is_late_night());
        assert!(!at("2026-11-21T05:00:00Z").is_late_night());
        // 2026-11-21 is a Saturday.
        assert!(at("2026-11-21T10:00:00Z").is_weekend());
        assert!(
            !Moment::derive("2026-11-18T00:00:00Z", "2026-11-18T10:00:00Z")
                .unwrap()
                .is_weekend()
        );
    }

    #[test]
    fn how_long_ago_is_coarse() {
        let minutes = |n| ago(Duration::minutes(n));
        assert_eq!(minutes(-5), "a few minutes ago");
        assert_eq!(minutes(9), "a few minutes ago");
        assert_eq!(minutes(10), "about 10 minutes ago");
        assert_eq!(minutes(42), "about 40 minutes ago");
        assert_eq!(minutes(56), "about an hour ago");
        assert_eq!(minutes(60), "about an hour ago");
        assert_eq!(minutes(130), "about 2 hours ago");
        assert_eq!(minutes(23 * 60), "about 23 hours ago");
        assert_eq!(minutes(24 * 60), "1 day ago");
        assert_eq!(minutes(3 * 24 * 60 + 300), "3 days ago");
    }

    #[test]
    fn an_occasion_reads_in_both_languages() {
        assert_eq!(
            occasion_fact(Occasion::Lidong),
            "Today is the solar term 立冬 (Start of Winter)"
        );
        assert_eq!(
            occasion_fact(Occasion::MidAutumn),
            "Today is the Mid-Autumn Festival (中秋节)"
        );
    }
}
