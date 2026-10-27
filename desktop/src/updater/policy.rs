//! Updater decisions that need no Tauri and no updater plugin: the state
//! machine, version comparison, manifest parsing and the check schedule.
//! A server edition can later reuse them as they are.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// The release manifest is a few kilobytes: a slow answer means a broken
/// network, not a slow link.
pub(crate) const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The installer is over 10 MB. `tauri-plugin-updater` applies one request
/// timeout to the whole download (`Update::download`, the `self.timeout`
/// request option), so this is the time allowed for the *entire* body, not
/// between chunks: on a slow link the check timeout would cut it off. The user
/// can cancel, so the ceiling can be generous.
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);

pub(crate) const PROGRESS_EMIT_GAP: Duration = Duration::from_millis(250);
/// The release notes come from a remote manifest: cap what reaches the UI.
pub(crate) const NOTES_LIMIT: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FailureStage {
    Check,
    Download,
}

/// What the frontend may know. Failures carry the stage only: remote and
/// system error text never reaches the UI or the logs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub(crate) enum UpdateState {
    Idle,
    Checking,
    #[serde(rename_all = "camelCase")]
    Available {
        version: String,
        current: String,
        notes: Option<String>,
        /// Publication time, seconds since the Unix epoch.
        date: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Ready {
        version: String,
    },
    Failed {
        stage: FailureStage,
        /// The version whose download failed, so the modal can still name it.
        version: Option<String>,
    },
}

impl UpdateState {
    /// A check may start from a quiet state, never while one is already in
    /// flight or after a version was found: that version stays the target.
    pub(crate) fn may_check(&self) -> bool {
        matches!(self, Self::Idle | Self::Failed { .. })
    }

    /// Whether a version has been found, so periodic checks are pointless.
    pub(crate) fn found_version(&self) -> bool {
        matches!(
            self,
            Self::Available { .. }
                | Self::Downloading { .. }
                | Self::Ready { .. }
                | Self::Failed {
                    stage: FailureStage::Download,
                    ..
                }
        )
    }

    pub(crate) fn may_download(&self) -> bool {
        matches!(
            self,
            Self::Available { .. }
                | Self::Failed {
                    stage: FailureStage::Download,
                    ..
                }
        )
    }

    pub(crate) fn may_cancel(&self) -> bool {
        matches!(self, Self::Downloading { .. })
    }

