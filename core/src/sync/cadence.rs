//! How long a healthy account waits before its next sync round.
//!
//! A fixed interval is too slow while someone is reading and wasteful while
//! nobody is. The host tells the core whether the app is in front ([`Activity`]);
//! this module turns that, and whether the last round found anything, into a
//! wait. There is deliberately no "asleep" or "locked" state: a locked screen is
//! exactly when a new-mail notification is wanted, so it counts as background,
//! and a computer that is really asleep does not run timers anyway. It only decides durations: no clocks, no network, so every rule is a
//! plain test. Failures are not its business: they keep their own exponential
//! backoff in the worker.
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Wait between rounds while the app is in front.
pub(crate) const ACTIVE_INTERVAL: Duration = Duration::from_secs(20);
/// First wait between rounds while the app is in the background.
pub(crate) const BACKGROUND_BASE: Duration = Duration::from_secs(3 * 60);
/// The background wait never grows past this, however quiet the mailbox is.
pub(crate) const BACKGROUND_CAP: Duration = Duration::from_secs(10 * 60);
/// Coming to the front starts a round at once, but never sooner than this after
/// the previous one ended: every round logs in again, and flicking between
/// windows must not turn into a stream of logins.
pub(crate) const FRONT_WAKE_MIN_GAP: Duration = Duration::from_secs(10);
/// Quiet rounds in a row before the background wait starts to grow.
const QUIET_ROUNDS_BEFORE_GROWTH: u32 = 2;

/// How the app is being used right now, as told by the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    /// The window is in front: someone is looking at the mail.
    #[default]
    Active,
    /// The app is running but out of sight, including behind a locked screen.
    Background,
}

/// Per-account scheduling state. One value lives in each account's worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cadence {
    /// Rounds in a row that found nothing new.
    quiet_rounds: u32,
}

impl Cadence {
    /// Records how the last successful round went.
    pub(crate) fn record(&mut self, changed: bool) {
        self.quiet_rounds = if changed {
            0
        } else {
            self.quiet_rounds.saturating_add(1)
        };
    }

    /// Anyone looking at the app wants the newest state and a short wait: the
    /// quiet streak of the background does not carry over to the front.
    pub(crate) fn reset(&mut self) {
        self.quiet_rounds = 0;
    }

    /// Wait before the next round.
    pub(crate) fn wait(&self, activity: Activity) -> Duration {
        match activity {
            Activity::Active => ACTIVE_INTERVAL,
            Activity::Background => {
                // Doubling per quiet round beyond the first few, up to the cap.
                let growth = self.quiet_rounds.saturating_sub(QUIET_ROUNDS_BEFORE_GROWTH);
                let factor = 1u32 << growth.min(8);
                BACKGROUND_BASE.saturating_mul(factor).min(BACKGROUND_CAP)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(rounds: u32) -> Cadence {
        let mut cadence = Cadence::default();
        for _ in 0..rounds {
            cadence.record(false);
        }
        cadence
    }

    #[test]
    fn in_front_the_wait_is_short_and_ignores_how_quiet_it_has_been() {
        for rounds in [0, 1, 5, 50, u32::MAX] {
            assert_eq!(quiet(rounds).wait(Activity::Active), ACTIVE_INTERVAL);
        }
    }

    #[test]
    fn in_the_background_it_starts_slow_then_backs_off_in_steps_up_to_the_cap() {
        let waits: Vec<u64> = (0..8)
            .map(|rounds| quiet(rounds).wait(Activity::Background).as_secs())
            .collect();
        // The first quiet rounds keep the base wait; then it doubles, never past ten minutes.
        assert_eq!(waits, [180, 180, 180, 360, 600, 600, 600, 600]);
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(quiet(u32::MAX).wait(Activity::Background), BACKGROUND_CAP);
    }

    #[test]
    fn a_round_that_finds_something_starts_the_backing_off_over() {
        let mut cadence = quiet(10);
        assert_eq!(cadence.wait(Activity::Background), BACKGROUND_CAP);
        cadence.record(true);
        assert_eq!(cadence.wait(Activity::Background), BACKGROUND_BASE);
    }

    #[test]
    fn coming_to_the_front_resets_the_streak_so_going_back_starts_fresh() {
        let mut cadence = quiet(10);
        cadence.reset();
        assert_eq!(cadence.wait(Activity::Background), BACKGROUND_BASE);
    }

    #[test]
    fn the_wait_is_never_shorter_in_the_background_than_in_front() {
        for rounds in 0..20 {
            let background = quiet(rounds).wait(Activity::Background);
            assert!(background > ACTIVE_INTERVAL);
        }
    }

    #[test]
    fn activity_serializes_as_camel_case_words_for_hosts() {
        let words: Vec<String> = [Activity::Active, Activity::Background]
            .iter()
            .map(|activity| serde_json::to_string(activity).unwrap())
            .collect();
        assert_eq!(words, ["\"active\"", "\"background\""]);
    }
}
