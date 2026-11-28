//! Phase 2 macOS EventKit direct-write integration for Apple Calendar
//! (write-only access on macOS 14+) and Apple Reminders. Non-macOS targets
//! return `Unsupported` so the UI cleanly stays on Phase 1 `.ics` / copy flows.
use mailx_core::{ActionItemExportInput, ActionItemKind, AppError};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeCalendarCapabilities {
    pub calendar_write: bool,
    pub reminders_write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) enum NativeAddOutcome {
    Added,
    PermissionDenied,
    Unavailable,
    Unsupported,
}

pub(crate) fn capabilities() -> NativeCalendarCapabilities {
    #[cfg(target_os = "macos")]
    {
        let supported = macos::is_macos_14_or_newer();
        NativeCalendarCapabilities {
            calendar_write: supported,
            reminders_write: supported,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        NativeCalendarCapabilities {
            calendar_write: false,
            reminders_write: false,
        }
    }
}

pub(crate) fn add_native(input: &ActionItemExportInput) -> Result<NativeAddOutcome, AppError> {
    // Re-validate through core's strict validator before touching EventKit.
    let _ = mailx_core::MailxCore::generate_action_item_ics(input)?;

    #[cfg(target_os = "macos")]
    {
        if !macos::is_macos_14_or_newer() {
            return Ok(NativeAddOutcome::Unsupported);
        }
        match input.kind {
            ActionItemKind::Event => macos::add_event(input),
            ActionItemKind::Task => macos::add_reminder(input),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(NativeAddOutcome::Unsupported)
    }
}

pub(crate) fn privacy_settings_url(kind: ActionItemKind) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        Some(match kind {
            ActionItemKind::Event => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars"
            }
            ActionItemKind::Task => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Reminders"
            }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        None
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use std::sync::mpsc;

    use block2::RcBlock;
    use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
    use mailx_core::{
        ActionItemExportInput, AppError,
        mail::action_items::{
            MAX_LOCATION_CHARS, MAX_NOTES_CHARS, MAX_TITLE_CHARS, clean_text, sanitize_http_url,
        },
    };
    use objc2::runtime::Bool;
    use objc2_event_kit::{
        EKAuthorizationStatus, EKEntityType, EKEvent, EKEventStore, EKReminder, EKSpan,
    };
    use objc2_foundation::{
        NSDate, NSDateComponents, NSError, NSOperatingSystemVersion, NSProcessInfo, NSString, NSURL,
    };

    use super::NativeAddOutcome;

    pub(super) fn is_macos_14_or_newer() -> bool {
        NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
            majorVersion: 14,
            minorVersion: 0,
            patchVersion: 0,
        })
    }

    fn request_event_write_access(store: &EKEventStore) -> bool {
        // SAFETY: Class method returning a POD authorization status enum.
        let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
        match status {
            EKAuthorizationStatus::FullAccess | EKAuthorizationStatus::WriteOnly => return true,
            EKAuthorizationStatus::Denied | EKAuthorizationStatus::Restricted => return false,
            EKAuthorizationStatus::NotDetermined => {}
            _ => return false,
        }
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        // SAFETY: `RcBlock` is heap-allocated and copied by EventKit for the
        // asynchronous completion callback; `tx` is Send and safe to signal once.
        unsafe {
            store.requestWriteOnlyAccessToEventsWithCompletion(&*block as *const _ as *mut _);
        }
        rx.recv_timeout(std::time::Duration::from_secs(60))
            .unwrap_or(false)
    }

    fn request_reminder_access(store: &EKEventStore) -> bool {
        // SAFETY: Class method returning a POD authorization status enum.
        let status =
            unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
        match status {
            EKAuthorizationStatus::FullAccess => return true,
            EKAuthorizationStatus::Denied | EKAuthorizationStatus::Restricted => return false,
            EKAuthorizationStatus::NotDetermined => {}
            _ => return false,
        }
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        // SAFETY: `RcBlock` is heap-allocated and copied by EventKit for the
        // asynchronous completion callback; `tx` is Send and safe to signal once.
        unsafe {
            store.requestFullAccessToRemindersWithCompletion(&*block as *const _ as *mut _);
        }
        rx.recv_timeout(std::time::Duration::from_secs(60))
            .unwrap_or(false)
    }

    fn parse_start_end_timestamps(input: &ActionItemExportInput) -> Option<(f64, f64)> {
        let start_raw = input.start_at.as_deref()?.trim();
        if input.all_day || start_raw.len() == 10 {
            let start_date = NaiveDate::parse_from_str(start_raw.get(..10)?, "%Y-%m-%d").ok()?;
            let end_date = input
                .end_at
                .as_deref()
                .and_then(|s| s.trim().get(..10))
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                .filter(|d| *d >= start_date)
                .unwrap_or(start_date);
            let start_dt = Local
                .from_local_datetime(&start_date.and_hms_opt(0, 0, 0)?)
                .earliest()?;
            let end_dt = Local
                .from_local_datetime(&end_date.and_hms_opt(23, 59, 0)?)
                .earliest()?;
            Some((start_dt.timestamp() as f64, end_dt.timestamp() as f64))
        } else {
            let start_dt = DateTime::parse_from_rfc3339(start_raw).ok()?;
            let end_dt = input
                .end_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
                .filter(|d| *d > start_dt)
                .unwrap_or_else(|| start_dt + Duration::hours(1));
            Some((start_dt.timestamp() as f64, end_dt.timestamp() as f64))
        }
    }

    pub(super) fn add_event(input: &ActionItemExportInput) -> Result<NativeAddOutcome, AppError> {
        let Some((start_secs, end_secs)) = parse_start_end_timestamps(input) else {
            return Err(AppError::validation("actionItem.startAt"));
        };
        // SAFETY: Standard EventKit store initialization and event creation on
        // validated Foundation strings/dates; store outlives `calendar` and `event`.
        unsafe {
            let store = EKEventStore::new();
            if !request_event_write_access(&store) {
                return Ok(NativeAddOutcome::PermissionDenied);
            }
            let Some(calendar) = store.defaultCalendarForNewEvents() else {
                return Ok(NativeAddOutcome::Unavailable);
            };
            let event = EKEvent::eventWithEventStore(&store);
            let title = clean_text(&input.title, MAX_TITLE_CHARS, false);
            event.setCalendar(Some(&calendar));
            event.setTitle(Some(&NSString::from_str(&title)));
            event.setAllDay(input.all_day);
            event.setStartDate(Some(&NSDate::dateWithTimeIntervalSince1970(start_secs)));
            event.setEndDate(Some(&NSDate::dateWithTimeIntervalSince1970(end_secs)));
            if let Some(loc) = input
                .location
                .as_deref()
                .map(|s| clean_text(s, MAX_LOCATION_CHARS, false))
                .filter(|s| !s.is_empty())
            {
                event.setLocation(Some(&NSString::from_str(&loc)));
            }
            if let Some(notes) = input
                .notes
                .as_deref()
                .map(|s| clean_text(s, MAX_NOTES_CHARS, true))
                .filter(|s| !s.is_empty())
            {
                event.setNotes(Some(&NSString::from_str(&notes)));
            }
            if let Some(url) = input.url.as_deref().and_then(sanitize_http_url)
                && let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(&url))
            {
                event.setURL(Some(&nsurl));
            }
            match store.saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true) {
                Ok(()) => Ok(NativeAddOutcome::Added),
                Err(_) => Ok(NativeAddOutcome::Unavailable),
            }
        }
    }

    pub(super) fn add_reminder(
        input: &ActionItemExportInput,
    ) -> Result<NativeAddOutcome, AppError> {
        // SAFETY: Standard EventKit store initialization and reminder creation on
        // validated Foundation strings/date components; store outlives `calendar` and `reminder`.
        unsafe {
            let store = EKEventStore::new();
            if !request_reminder_access(&store) {
                return Ok(NativeAddOutcome::PermissionDenied);
            }
            let Some(calendar) = store.defaultCalendarForNewReminders() else {
                return Ok(NativeAddOutcome::Unavailable);
            };
            let reminder = EKReminder::reminderWithEventStore(&store);
            let title = clean_text(&input.title, MAX_TITLE_CHARS, false);
            reminder.setCalendar(Some(&calendar));
            reminder.setTitle(Some(&NSString::from_str(&title)));
            if let Some(notes) = input
                .notes
                .as_deref()
                .map(|s| clean_text(s, MAX_NOTES_CHARS, true))
                .filter(|s| !s.is_empty())
            {
                reminder.setNotes(Some(&NSString::from_str(&notes)));
            }
            if let Some(url) = input.url.as_deref().and_then(sanitize_http_url)
                && let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(&url))
            {
                reminder.setURL(Some(&nsurl));
            }
            if let Some(start_raw) = input
                .start_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let comps = NSDateComponents::new();
                if input.all_day || start_raw.len() == 10 {
                    if let Some(date) = start_raw
                        .get(..10)
                        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                    {
                        comps.setYear(date.year() as isize);
                        comps.setMonth(date.month() as isize);
                        comps.setDay(date.day() as isize);
                        reminder.setDueDateComponents(Some(&comps));
                    }
                } else if let Ok(dt) = DateTime::parse_from_rfc3339(start_raw) {
                    let local = dt.with_timezone(&Local);
                    comps.setYear(local.year() as isize);
                    comps.setMonth(local.month() as isize);
                    comps.setDay(local.day() as isize);
                    comps.setHour(local.hour() as isize);
                    comps.setMinute(local.minute() as isize);
                    reminder.setDueDateComponents(Some(&comps));
                }
            }
            match store.saveReminder_commit_error(&reminder, true) {
                Ok(()) => Ok(NativeAddOutcome::Added),
                Err(_) => Ok(NativeAddOutcome::Unavailable),
            }
        }
    }
}
