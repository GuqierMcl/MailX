//! Bounded preparation of new-mail notifications and explicit copy actions.
//! Codes remain in this process; native action payloads contain only random IDs.

use std::{collections::HashMap, sync::Arc, time::Duration};

use mailx_core::mail::verification_code::{Candidate, is_candidate_subject};
use mailx_core::{NewMail, NotificationSettings, SettingsSnapshot, SupportedLocale};
use tauri::{AppHandle, Manager};

use super::{AccountNotifications, Shown, plan_localized, show, window_in_front};
use crate::state::DesktopState;

const MAX_CODE_NOTICES: usize = 5;
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(5);

struct CodeState {
    batches: Arc<tokio::sync::Semaphore>,
    // Lock order: account retirement -> policy -> action store. Final native
    // delivery and clipboard writes share this lock with settings revocation.
    policy: std::sync::Mutex<Option<Policy>>,
    #[cfg(any(target_os = "macos", windows))]
    actions: std::sync::Mutex<ActionStore>,
    #[cfg(any(target_os = "macos", windows))]
    native_available: bool,
}

struct Policy {
    revision: u64,
    settings: NotificationSettings,
    locale: SupportedLocale,
}

fn apply_policy(
    policy: &mut Option<Policy>,
    revision: u64,
    settings: &NotificationSettings,
    locale: SupportedLocale,
) {
    if policy
        .as_ref()
        .is_none_or(|current| current.revision < revision)
    {
        *policy = Some(Policy {
            revision,
            settings: settings.clone(),
            locale,
        });
    }
}

pub(crate) fn setup(app: &AppHandle, accounts: Arc<AccountNotifications>) {
    #[cfg(any(target_os = "macos", windows))]
    let native_available = {
        let handle = app.clone();
        let handler: super::CodeActionHandler = Arc::new(move |action| {
            let handle = handle.clone();
            let accounts = accounts.clone();
            tauri::async_runtime::spawn_blocking(move || handle_action(&handle, &accounts, action));
        });
        native::setup(app, handler).is_ok()
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = accounts;
    app.manage(CodeState {
        batches: Arc::new(tokio::sync::Semaphore::new(2)),
        policy: Default::default(),
        #[cfg(any(target_os = "macos", windows))]
        actions: Default::default(),
        #[cfg(any(target_os = "macos", windows))]
        native_available,
    });
}

/// Initial filtering avoids fetching content for suppressed/private notices.
/// Settings and focus are read again after preparation, immediately before show.
pub(crate) fn new_mail(app: &AppHandle, mail: NewMail, accounts: Arc<AccountNotifications>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let initial = {
            let app = app.clone();
            let mail = mail.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.try_state::<DesktopState>()?;
                let (settings, locale) = state.notification_settings()?;
                plan_localized(&mail, &settings, window_in_front(&app), locale)?;
                Some((settings, state.core().ok()))
            })
            .await
            .ok()
            .flatten()
        };
        let Some((settings, core)) = initial else {
            return;
        };
        let mut found = HashMap::new();
        let gate = app.state::<CodeState>().batches.clone();
        // A burst cannot queue unlimited network jobs or hold ordinary notices.
        if settings.show_preview
            && let Some(core) = core.as_ref()
            && let Ok(_permit) = gate.try_acquire_owned()
        {
            let deadline = tokio::time::Instant::now() + PREPARATION_TIMEOUT;
            for message in mail
                .messages
                .iter()
                .filter(|message| is_candidate_subject(&message.subject))
                .take(MAX_CODE_NOTICES)
            {
                let Ok(snapshot) = app
                    .state::<DesktopState>()
                    .settings(|store| store.get())
                    .await
                else {
                    break;
                };
                let current = &snapshot.settings.notifications;
                if !current.enabled
                    || !current.show_preview
                    || current.muted_accounts.contains(&mail.account_id)
                {
                    break;
                }
                let result = tokio::time::timeout_at(
                    deadline,
                    core.notification_verification_code(message.email_id.clone()),
                )
                .await;
                match result {
                    Ok(Ok(Some(candidate))) => {
                        found.insert(message.email_id.clone(), candidate);
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        }
        tauri::async_runtime::spawn_blocking(move || {
            accounts.deliver(
                &mail.account_id,
                || {
                    let state = app.try_state::<DesktopState>()?;
                    let snapshot = state.notification_snapshot()?;
                    found.retain(|id, _| {
                        core.as_ref().is_some_and(|core| {
                            core.notification_code_is_current(&mail.account_id, id)
                                .unwrap_or(false)
                        })
                    });
                    Some((snapshot, window_in_front(&app), std::mem::take(&mut found)))
                },
                |(snapshot, focused, found)| {
                    let state = app.state::<CodeState>();
                    let mut policy = state.policy.lock().unwrap_or_else(|e| e.into_inner());
                    apply_policy(
                        &mut policy,
                        snapshot.revision,
                        &snapshot.settings.notifications,
                        snapshot.effective_locale,
                    );
                    let policy = policy.as_ref().unwrap();
                    let deliveries =
                        plan_batch(&mail, &policy.settings, focused, policy.locale, &found);
                    for delivery in deliveries {
                        deliver(&app, delivery, policy.locale);
                    }
                },
            );
        })
        .await
        .ok();
    });
}

