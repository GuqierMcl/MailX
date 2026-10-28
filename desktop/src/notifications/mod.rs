//! New-mail notifications on the desktop: the system notification center plus,
//! for MailX's own sounds, a sound played here.
//!
//! What may be announced at all (switch, muted accounts, previews) is decided
//! by the core for every host (`NewMail::for_delivery`); this module only adds
//! the desktop's own rules: stay quiet while the window is in front unless the
//! user asked otherwise, and which sound goes with it.
mod sounds;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use mailx_core::{
    NewMail, NotificationSettings, NotificationSound, OutboxChanged, OutboxState, SupportedLocale,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::window_chrome::{self, MAIN_WINDOW};

/// Pushed to the main window when the user clicks a notification.
const OPEN_EVENT: &str = "mailx://notification-open";
/// Keys of the payload a notification carries back when it is clicked.
const ACCOUNT_KEY: &str = "mailxAccountId";
const EMAIL_KEY: &str = "mailxEmailId";
const OUTBOX_KEY: &str = "mailxOutboxId";

/// The sound name each notification center plays as its default.
#[cfg(target_os = "macos")]
const SYSTEM_SOUND: &str = "NSUserNotificationDefaultSoundName";
#[cfg(windows)]
const SYSTEM_SOUND: &str = "Default";
#[cfg(not(any(target_os = "macos", windows)))]
const SYSTEM_SOUND: &str = "message-new-email";

/// A delayed core event or notification lookup may outlive account removal.
/// The final delivery shares this lock with retirement, so it cannot display
/// a new notification after the account-removed event reaches the host.
#[derive(Default)]
pub(crate) struct AccountNotifications {
    removed: Mutex<HashSet<String>>,
}

impl AccountNotifications {
    pub(crate) fn retire(&self, account_id: &str) {
        self.removed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(account_id.to_owned());
    }

    fn deliver<T>(
        &self,
        account_id: &str,
        prepare: impl FnOnce() -> Option<T>,
        show: impl FnOnce(T),
    ) -> bool {
        if !self
            .removed
            .lock()
            .is_ok_and(|removed| !removed.contains(account_id))
        {
            return false;
        }
        // Settings and window lookups can block. Do not hold the retirement
        // lock while waiting for them; check again before the native side effect.
        let Some(prepared) = prepare() else {
            return false;
        };
        let Ok(removed) = self.removed.lock() else {
            return false;
        };
        if removed.contains(account_id) {
            return false;
        }
        show(prepared);
        drop(removed);
        true
    }
}

/// Everything needed to show one notification, decided without side effects.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shown {
    pub title: String,
    pub body: String,
    /// Sound the notification center plays; `None` shows it silently.
    pub system_sound: Option<&'static str>,
    /// Sound MailX plays itself (the notification is then silent).
    pub own_sound: Option<NotificationSound>,
    pub account_id: String,
    /// Set when the notification is about exactly one message.
    pub email_id: Option<String>,
    /// Set when the notification reports a send result.
    pub outbox_id: Option<String>,
}

/// (system sound, own sound) for the chosen sound setting.
fn sounds_for(sound: NotificationSound) -> (Option<&'static str>, Option<NotificationSound>) {
    match sound {
        NotificationSound::System => (Some(SYSTEM_SOUND), None),
        NotificationSound::None => (None, None),
        own => (None, Some(own)),
    }
}

/// Whether and how to announce `mail`, given the settings and whether the main
/// window is in front.
pub(crate) fn plan_localized(
    mail: &NewMail,
    settings: &NotificationSettings,
    window_in_front: bool,
    locale: SupportedLocale,
) -> Option<Shown> {
    let mail = mail.for_delivery(settings)?;
    if window_in_front && !settings.when_focused {
        return None;
    }
    let (title, body) = mail.summary_localized(locale);
    let (system_sound, own_sound) = sounds_for(settings.sound);
    Some(Shown {
        title,
        body,
        system_sound,
        own_sound,
        email_id: match mail.messages.as_slice() {
            [message] if mail.count == 1 => Some(message.email_id.clone()),
            _ => None,
        },
        account_id: mail.account_id,
        outbox_id: None,
    })
}

/// Whether a queue state is a final answer worth announcing.
pub(crate) fn announces_send_result(state: OutboxState) -> bool {
    matches!(
        state,
        OutboxState::Accepted | OutboxState::Failed | OutboxState::Unknown
    )
}