    pub(crate) fn may_restart(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoCheck {
    Run,
    /// Switched off or busy: look again at the next interval.
    Skip,
    /// A version is already known: the scheduler is done.
    Stop,
}

/// The delay before the next scheduler tick: a short one after start, then
/// the regular interval.
pub(crate) fn next_wait(first: bool) -> Duration {
    if first {
        FIRST_CHECK_DELAY
    } else {
        CHECK_INTERVAL
    }
}

pub(crate) fn auto_check(enabled: bool, state: &UpdateState) -> AutoCheck {
    if state.found_version() {
        AutoCheck::Stop
    } else if enabled && state.may_check() {
        AutoCheck::Run
    } else {
        AutoCheck::Skip
    }
}

/// Throttle for progress events: every new percent, or a time gap.
pub(crate) fn progress_due(
    last_emit: Option<Instant>,
    now: Instant,
    last_percent: Option<u64>,
    percent: Option<u64>,
) -> bool {
    match last_emit {
        None => true,
        Some(at) => percent != last_percent || now.duration_since(at) >= PROGRESS_EMIT_GAP,
    }
}

pub(crate) fn cap_notes(notes: Option<String>) -> Option<String> {
    let notes = notes?;
    let notes = notes.trim();
    if notes.is_empty() {
        return None;
    }
    Some(notes.chars().take(NOTES_LIMIT).collect())
}

/// What the frontend receives: the state plus what this edition can do with
/// it. A server edition reports `can_install: false` and only notifies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub(crate) can_install: bool,
    #[serde(flatten)]
    pub(crate) state: UpdateState,
}

/// The manifest parts below are what a notify-only edition builds on; the
/// desktop uses the plugin for fetching and only the comparison.
///
/// `MAJOR.MINOR.PATCH[-prerelease]`, ordered like semver (build metadata is
/// ignored). A leading `v` is tolerated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    core: [u64; 3],
    pre: Vec<PreIdent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PreIdent {
    Number(u64),
    Text(String),
}

impl Version {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = core.split('.');
        let mut parts = [0_u64; 3];
        for part in &mut parts {
            *part = numbers.next()?.parse().ok()?;
        }
        if numbers.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|ident| match ident.parse::<u64>() {
                    Ok(number) => Some(PreIdent::Number(number)),
                    Err(_) if !ident.is_empty() => Some(PreIdent::Text(ident.to_string())),
                    Err(_) => None,
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Self { core: parts, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release outranks its own pre-releases.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreIdent {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
        }
    }
}

/// Whether `candidate` is a newer version than `current`. Unparseable input
/// is never newer.
pub(crate) fn is_newer(current: &str, candidate: &str) -> bool {
    match (Version::parse(current), Version::parse(candidate)) {
        (Some(current), Some(candidate)) => candidate > current,
        _ => false,
    }
}

/// The part of `latest.json` every edition reads. `platforms` is not here:
/// a notify-only edition has no use for it and must not require it.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Manifest {
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) notes: Option<String>,
    #[serde(default)]
    pub(crate) pub_date: Option<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_manifest(text: &str) -> Option<Manifest> {
    let manifest: Manifest = serde_json::from_str(text).ok()?;
    Version::parse(&manifest.version)?;
    Some(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> UpdateState {
        UpdateState::Available {
            version: "0.2.0".into(),
            current: "0.1.0".into(),
            notes: None,
            date: None,
        }
    }

    fn downloading() -> UpdateState {
        UpdateState::Downloading {
            version: "0.2.0".into(),
            downloaded: 0,
            total: None,
        }
    }

    fn ready() -> UpdateState {
        UpdateState::Ready {
            version: "0.2.0".into(),
        }
    }

    fn failed(stage: FailureStage) -> UpdateState {
        UpdateState::Failed {
            stage,
            version: None,
        }
    }

    #[test]
    fn the_download_gets_much_longer_than_the_check() {
        // A 30 s ceiling would abort a 15 MB bundle on a slow link; the check
        // stays short so a broken network fails quietly and quickly.
        assert_eq!(CHECK_TIMEOUT, Duration::from_secs(30));
        assert!(DOWNLOAD_TIMEOUT >= CHECK_TIMEOUT * 10);
    }

    #[test]
    fn the_scheduler_waits_thirty_seconds_then_six_hours() {
        assert_eq!(next_wait(true), Duration::from_secs(30));
        assert_eq!(next_wait(false), Duration::from_secs(6 * 3600));
    }

    #[test]
    fn auto_checks_run_only_when_enabled_and_quiet() {
        assert_eq!(auto_check(true, &UpdateState::Idle), AutoCheck::Run);
        assert_eq!(
            auto_check(true, &failed(FailureStage::Check)),
            AutoCheck::Run
        );
        assert_eq!(auto_check(false, &UpdateState::Idle), AutoCheck::Skip);
        assert_eq!(auto_check(true, &UpdateState::Checking), AutoCheck::Skip);
    }

    #[test]
    fn the_scheduler_stops_once_a_version_is_known() {
        for state in [
            available(),
            downloading(),
            ready(),
            failed(FailureStage::Download),
        ] {
            assert_eq!(auto_check(true, &state), AutoCheck::Stop, "{state:?}");
            assert_eq!(auto_check(false, &state), AutoCheck::Stop, "{state:?}");
        }
    }

    #[test]
    fn transitions_follow_the_state_machine() {
        let all = [
            UpdateState::Idle,
            UpdateState::Checking,
            available(),
            downloading(),
            ready(),
            failed(FailureStage::Check),
            failed(FailureStage::Download),
        ];
        let allowed =
            |test: fn(&UpdateState) -> bool| -> Vec<bool> { all.iter().map(test).collect() };
        assert_eq!(
            allowed(UpdateState::may_check),
            [true, false, false, false, false, true, true]
        );
        assert_eq!(
            allowed(UpdateState::may_download),
            [false, false, true, false, false, false, true]
        );
        assert_eq!(
            allowed(UpdateState::may_cancel),
            [false, false, false, true, false, false, false]
        );
        assert_eq!(
            allowed(UpdateState::may_restart),
            [false, false, false, false, true, false, false]
        );
    }

    #[test]
    fn progress_events_are_throttled() {
        let start = Instant::now();
        assert!(progress_due(None, start, None, Some(0)));
        let soon = start + Duration::from_millis(10);
        assert!(!progress_due(Some(start), soon, Some(3), Some(3)));
        assert!(progress_due(Some(start), soon, Some(3), Some(4)));
        let later = start + PROGRESS_EMIT_GAP;
        assert!(progress_due(Some(start), later, None, None));
    }

    #[test]
    fn notes_are_trimmed_and_capped() {
        assert_eq!(cap_notes(None), None);
        assert_eq!(cap_notes(Some("  \n ".into())), None);
        assert_eq!(cap_notes(Some(" hi ".into())).as_deref(), Some("hi"));
        let long = "字".repeat(NOTES_LIMIT + 10);
        assert_eq!(cap_notes(Some(long)).unwrap().chars().count(), NOTES_LIMIT);
    }

    #[test]
    fn states_serialize_for_the_frontend() {
        let json = serde_json::to_value(available()).unwrap();
        assert_eq!(json["phase"], "available");
        assert_eq!(json["current"], "0.1.0");
        let json = serde_json::to_value(failed(FailureStage::Download)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"phase": "failed", "stage": "download", "version": null})
        );
        assert_eq!(
            serde_json::to_value(UpdateState::Idle).unwrap(),
            serde_json::json!({"phase": "idle"})
        );
    }