struct Delivery {
    shown: Shown,
    code: Option<Candidate>,
}

/// Detected messages leave the ordinary account summary, so each arrival is
/// represented once. Sound is played only for the first notice in the batch.
fn plan_batch(
    mail: &NewMail,
    settings: &NotificationSettings,
    focused: bool,
    locale: SupportedLocale,
    found: &HashMap<String, Candidate>,
) -> Vec<Delivery> {
    if plan_localized(mail, settings, focused, locale).is_none() {
        return Vec::new();
    }
    if !settings.show_preview {
        return vec![Delivery {
            shown: plan_localized(mail, settings, focused, locale).unwrap(),
            code: None,
        }];
    }
    let mut remaining = mail.clone();
    let mut deliveries = Vec::new();
    let mut emitted = std::collections::HashSet::new();
    for message in &mail.messages {
        let Some(code) = found.get(&message.email_id) else {
            continue;
        };
        if !emitted.insert(&message.email_id) {
            continue;
        }
        let one = NewMail {
            count: 1,
            messages: vec![message.clone()],
            ..mail.clone()
        };
        let Some(mut shown) = plan_localized(&one, settings, focused, locale) else {
            continue;
        };
        shown.title = format!("{} · {}", shown.title, mail.account_name);
        shown.body = format!("{}\n{}", locale.verification_code(&code.code), shown.body);
        deliveries.push(Delivery {
            shown,
            code: Some(code.clone()),
        });
    }
    remaining
        .messages
        .retain(|message| !emitted.contains(&message.email_id));
    remaining.count = remaining.count.saturating_sub(emitted.len() as u32);
    if let Some(shown) = plan_localized(&remaining, settings, focused, locale) {
        deliveries.push(Delivery { shown, code: None });
    }
    for delivery in deliveries.iter_mut().skip(1) {
        delivery.shown.system_sound = None;
        delivery.shown.own_sound = None;
    }
    deliveries
}

fn deliver(app: &AppHandle, delivery: Delivery, locale: SupportedLocale) {
    #[cfg(any(target_os = "macos", windows))]
    if let Some(code) = delivery.code {
        let state = app.state::<CodeState>();
        if state.native_available
            && let Some(email_id) = delivery.shown.email_id.clone()
        {
            let token = state
                .actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    delivery.shown.account_id.clone(),
                    email_id,
                    code.code,
                    std::time::Instant::now(),
                );
            let notice = super::CodeNotification {
                title: delivery.shown.title.clone(),
                body: delivery.shown.body.clone(),
                copy_label: locale.copy_verification_code(),
                token: token.clone(),
                sound: delivery.shown.system_sound,
            };
            if native::show(app, notice).is_ok() {
                if let Some(sound) = delivery.shown.own_sound {
                    super::sounds::play(sound);
                }
                return;
            }
            state
                .actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&token);
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (delivery.code, locale);
    show(app, delivery.shown);
}

pub(crate) fn settings_changed(app: &AppHandle, snapshot: &SettingsSnapshot) {
    if let Some(state) = app.try_state::<CodeState>() {
        let mut policy = state.policy.lock().unwrap_or_else(|e| e.into_inner());
        apply_policy(
            &mut policy,
            snapshot.revision,
            &snapshot.settings.notifications,
            snapshot.effective_locale,
        );
        #[cfg(any(target_os = "macos", windows))]
        state
            .actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain_allowed(&policy.as_ref().unwrap().settings);
        #[cfg(windows)]
        if !policy.as_ref().unwrap().settings.enabled
            || !policy.as_ref().unwrap().settings.show_preview
        {
            super::windows::clear(app);
        }
    }
}

pub(crate) fn retire_codes(app: &AppHandle, account_id: &str) {
    #[cfg(any(target_os = "macos", windows))]
    if let Some(state) = app.try_state::<CodeState>() {
        state
            .actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .retain(|entry| entry.account_id != account_id);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (app, account_id);
}

#[cfg(target_os = "macos")]
use super::macos as native;
#[cfg(windows)]
use super::windows as native;

#[cfg(any(target_os = "macos", windows))]
#[derive(Clone)]
struct CodeEntry {
    token: String,
    account_id: String,
    email_id: String,
    code: String,
    expires: std::time::Instant,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Default)]