/// Whether and how to announce a send result. The subject is only included
/// when previews are on, like new mail; `enabled` and the in-front rule apply
/// the same way. Accepted results are silent; failures and unknown outcomes
/// use the chosen sound.
pub(crate) fn plan_send_result_localized(
    change: &OutboxChanged,
    subject: Option<&str>,
    settings: &NotificationSettings,
    window_in_front: bool,
    locale: SupportedLocale,
) -> Option<Shown> {
    let title = locale.send_result(change.state)?;
    if !settings.enabled || (window_in_front && !settings.when_focused) {
        return None;
    }
    let body = if settings.show_preview {
        match subject.map(str::trim) {
            Some(subject) if !subject.is_empty() => subject.to_owned(),
            Some(_) => locale.no_subject(),
            // The subject could not be read: say nothing rather than guess.
            None => String::new(),
        }
    } else {
        String::new()
    };
    let (system_sound, own_sound) = if change.state == OutboxState::Accepted {
        (None, None)
    } else {
        sounds_for(settings.sound)
    };
    Some(Shown {
        title,
        body,
        system_sound,
        own_sound,
        account_id: change.account_id.clone(),
        email_id: None,
        outbox_id: Some(change.outbox_id.clone()),
    })
}

pub(crate) fn show<R: Runtime>(app: &AppHandle<R>, shown: Shown) {
    let mut builder = app
        .notification()
        .builder()
        .title(shown.title)
        .extra(ACCOUNT_KEY, shown.account_id);
    if !shown.body.is_empty() {
        builder = builder.body(shown.body);
    }
    if let Some(email_id) = shown.email_id {
        builder = builder.extra(EMAIL_KEY, email_id);
    }
    if let Some(outbox_id) = shown.outbox_id {
        builder = builder.extra(OUTBOX_KEY, outbox_id);
    }
    if let Some(sound) = shown.system_sound {
        builder = builder.sound(sound);
    }
    if builder.show().is_ok()
        && let Some(sound) = shown.own_sound
    {
        sounds::play(sound);
    }
}

fn window_in_front<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(MAIN_WINDOW).is_some_and(|window| {
        window.is_focused().unwrap_or(false)
            && window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
    })
}

/// Handles a `NewMail` event from the core. Runs off the core's worker: asking
/// the window whether it is in front waits for the main thread.
pub(crate) fn new_mail<R: Runtime>(
    app: &AppHandle<R>,
    mail: NewMail,
    accounts: Arc<AccountNotifications>,
    settings: impl FnOnce() -> Option<(NotificationSettings, SupportedLocale)> + Send + 'static,
) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        accounts.deliver(
            &mail.account_id,
            || {
                let (settings, locale) = settings()?;
                plan_localized(&mail, &settings, window_in_front(&app), locale)
            },
            |shown| show(&app, shown),
        );
    });
}

/// Handles a send result from the core, off the core's worker for the same
/// reason as `new_mail`. `lookup` returns the notification settings and the
/// item's subject (read only to be shown when previews are on).
pub(crate) fn send_result<R: Runtime>(
    app: &AppHandle<R>,
    change: OutboxChanged,
    accounts: Arc<AccountNotifications>,
    lookup: impl FnOnce() -> Option<(NotificationSettings, Option<String>, SupportedLocale)>
    + Send
    + 'static,
) {
    if !announces_send_result(change.state) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        accounts.deliver(
            &change.account_id,
            || {
                let (settings, subject, locale) = lookup()?;
                plan_send_result_localized(
                    &change,
                    subject.as_deref(),
                    &settings,
                    window_in_front(&app),
                    locale,
                )
            },
            |shown| show(&app, shown),
        );
    });
}

/// A sample notification under the current settings, ignoring whether the
/// window is in front: the user is looking at the settings when asking.
pub(crate) fn test<R: Runtime>(
    app: &AppHandle<R>,
    settings: &NotificationSettings,
    locale: SupportedLocale,
) {
    let (system_sound, own_sound) = sounds_for(settings.sound);
    let (title, body) = if settings.show_preview {
        ("MailX".to_owned(), locale.test_notification())
    } else {
        ("MailX".to_owned(), locale.new_mail(1))
    };
    let mut builder = app.notification().builder().title(title).body(body);
    if let Some(sound) = system_sound {
        builder = builder.sound(sound);
    }
    if builder.show().is_ok()
        && let Some(sound) = own_sound
    {
        sounds::play(sound);
    }
}

