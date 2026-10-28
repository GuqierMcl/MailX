//! Which window may call which host command.
//!
//! Tauri's capabilities already limit commands per window label; this module
//! is the second, host-side check every command makes. Two window kinds exist:
//! the main window and one composer window per draft (`composer-<uuid>`). A
//! composer may only touch its own draft, and the caller identity handed to
//! the core is derived from the window kind here, never from the request.

use mailx_core::{AppError, Caller};
use uuid::Uuid;

use crate::window_chrome::MAIN_WINDOW;

pub(crate) const COMPOSER_PREFIX: &str = "composer-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowKind {
    Main,
    /// The draft this window edits; fixed by the window label.
    Composer(Uuid),
}

/// `composer-{draftId}`.
pub(crate) fn composer_label(draft_id: &Uuid) -> String {
    format!("{COMPOSER_PREFIX}{}", draft_id.hyphenated())
}

/// A draft id in the canonical lowercase hyphenated form; anything else
/// (braces, URN prefix, upper case) is not an id this host created.
pub(crate) fn parse_draft_id(id: &str) -> Option<Uuid> {
    let parsed = Uuid::parse_str(id).ok()?;
    (parsed.hyphenated().to_string() == id).then_some(parsed)
}

impl WindowKind {
    pub(crate) fn parse(label: &str) -> Option<Self> {
        if label == MAIN_WINDOW {
            return Some(Self::Main);
        }
        parse_draft_id(label.strip_prefix(COMPOSER_PREFIX)?).map(Self::Composer)
    }

    /// The principal the core sees for calls from this window.
    pub(crate) fn caller(self) -> Caller {
        match self {
            Self::Main => Caller::MainWindow,
            Self::Composer(_) => Caller::Composer,
        }
    }
}

/// The page itself must be the bundled app (or the dev server in debug
/// builds), without credentials in the URL. Path and query are free: a
/// composer window loads `index.html?composer=<id>`.
pub(crate) fn url_allowed(url: &tauri::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let bundled = matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
    );
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(5173);
    bundled || development
}

/// Commands only the main window may call.
const MAIN_ONLY: &[&str] = &[
    "app_get_startup_status",
    "app_retry_startup",
    "accounts_get",
    "accounts_create",
    "accounts_remove",
    "accounts_update_connection",
    "accounts_update_profile",
    "accounts_verify_connection",
    "accounts_cancel_verification",
    "accounts_open_provider_help",
    "folders_list",
    "emails_list",
    "emails_get",
    "emails_fetch_body",
    "emails_prepare_body",
    "emails_image_status",
    "emails_retry_images",
    "emails_release_body",
    "emails_body_links",
    "links_request",
    "links_open",
    "sync_states",
    "sync_refresh",
    "emails_set_flag",
    "attachments_save",
    "avatars_get",
    "image_cache_clear",
    "settings_update",
    "notifications_test",
    "notifications_preview_sound",
    "drafts_create",
    "drafts_create_from",
    "drafts_list",
    "outbox_list",
    "outbox_get",
    "outbox_cancel",
    "outbox_retry_now",
    "outbox_resolve_unknown",
    "composer_open",
    "updater_get_state",
    "updater_check",
    "updater_download",
    "updater_cancel",
    "updater_restart",
    "updater_open_link",
];

/// Commands both kinds may call.
const BOTH: &[&str] = &[
    "accounts_list",
    "settings_get",
    "window_get_chrome",
    "window_perform",
    "window_set_theme",
    "drafts_get",
    "drafts_update",
    "drafts_delete",
];

/// Commands only a composer window may call.
const COMPOSER_ONLY: &[&str] = &["outbox_queue_send", "composer_close"];

/// Commands that name a draft; a composer may only name its own.
const DRAFT_SCOPED: &[&str] = &[
    "drafts_get",
    "drafts_update",
    "drafts_delete",
    "outbox_queue_send",
];

pub(crate) fn command_allowed(kind: WindowKind, command: &str) -> bool {
    match kind {
        WindowKind::Main => MAIN_ONLY.contains(&command) || BOTH.contains(&command),
        WindowKind::Composer(_) => COMPOSER_ONLY.contains(&command) || BOTH.contains(&command),
    }
}