    #[test]
    fn snapshots_carry_the_capability_flat() {
        let json = serde_json::to_value(Snapshot {
            can_install: false,
            state: available(),
        })
        .unwrap();
        assert_eq!(json["canInstall"], false);
        assert_eq!(json["phase"], "available");
        assert_eq!(json["version"], "0.2.0");
    }

    #[test]
    fn versions_order_like_semver() {
        let ordered = [
            "0.1.0-alpha.2",
            "0.1.0-alpha.10",
            "0.1.0-beta",
            "0.1.0",
            "0.1.1",
            "0.2.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(is_newer(pair[0], pair[1]), "{pair:?}");
            assert!(!is_newer(pair[1], pair[0]), "{pair:?}");
        }
        assert!(!is_newer("0.1.0", "v0.1.0+build5"));
        assert!(is_newer("v0.1.0", "0.1.1"));
    }

    #[test]
    fn unparseable_versions_are_never_newer() {
        for bad in ["", "1", "1.2", "1.2.3.4", "a.b.c", "1.2.3-", "1.2.3-a..b"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
            assert!(!is_newer("0.1.0", bad), "{bad}");
            assert!(!is_newer(bad, "0.1.0"), "{bad}");
        }
    }

    #[test]
    fn manifests_parse_without_platforms() {
        let text = r#"{"version":"0.2.0","notes":"- fixed","pub_date":"2026-01-02T03:04:05Z"}"#;
        let manifest = parse_manifest(text).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.notes.as_deref(), Some("- fixed"));
        assert_eq!(manifest.pub_date.as_deref(), Some("2026-01-02T03:04:05Z"));
        let bare = parse_manifest(r#"{"version":"0.2.0"}"#).unwrap();
        assert_eq!((bare.notes, bare.pub_date), (None, None));
        let full =
            r#"{"version":"0.2.0","platforms":{"darwin-aarch64":{"url":"u","signature":"s"}}}"#;
        assert!(parse_manifest(full).is_some());
    }

    #[test]
    fn invalid_manifests_are_rejected() {
        for bad in ["", "{}", "[]", r#"{"version":"soon"}"#, r#"{"version":3}"#] {
            assert!(parse_manifest(bad).is_none(), "{bad}");
        }
    }
}