struct ActionStore {
    entries: std::collections::VecDeque<CodeEntry>,
}

#[cfg(any(target_os = "macos", windows))]
impl ActionStore {
    fn insert(
        &mut self,
        account_id: String,
        email_id: String,
        code: String,
        now: std::time::Instant,
    ) -> String {
        self.entries.retain(|entry| entry.expires > now);
        while self.entries.len() >= 128 {
            self.entries.pop_front();
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.entries.push_back(CodeEntry {
            token: token.clone(),
            account_id,
            email_id,
            code,
            expires: now + super::CODE_ACTION_TTL,
        });
        token
    }
    fn get(&mut self, token: &str, now: std::time::Instant) -> Option<CodeEntry> {
        self.entries.retain(|entry| entry.expires > now);
        self.entries
            .iter()
            .find(|entry| entry.token == token)
            .cloned()
    }
    fn remove(&mut self, token: &str) {
        self.entries.retain(|entry| entry.token != token);
    }
    fn retain_allowed(&mut self, settings: &NotificationSettings) {
        self.entries
            .retain(|entry| copy_allowed(settings, &entry.account_id));
    }
}

#[cfg(any(target_os = "macos", windows))]
fn copy_allowed(settings: &NotificationSettings, account_id: &str) -> bool {
    settings.enabled
        && settings.show_preview
        && !settings.muted_accounts.iter().any(|id| id == account_id)
}

#[cfg(any(target_os = "macos", windows))]
fn handle_action(app: &AppHandle, accounts: &AccountNotifications, action: super::CodeAction) {
    let (copy, token) = match action {
        super::CodeAction::Copy(token) => (true, token),
        super::CodeAction::Open(token) => (false, token),
    };
    let Some(state) = app.try_state::<CodeState>() else {
        return;
    };
    let entry = state
        .actions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&token, std::time::Instant::now());
    if !copy {
        if let Some(entry) = entry {
            super::open_email(app, &entry.account_id, &entry.email_id);
        } else {
            crate::window_chrome::focus_main_window(app);
        }
        return;
    }
    let Some(desktop) = app.try_state::<DesktopState>() else {
        return;
    };
    let Some(snapshot) = desktop.notification_snapshot() else {
        return;
    };
    let Some(entry) = entry else {
        let mut policy = state.policy.lock().unwrap_or_else(|e| e.into_inner());
        apply_policy(
            &mut policy,
            snapshot.revision,
            &snapshot.settings.notifications,
            snapshot.effective_locale,
        );
        let policy = policy.as_ref().unwrap();
        if policy.settings.enabled && policy.settings.show_preview {
            copy_unavailable(app, policy.locale, None);
        }
        return;
    };
    accounts.deliver(
        &entry.account_id,
        || {
            Some(desktop.core().ok().is_some_and(|core| {
                core.notification_code_is_current(&entry.account_id, &entry.email_id)
                    .unwrap_or(false)
            }))
        },
        |current| {
            let mut policy = state.policy.lock().unwrap_or_else(|e| e.into_inner());
            apply_policy(
                &mut policy,
                snapshot.revision,
                &snapshot.settings.notifications,
                snapshot.effective_locale,
            );
            let policy = policy.as_ref().unwrap();
            if !copy_allowed(&policy.settings, &entry.account_id) {
                return;
            }
            let locale = policy.locale;
            let mut actions = state.actions.lock().unwrap_or_else(|e| e.into_inner());
            // The first lookup is for identity only. Revocation or an earlier
            // callback may have removed it while the core query was waiting.
            let Some(live) = actions.get(&token, std::time::Instant::now()) else {
                copy_unavailable(app, locale, Some(&entry));
                return;
            };
            if !current {
                copy_unavailable(app, locale, Some(&entry));
                return;
            }
            let copied =
                arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(live.code));
            if copied.is_err() {
                copy_unavailable(app, locale, Some(&entry));
            } else {
                actions.remove(&token);
            }
        },
    );
}

#[cfg(any(target_os = "macos", windows))]
fn copy_unavailable(app: &AppHandle, locale: SupportedLocale, entry: Option<&CodeEntry>) {
    use tauri_plugin_notification::NotificationExt;
    let mut notice = app
        .notification()
        .builder()
        .title(locale.copy_code_unavailable())
        .body(locale.copy_code_open_mail());
    if let Some(entry) = entry {
        notice = notice
            .extra(super::ACCOUNT_KEY, entry.account_id.clone())
            .extra(super::EMAIL_KEY, entry.email_id.clone());
    }
    let _ = notice.show();
}

#[cfg(test)]
mod tests;