/// Every command with its permission identifier, for tests and docs.
#[cfg(test)]
pub(crate) fn all_commands() -> impl Iterator<Item = &'static str> {
    MAIN_ONLY.iter().chain(BOTH).chain(COMPOSER_ONLY).copied()
}

/// Decides whether this window may call this command at all. Every refusal is
/// the same redacted `validation` failure, so a page learns nothing about the
/// whitelist.
pub(crate) fn check_kind(
    label: &str,
    url: &tauri::Url,
    command: &str,
) -> Result<WindowKind, AppError> {
    let kind = WindowKind::parse(label).ok_or_else(refused)?;
    if url_allowed(url) && command_allowed(kind, command) {
        Ok(kind)
    } else {
        Err(refused())
    }
}

/// For commands that name a draft: a composer window may only name its own.
pub(crate) fn check_draft(kind: WindowKind, command: &str, draft: &str) -> Result<(), AppError> {
    match kind {
        WindowKind::Composer(own)
            if DRAFT_SCOPED.contains(&command) && parse_draft_id(draft) != Some(own) =>
        {
            Err(refused())
        }
        _ => Ok(()),
    }
}

fn refused() -> AppError {
    AppError::validation("caller")
}

/// `check_kind` and `check_draft` in one step. `draft` is the id the request
/// names; the check fails closed if a draft-scoped command is checked without
/// one.
#[cfg(test)]
pub(crate) fn check(
    label: &str,
    url: &tauri::Url,
    command: &str,
    draft: Option<&str>,
) -> Result<WindowKind, AppError> {
    let kind = check_kind(label, url, command)?;
    check_draft(kind, command, draft.unwrap_or(""))?;
    Ok(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> tauri::Url {
        tauri::Url::parse(text).unwrap()
    }

    fn id() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn labels_parse_to_window_kinds() {
        let draft = id();
        assert_eq!(WindowKind::parse("main"), Some(WindowKind::Main));
        assert_eq!(
            WindowKind::parse(&composer_label(&draft)),
            Some(WindowKind::Composer(draft))
        );
        for label in [
            "composer-x",
            "composer-",
            "composer",
            "Main",
            "mail-body",
            "other",
            "",
            "main2",
            "composer-main",
            &format!("composer-{}", draft.hyphenated().to_string().to_uppercase()),
            &format!("composer-{}", draft.simple()),
            &format!("composer-{{{}}}", draft.hyphenated()),
            &format!("composer-{}-extra", draft.hyphenated()),
            &format!("composer-composer-{}", draft.hyphenated()),
        ] {
            assert_eq!(WindowKind::parse(label), None, "{label}");
        }
    }

    #[test]
    fn window_kinds_map_to_core_callers() {
        assert_eq!(WindowKind::Main.caller(), Caller::MainWindow);
        assert_eq!(WindowKind::Composer(id()).caller(), Caller::Composer);
    }

    #[test]
    fn origin_checks_are_the_same_for_both_kinds() {
        let draft = id();
        let labels = ["main".to_owned(), composer_label(&draft)];
        for label in &labels {
            for ok in [
                "tauri://localhost",
                "http://tauri.localhost",
                "https://tauri.localhost",
                "tauri://localhost/index.html?composer=x",
            ] {
                assert!(
                    check(label, &url(ok), "settings_get", None).is_ok(),
                    "{label} {ok}"
                );
            }
            for bad in [
                "https://example.com",
                "http://localhost:5174",
                "https://localhost:5173",
                "http://tauri.localhost:9000",
                "http://user:pw@tauri.localhost",
                "mailx-body://localhost/document/example",
                "http://mailx-body.localhost/document/example",
            ] {
                assert!(
                    check(label, &url(bad), "settings_get", None).is_err(),
                    "{label} {bad}"
                );
            }
            assert_eq!(
                check(label, &url("http://localhost:5173"), "settings_get", None).is_ok(),
                cfg!(debug_assertions)
            );
        }
        assert!(check("mail-body", &url("tauri://localhost"), "settings_get", None).is_err());
    }

    #[test]
    fn the_whitelist_table() {
        let composer = WindowKind::Composer(id());
        let main = WindowKind::Main;
        // (command, main, composer)
        let table = [
            ("app_get_startup_status", true, false),
            ("accounts_list", true, true),
            ("accounts_get", true, false),
            ("accounts_create", true, false),
            ("accounts_remove", true, false),
            ("accounts_update_connection", true, false),
            ("accounts_update_profile", true, false),
            ("accounts_verify_connection", true, false),
            ("accounts_cancel_verification", true, false),
            ("accounts_open_provider_help", true, false),
            ("folders_list", true, false),
            ("emails_list", true, false),
            ("emails_get", true, false),
            ("emails_fetch_body", true, false),
            ("emails_prepare_body", true, false),
            ("emails_image_status", true, false),
            ("emails_retry_images", true, false),
            ("emails_release_body", true, false),
            ("emails_body_links", true, false),
            ("links_request", true, false),
            ("links_open", true, false),
            ("attachments_save", true, false),
            ("avatars_get", true, false),
            ("image_cache_clear", true, false),
            ("emails_set_flag", true, false),
            ("sync_states", true, false),
            ("sync_refresh", true, false),
            ("settings_get", true, true),
            ("settings_update", true, false),
            ("notifications_test", true, false),
            ("notifications_preview_sound", true, false),
            ("window_get_chrome", true, true),
            ("window_perform", true, true),
            ("window_set_theme", true, true),
            ("drafts_create", true, false),
            ("drafts_create_from", true, false),
            ("drafts_list", true, false),
            ("drafts_get", true, true),
            ("drafts_update", true, true),
            ("drafts_delete", true, true),
            ("outbox_queue_send", false, true),
            ("outbox_list", true, false),
            ("outbox_get", true, false),
            ("outbox_cancel", true, false),
            ("outbox_retry_now", true, false),
            ("outbox_resolve_unknown", true, false),
            ("composer_open", true, false),
            ("composer_close", false, true),
            ("updater_get_state", true, false),
            ("updater_check", true, false),
            ("updater_download", true, false),
            ("updater_cancel", true, false),
            ("updater_restart", true, false),
            ("updater_open_link", true, false),
        ];
        for (command, by_main, by_composer) in table {
            assert_eq!(command_allowed(main, command), by_main, "main {command}");
            assert_eq!(
                command_allowed(composer, command),
                by_composer,
                "composer {command}"
            );
        }
        // The table covers every whitelisted command, and nothing else exists.
        let listed: Vec<_> = table.iter().map(|row| row.0).collect();
        for command in all_commands() {
            assert!(
                listed.contains(&command) || command == "app_retry_startup",
                "{command}"
            );
        }
        assert!(!command_allowed(main, "settings_set"));
        assert!(!command_allowed(composer, "nonexistent"));
    }

    #[test]
    fn a_composer_may_only_touch_its_own_draft() {
        let own = id();
        let other = id();
        let label = composer_label(&own);
        let app = url("tauri://localhost");
        for command in [
            "drafts_get",
            "drafts_update",
            "drafts_delete",
            "outbox_queue_send",
        ] {
            let own_text = own.hyphenated().to_string();
            let other_text = other.hyphenated().to_string();
            assert_eq!(
                check(&label, &app, command, Some(&own_text)),
                Ok(WindowKind::Composer(own))
            );
            assert!(check(&label, &app, command, Some(&other_text)).is_err());
            assert!(check(&label, &app, command, Some(&own_text.to_uppercase())).is_err());
            assert!(check(&label, &app, command, Some("")).is_err());
            // Fails closed when the caller forgot to name the draft.
            assert!(check(&label, &app, command, None).is_err());
        }
    }

    #[test]
    fn queue_send_is_refused_from_main_and_resolve_is_allowed_from_it() {
        let app = url("tauri://localhost");
        let draft = id().hyphenated().to_string();
        assert!(check("main", &app, "outbox_queue_send", Some(&draft)).is_err());
        assert_eq!(
            check("main", &app, "outbox_resolve_unknown", None),
            Ok(WindowKind::Main)
        );
        // A composer may not manage the outbox: settling goes through main.
        let label = composer_label(&id());
        assert!(check(&label, &app, "outbox_resolve_unknown", None).is_err());
        assert!(check(&label, &app, "composer_open", None).is_err());
        assert!(check("main", &app, "composer_close", None).is_err());
    }
}