/// Plays one of MailX's own sounds, for choosing between them.
pub(crate) fn preview(sound: NotificationSound) {
    sounds::play(sound);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenRequest {
    account_id: String,
    email_id: Option<String>,
    /// Set for send results: the queue item to show.
    outbox_id: Option<String>,
}

/// A click on a notification brings the window forward and tells the page
/// what to show. Clicks on notifications of an older launch report nothing.
pub(crate) fn on_click<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        let extra = performed.notification().map(|n| n.extra().clone());
        let text = |key: &str| {
            extra
                .as_ref()
                .and_then(|extra| extra.get(key))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };
        window_chrome::focus_main_window(&handle);
        let Some(account_id) = text(ACCOUNT_KEY) else {
            return;
        };
        let _ = handle.emit_to(
            EventTarget::webview_window(MAIN_WINDOW),
            OPEN_EVENT,
            OpenRequest {
                account_id,
                email_id: text(EMAIL_KEY),
                outbox_id: text(OUTBOX_KEY),
            },
        );
    });
}

#[cfg(test)]
fn plan(mail: &NewMail, settings: &NotificationSettings, window_in_front: bool) -> Option<Shown> {
    plan_localized(mail, settings, window_in_front, SupportedLocale::ZhCn)
}

#[cfg(test)]
fn plan_send_result(
    change: &OutboxChanged,
    subject: Option<&str>,
    settings: &NotificationSettings,
    window_in_front: bool,
) -> Option<Shown> {
    plan_send_result_localized(
        change,
        subject,
        settings,
        window_in_front,
        SupportedLocale::ZhCn,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailx_core::{MailAddress, NewMailMessage};

    #[test]
    fn removed_accounts_reject_late_notifications_and_in_flight_settings_lookups() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let accounts = Arc::new(AccountNotifications::default());
        let looked_up = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let shown = Arc::new(AtomicUsize::new(0));
        let worker = {
            let accounts = accounts.clone();
            let looked_up = looked_up.clone();
            let resume = resume.clone();
            let shown = shown.clone();
            std::thread::spawn(move || {
                accounts.deliver(
                    "removed",
                    || {
                        looked_up.wait();
                        resume.wait();
                        Some(())
                    },
                    |_| {
                        shown.fetch_add(1, Ordering::SeqCst);
                    },
                )
            })
        };
        looked_up.wait();
        accounts.retire("removed");
        resume.wait();
        assert!(!worker.join().unwrap());
        assert!(!accounts.deliver::<()>(
            "removed",
            || panic!("late events must not look up settings"),
            |_| panic!("removed account must stay quiet")
        ));
        assert!(accounts.deliver(
            "other",
            || Some(()),
            |_| {
                shown.fetch_add(1, Ordering::SeqCst);
            }
        ));
        assert_eq!(shown.load(Ordering::SeqCst), 1);
    }

    fn mail(count: u32, ids: &[&str]) -> NewMail {
        NewMail {
            account_id: "a1".into(),
            account_name: "工作".into(),
            count,
            messages: ids
                .iter()
                .map(|id| NewMailMessage {
                    email_id: (*id).into(),
                    from: MailAddress {
                        name: Some("Lin".into()),
                        address: "lin@x.io".into(),
                    },
                    subject: "周报".into(),
                    received_at: "2026-10-22T08:00:00.000Z".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_window_in_front_silences_notifications_unless_asked_for() {
        let settings = NotificationSettings::default();
        assert!(plan(&mail(1, &["e1"]), &settings, true).is_none());
        assert!(plan(&mail(1, &["e1"]), &settings, false).is_some());
        let always = NotificationSettings {
            when_focused: true,
            ..settings
        };
        assert!(plan(&mail(1, &["e1"]), &always, true).is_some());
    }

    #[test]
    fn english_text_preserves_preview_privacy_and_original_list_joining() {
        let settings = NotificationSettings::default();
        let private = NotificationSettings {
            show_preview: false,
            ..settings.clone()
        };
        let hidden =
            plan_localized(&mail(1, &["e1"]), &private, false, SupportedLocale::En).unwrap();
        assert_eq!(hidden.body, "1 new message");
        assert!(!format!("{hidden:?}").contains("周报"));
        let several = plan_localized(
            &mail(4, &["e1", "e2"]),
            &settings,
            false,
            SupportedLocale::En,
        )
        .unwrap();
        assert_eq!(several.body, "4 new messages from Lin and others");
        let failed = plan_send_result_localized(
            &change(OutboxState::Failed),
            Some(" "),
            &settings,
            false,
            SupportedLocale::En,
        )
        .unwrap();
        assert_eq!(failed.title, "Send failed");
        assert_eq!(failed.body, "(No subject)");
    }

    #[test]
    fn one_message_opens_itself_and_several_open_the_account() {
        let settings = NotificationSettings::default();
        let one = plan(&mail(1, &["e1"]), &settings, false).unwrap();
        assert_eq!(one.email_id.as_deref(), Some("e1"));
        assert_eq!((one.title.as_str(), one.body.as_str()), ("Lin", "周报"));
        let many = plan(&mail(4, &["e1", "e2", "e3"]), &settings, false).unwrap();
        assert_eq!(many.email_id, None);
        assert_eq!(many.account_id, "a1");
        // Without previews nothing points at a particular message either.
        let private = NotificationSettings {
            show_preview: false,
            ..NotificationSettings::default()
        };
        let hidden = plan(&mail(1, &["e1"]), &private, false).unwrap();
        assert_eq!(hidden.email_id, None);
        assert_eq!(hidden.body, "1 封新邮件");
    }

    fn change(state: OutboxState) -> OutboxChanged {
        OutboxChanged {
            account_id: "a1".into(),
            outbox_id: "o1".into(),
            draft_id: Some("d1".into()),
            state,
            attempts: 1,
            next_attempt_at: None,
            error_reason: None,
        }
    }

    #[test]
    fn only_final_send_results_are_announced() {
        let settings = NotificationSettings::default();
        for (state, title) in [
            (OutboxState::Accepted, "已发送"),
            (OutboxState::Failed, "发送失败"),
            (OutboxState::Unknown, "可能已发送，需要确认"),
        ] {
            assert!(announces_send_result(state));
            let shown = plan_send_result(&change(state), Some("周报"), &settings, false).unwrap();
            assert_eq!(shown.title, title);
            assert_eq!(shown.body, "周报");
            assert_eq!(shown.outbox_id.as_deref(), Some("o1"));
            assert_eq!(shown.email_id, None);
        }
        for state in [
            OutboxState::Queued,
            OutboxState::Sending,
            OutboxState::Canceled,
        ] {
            assert!(!announces_send_result(state));
            assert!(plan_send_result(&change(state), Some("x"), &settings, false).is_none());
        }
    }

    #[test]
    fn send_results_follow_the_preview_focus_and_enabled_settings() {
        let settings = NotificationSettings::default();
        let failed = change(OutboxState::Failed);
        assert!(plan_send_result(&failed, None, &settings, true).is_none());
        let always = NotificationSettings {
            when_focused: true,
            ..NotificationSettings::default()
        };
        assert!(plan_send_result(&failed, None, &always, true).is_some());
        let off = NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        };
        assert!(plan_send_result(&failed, None, &off, false).is_none());
        let private = NotificationSettings {
            show_preview: false,
            ..NotificationSettings::default()
        };
        let hidden = plan_send_result(&failed, Some("机密主题"), &private, false).unwrap();
        assert_eq!(hidden.body, "");
        assert!(!format!("{hidden:?}").contains("机密主题"));
        let blank = plan_send_result(&failed, Some("  "), &settings, false).unwrap();
        assert_eq!(blank.body, "（无主题）");
    }

    #[test]
    fn accepted_is_silent_but_failures_use_the_chosen_sound() {
        let settings = NotificationSettings::default();
        let accepted =
            plan_send_result(&change(OutboxState::Accepted), None, &settings, false).unwrap();
        assert_eq!((accepted.system_sound, accepted.own_sound), (None, None));
        let failed =
            plan_send_result(&change(OutboxState::Failed), None, &settings, false).unwrap();
        assert_eq!(
            (failed.system_sound, failed.own_sound),
            sounds_for(settings.sound)
        );
    }

    #[test]
    fn only_one_of_the_two_sounds_ever_plays() {
        let with = |sound| {
            plan(
                &mail(1, &["e1"]),
                &NotificationSettings {
                    sound,
                    ..NotificationSettings::default()
                },
                false,
            )
            .unwrap()
        };
        let system = with(NotificationSound::System);
        assert_eq!(
            (system.system_sound, system.own_sound),
            (Some(SYSTEM_SOUND), None)
        );
        let chime = with(NotificationSound::Chime);
        assert_eq!(
            (chime.system_sound, chime.own_sound),
            (None, Some(NotificationSound::Chime))
        );
        let quiet = with(NotificationSound::None);
        assert_eq!((quiet.system_sound, quiet.own_sound), (None, None));
    }
}
