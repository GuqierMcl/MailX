//! New-mail notifications on the desktop: the system notification center plus,
//! for MailX's own sounds, a sound played here.
//!
//! What may be announced at all (switch, muted accounts, previews) is decided
//! by the core for every host (`NewMail::for_delivery`); this module only adds
//! the desktop's own rules: stay quiet while the window is in front unless the
//! user asked otherwise, and which sound goes with it.
mod sounds;

use mailx_core::{NewMail, NotificationSettings, NotificationSound};
use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::window_chrome::{self, MAIN_WINDOW};

/// Pushed to the main window when the user clicks a notification.
const OPEN_EVENT: &str = "mailx://notification-open";
/// Keys of the payload a notification carries back when it is clicked.
const ACCOUNT_KEY: &str = "mailxAccountId";
const EMAIL_KEY: &str = "mailxEmailId";

/// The sound name each notification center plays as its default.
#[cfg(target_os = "macos")]
const SYSTEM_SOUND: &str = "NSUserNotificationDefaultSoundName";
#[cfg(windows)]
const SYSTEM_SOUND: &str = "Default";
#[cfg(not(any(target_os = "macos", windows)))]
const SYSTEM_SOUND: &str = "message-new-email";

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
}

/// Whether and how to announce `mail`, given the settings and whether the main
/// window is in front.
pub(crate) fn plan(
    mail: &NewMail,
    settings: &NotificationSettings,
    window_in_front: bool,
) -> Option<Shown> {
    let mail = mail.for_delivery(settings)?;
    if window_in_front && !settings.when_focused {
        return None;
    }
    let (title, body) = mail.summary();
    let (system_sound, own_sound) = match settings.sound {
        NotificationSound::System => (Some(SYSTEM_SOUND), None),
        NotificationSound::None => (None, None),
        own => (None, Some(own)),
    };
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
    })
}

pub(crate) fn show<R: Runtime>(app: &AppHandle<R>, shown: Shown) {
    let mut builder = app
        .notification()
        .builder()
        .title(shown.title)
        .body(shown.body)
        .extra(ACCOUNT_KEY, shown.account_id);
    if let Some(email_id) = shown.email_id {
        builder = builder.extra(EMAIL_KEY, email_id);
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
    settings: impl FnOnce() -> Option<NotificationSettings> + Send + 'static,
) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(settings) = settings() else {
            return;
        };
        if let Some(shown) = plan(&mail, &settings, window_in_front(&app)) {
            show(&app, shown);
        }
    });
}

/// A sample notification under the current settings, ignoring whether the
/// window is in front: the user is looking at the settings when asking.
pub(crate) fn test<R: Runtime>(app: &AppHandle<R>, settings: &NotificationSettings) {
    let (system_sound, own_sound) = match settings.sound {
        NotificationSound::System => (Some(SYSTEM_SOUND), None),
        NotificationSound::None => (None, None),
        own => (None, Some(own)),
    };
    let (title, body) = if settings.show_preview {
        (
            "MailX".to_owned(),
            "这是一条测试通知，新邮件会这样提醒你。".to_owned(),
        )
    } else {
        ("MailX".to_owned(), "1 封新邮件".to_owned())
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
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailx_core::{MailAddress, NewMailMessage};

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
