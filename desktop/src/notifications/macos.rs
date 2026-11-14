//! Actionable macOS notifications through the existing native plugin.
//!
//! A single action remains a button instead of the old backend's multi-action
//! Options menu. Its native delegate keeps observing a delivered notification
//! while it remains in Notification Center and this process is alive. Native
//! presentation and visibility still follow the user's notification settings.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{Action, ActionType, NotificationExt};

use super::{CodeAction, CodeActionHandler, CodeNotification, CodeNotificationKind};

/// The ordinary notification tap handler must ignore notifications with this
/// key: the dedicated handler below owns both their copy and open actions.
pub(super) const TOKEN_KEY: &str = "mailxVerificationToken";
const COPY_ACTION: &str = "mailx-copy-verification-code";
const UNDO_ACTION: &str = "mailx-undo-verification-code-deletion";

struct Registered;

/// Register before showing any code notification. The handler is native Rust,
/// so hiding the WebView does not suspend copying. Copy actions never call the
/// application's focus/open path; the parent routes the two actions separately.
pub(super) fn setup(app: &AppHandle, handler: CodeActionHandler) -> Result<(), ()> {
    if app.try_state::<Registered>().is_some() {
        return Err(());
    }
    app.notification()
        .on_action(move |performed| {
            let token = performed
                .notification()
                .and_then(|notification| notification.extra().get(TOKEN_KEY))
                .and_then(|value| value.as_str());
            if let Some(action) = action(performed.action_id(), token) {
                handler(action);
            }
        })
        .map_err(|_| ())?;
    app.manage(Registered);
    Ok(())
}

/// Queue one native notification. As with the plugin's ordinary notifications,
/// success means the request was prepared; macOS can still suppress delivery.
/// The code appears only in the displayed text. The callback payload contains
/// an opaque token that the parent resolves and revalidates before copying.
pub(super) fn show(app: &AppHandle, notification: CodeNotification) -> Result<(), ()> {
    if app.try_state::<Registered>().is_none() || notification.expires_in.is_zero() {
        return Err(());
    }
    // Labels come from the app's translation catalog. Distinct category keys
    // keep concurrent notifications in different languages from replacing each
    // other's button title between registration and builder.show().
    let action_id = button_action(&notification.kind);
    let category = format!("{action_id}:{}", notification.copy_label);
    app.notification()
        .register_action_types(vec![
            ActionType::builder(&category)
                .actions(vec![
                    Action::builder(action_id, notification.copy_label).build(),
                ])
                .build(),
        ])
        .map_err(|_| ())?;
    let mut builder = app
        .notification()
        .builder()
        .title(notification.title)
        .body(notification.body)
        .action_type_id(category)
        .extra(TOKEN_KEY, notification.token);
    if let Some(sound) = notification.sound {
        builder = builder.sound(sound);
    }
    builder.show().map_err(|_| ())
}

fn button_action(kind: &CodeNotificationKind) -> &'static str {
    match kind {
        CodeNotificationKind::Copy => COPY_ACTION,
        CodeNotificationKind::Undo => UNDO_ACTION,
    }
}

fn action(identifier: &str, token: Option<&str>) -> Option<CodeAction> {
    let token = token.filter(|token| !token.is_empty())?;
    match identifier {
        COPY_ACTION => Some(CodeAction::Copy(token.to_owned())),
        UNDO_ACTION => Some(CodeAction::Undo(token.to_owned())),
        "tap" => Some(CodeAction::Open(token.to_owned())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_undo_and_body_taps_are_routed_as_distinct_actions() {
        assert!(matches!(
            action(COPY_ACTION, Some("opaque-token")),
            Some(CodeAction::Copy(token)) if token == "opaque-token"
        ));
        assert!(matches!(
            action(UNDO_ACTION, Some("opaque-token")),
            Some(CodeAction::Undo(token)) if token == "opaque-token"
        ));
        assert!(matches!(
            action("tap", Some("opaque-token")),
            Some(CodeAction::Open(token)) if token == "opaque-token"
        ));
        assert_eq!(button_action(&CodeNotificationKind::Copy), COPY_ACTION);
        assert_eq!(button_action(&CodeNotificationKind::Undo), UNDO_ACTION);
    }

    #[test]
    fn other_notifications_and_dismissals_never_trigger_copy_or_open() {
        for identifier in ["tap", COPY_ACTION, UNDO_ACTION, "dismiss", "unknown"] {
            assert!(action(identifier, None).is_none());
            assert!(action(identifier, Some("")).is_none());
        }
        assert!(action("dismiss", Some("opaque-token")).is_none());
        assert!(action("unknown", Some("opaque-token")).is_none());
    }
}
