//! Application preferences stored as `settings.json` in the data directory.
//!
//! The file is meant to be readable and hand-editable, so it is pretty-printed
//! JSON. Every field has a default; an unreadable or newer-version file is moved
//! aside to `settings.json.bak` instead of being overwritten in place. Settings
//! never hold secrets and do not depend on account storage being open.

use crate::SupportedLocale;
use crate::accounts::types::ProxyKind;
use crate::error::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use ts_rs::TS;
use uuid::Uuid;

pub const SETTINGS_FILE_NAME: &str = "settings.json";
const BACKUP_FILE_NAME: &str = "settings.json.bak";
const CURRENT_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;

pub const NAV_WIDTH_MIN: u32 = 220;
pub const NAV_WIDTH_MAX: u32 = 420;
const NAV_WIDTH_DEFAULT: u32 = 256;

pub const LIST_WIDTH_MIN: u32 = 300;
pub const LIST_WIDTH_MAX: u32 = 2000;
const LIST_WIDTH_DEFAULT: u32 = 420;

const MAX_FONT_FAMILIES: usize = 16;
const MAX_FONT_FAMILY_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FontSize {
    Small,
    #[default]
    Default,
    Large,
}

/// How many lines of body preview each message list row shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PreviewLines {
    None,
    #[default]
    One,
    Two,
    Three,
}

/// How message bodies follow the app's color mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MailTheme {
    /// Adapt message bodies to dark mode when the app is dark.
    #[default]
    Follow,
    /// Always show message bodies on a white page.
    Light,
}

/// Empty strings mean "use the built-in default".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceSettings {
    pub theme: ThemeMode,
    /// `""` or a lowercase `#rrggbb` color.
    pub accent_color: String,
    /// `""` or comma-separated font families, tried in order.
    pub interface_font: String,
    /// `""` or comma-separated font families for message bodies, tried in order.
    pub mail_font: String,
    pub mail_theme: MailTheme,
    pub font_size: FontSize,
    pub preview_lines: PreviewLines,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RemoteImagePolicy {
    #[default]
    Ask,
    Automatic,
    Never,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct SecuritySettings {
    pub remote_images: RemoteImagePolicy,
    /// Look sender avatars up on Gravatar, which learns a hash of each sender
    /// address. Off by default.
    pub sender_avatars: bool,
}

/// The sound of a new-mail notification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NotificationSound {
    /// Whatever the system plays for notifications.
    #[default]
    System,
    /// Two short rising notes.
    Chime,
    /// A single soft bell tone.
    Ding,
    /// A quick rising drop.
    Droplet,
    /// Two rising pops.
    Bubble,
    /// Two warm wooden notes.
    Marimba,
    /// A fast rising arpeggio.
    Sparkle,
    /// Two soft low taps.
    Knock,
    /// A major chord ringing out.
    Chord,
    None,
}

const MAX_MUTED_ACCOUNTS: usize = 256;
const MAX_ACCOUNT_ID_CHARS: usize = 128;
const MAX_ACCOUNT_ORDER: usize = 256;

/// New-mail notifications. Every host applies `enabled`, `muted_accounts` and
/// `show_preview` the same way (see `NewMail::for_delivery`); the sound and
/// focus rules only mean something where there is a screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub enabled: bool,
    /// Include sender and subject. Off, a notification only says how many.
    pub show_preview: bool,
    /// Also notify while the MailX window is in front.
    pub when_focused: bool,
    pub sound: NotificationSound,
    /// Accounts whose new mail never notifies.
    pub muted_accounts: Vec<String>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_preview: true,
            when_focused: false,
            sound: NotificationSound::default(),
            muted_accounts: Vec::new(),
        }
    }
}

impl NotificationSettings {
    /// Sorted, unique, bounded account ids; `None` for input that is not.
    fn normalized_muted(&self) -> Option<Vec<String>> {
        let mut ids: Vec<String> = self
            .muted_accounts
            .iter()
            .map(|id| id.trim().to_owned())
            .collect();
        if ids.iter().any(|id| {
            id.is_empty()
                || id.chars().count() > MAX_ACCOUNT_ID_CHARS
                || id.chars().any(char::is_control)
        }) {
            return None;
        }
        ids.sort();
        ids.dedup();
        (ids.len() <= MAX_MUTED_ACCOUNTS).then_some(ids)
    }
}

/// Window layout and account display preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct LayoutSettings {
    /// Width of the navigation sidebar, `220..=420`.
    pub nav_width: u32,
    /// The navigation sidebar is folded to its icon rail. A narrow window may
    /// show the rail on its own without changing this.
    pub nav_collapsed: bool,
    /// Width of the message list beside the reader, `300..=2000`, applied when
    /// the window opens. In a window that is too narrow for it, list and reader
    /// shrink together, each down to its own minimum.
    pub list_width: u32,
    /// The reader beside the message list is folded away. Like the sidebar's
    /// rail, this survives a restart: the window comes back as it was left.
    pub reader_collapsed: bool,
    /// Display order only; unknown account ids are ignored by the UI.
    #[serde(deserialize_with = "deserialize_account_order")]
    pub account_order: Vec<String>,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            nav_width: NAV_WIDTH_DEFAULT,
            nav_collapsed: false,
            list_width: LIST_WIDTH_DEFAULT,
            reader_collapsed: false,
            account_order: Vec::new(),
        }
    }
}

impl LayoutSettings {
    fn normalized_order(&self) -> Option<Vec<String>> {
        if self.account_order.len() > MAX_ACCOUNT_ORDER {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        let mut order = Vec::with_capacity(self.account_order.len());
        for id in &self.account_order {
            let id = Uuid::parse_str(id.trim()).ok()?.to_string();
            if seen.insert(id.clone()) {
                order.push(id);
            }
        }
        Some(order)
    }
}

// Route malformed values through field validation; file loading then
// recovers just this preference instead of discarding all other settings.
fn deserialize_account_order<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_else(|_| vec![String::new()]))
}

fn deserialize_order_patch<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    deserialize_account_order(deserializer).map(Some)
}

/// Layout fields merge under the settings-store lock. Omitted fields preserve
/// concurrent edits from the other layout controls and windows.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutSettingsPatch {
    #[ts(optional)]
    pub nav_width: Option<u32>,
    #[ts(optional)]
    pub nav_collapsed: Option<bool>,
    #[ts(optional)]
    pub list_width: Option<u32>,
    #[ts(optional)]
    pub reader_collapsed: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_order_patch")]
    #[ts(optional)]
    pub account_order: Option<Vec<String>>,
}

impl LayoutSettingsPatch {
    fn merge(self, current: &LayoutSettings) -> LayoutSettings {
        LayoutSettings {
            nav_width: self.nav_width.unwrap_or(current.nav_width),
            nav_collapsed: self.nav_collapsed.unwrap_or(current.nav_collapsed),
            list_width: self.list_width.unwrap_or(current.list_width),
            reader_collapsed: self.reader_collapsed.unwrap_or(current.reader_collapsed),
            account_order: self
                .account_order
                .unwrap_or_else(|| current.account_order.clone()),
        }
    }
}

impl From<LayoutSettings> for LayoutSettingsPatch {
    fn from(settings: LayoutSettings) -> Self {
        Self {
            nav_width: Some(settings.nav_width),
            nav_collapsed: Some(settings.nav_collapsed),
            list_width: Some(settings.list_width),
            reader_collapsed: Some(settings.reader_collapsed),
            account_order: Some(settings.account_order),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TranslationProviderId {
    Deepl,
    Microsoft,
    Google,
}

impl TranslationProviderId {
    pub const ALL: [Self; 3] = [Self::Deepl, Self::Microsoft, Self::Google];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deepl => "deepl",
            Self::Microsoft => "microsoft",
            Self::Google => "google",
        }
    }
}

/// How translation requests reach the service. The mail accounts' own proxies
/// are never used. Proxy credentials are not supported in this version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TranslationRoute {
    #[default]
    Direct,
    Proxy {
        proxy_kind: ProxyKind,
        host: String,
        port: u16,
    },
}

const MAX_LANGUAGE_RULES: usize = 64;
const MAX_PROXY_HOST_CHARS: usize = 255;
const MAX_MICROSOFT_REGION_CHARS: usize = 40;

/// Mail translation through a third-party service. `consented_provider` is the
/// service the user agreed to send mail text to; changing the provider
/// invalidates the consent until the user agrees again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslationSettings {
    /// `None` until a service is chosen.
    pub provider: Option<TranslationProviderId>,
    /// Azure resource region; `""` for a global resource.
    pub microsoft_region: String,
    pub route: TranslationRoute,
    /// `"auto"` (follow the UI language) or one of `translate::languages::TARGETS`.
    pub target_language: String,
    /// Offer to translate when a mail is in another language.
    pub suggest: bool,
    /// Base language codes that never get the offer.
    pub never_suggest: Vec<String>,
    /// Base language codes translated without asking.
    pub always_translate: Vec<String>,
    /// Translate folded quoted text too.
    pub include_quotes: bool,
    pub consented_provider: Option<TranslationProviderId>,
}

impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            provider: None,
            microsoft_region: String::new(),
            route: TranslationRoute::Direct,
            target_language: "auto".into(),
            suggest: true,
            never_suggest: Vec::new(),
            always_translate: Vec::new(),
            include_quotes: false,
            consented_provider: None,
        }
    }
}

fn normalize_language_codes(codes: &[String]) -> Option<Vec<String>> {
    if codes.len() > MAX_LANGUAGE_RULES {
        return None;
    }
    let mut out: Vec<String> = codes.to_vec();
    if out
        .iter()
        .any(|code| !(2..=3).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_lowercase()))
    {
        return None;
    }
    out.sort();
    out.dedup();
    Some(out)
}

impl TranslationSettings {
    /// Validates client input and returns its canonical form.
    pub fn normalized(self) -> Result<Self, AppError> {
        if self.target_language != "auto"
            && !crate::translate::languages::is_target(&self.target_language)
        {
            return Err(AppError::validation("translation.targetLanguage"));
        }
        let never_suggest = normalize_language_codes(&self.never_suggest)
            .ok_or_else(|| AppError::validation("translation.neverSuggest"))?;
        let always_translate = normalize_language_codes(&self.always_translate)
            .ok_or_else(|| AppError::validation("translation.alwaysTranslate"))?;
        if never_suggest
            .iter()
            .any(|code| always_translate.contains(code))
        {
            return Err(AppError::validation("translation.alwaysTranslate"));
        }
        let region = &self.microsoft_region;
        if region.len() > MAX_MICROSOFT_REGION_CHARS
            || !region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(AppError::validation("translation.microsoftRegion"));
        }
        if let TranslationRoute::Proxy { host, port, .. } = &self.route {
            let bad_host = host.is_empty()
                || host.chars().count() > MAX_PROXY_HOST_CHARS
                || host.chars().any(|c| c.is_whitespace() || c.is_control());
            if bad_host {
                return Err(AppError::validation("translation.route.host"));
            }
            if *port == 0 {
                return Err(AppError::validation("translation.route.port"));
            }
        }
        Ok(Self {
            never_suggest,
            always_translate,
            ..self
        })
    }

    /// Hand-edited files fall back per field.
    fn sanitized(self) -> Self {
        let defaults = Self::default();
        let mut never_suggest = normalize_language_codes(&self.never_suggest).unwrap_or_default();
        let always_translate = normalize_language_codes(&self.always_translate).unwrap_or_default();
        never_suggest.retain(|code| !always_translate.contains(code));
        let valid_route = Self {
            route: self.route.clone(),
            ..defaults.clone()
        }
        .normalized()
        .is_ok();
        let valid_region = Self {
            microsoft_region: self.microsoft_region.clone(),
            ..defaults.clone()
        }
        .normalized()
        .is_ok();
        let valid_target = self.target_language == "auto"
            || crate::translate::languages::is_target(&self.target_language);
        Self {
            provider: self.provider,
            microsoft_region: if valid_region {
                self.microsoft_region
            } else {
                defaults.microsoft_region
            },
            route: if valid_route {
                self.route
            } else {
                defaults.route
            },
            target_language: if valid_target {
                self.target_language
            } else {
                defaults.target_language
            },
            suggest: self.suggest,
            never_suggest,
            always_translate,
            include_quotes: self.include_quotes,
            consented_provider: self.consented_provider,
        }
    }
}

/// Drops translation fields of the wrong type or with unknown values, so a
/// hand-edited file loses only that field instead of the whole file.
fn repair_translation(value: &mut serde_json::Value) {
    let Some(section) = value.as_object_mut() else {
        return;
    };
    let is_provider = |v: &serde_json::Value| {
        v.is_null() || serde_json::from_value::<TranslationProviderId>(v.clone()).is_ok()
    };
    let is_codes = |v: &serde_json::Value| {
        v.as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string))
    };
    section.retain(|key, v| match key.as_str() {
        "provider" | "consentedProvider" => is_provider(v),
        "route" => serde_json::from_value::<TranslationRoute>(v.clone()).is_ok(),
        "microsoftRegion" | "targetLanguage" => v.is_string(),
        "suggest" | "includeQuotes" => v.is_boolean(),
        "neverSuggest" | "alwaysTranslate" => is_codes(v),
        _ => true,
    });
}

/// Seconds a sent message waits in the queue where it can still be undone.
pub const UNDO_SEND_SECONDS: [u32; 5] = [0, 5, 10, 20, 30];

/// Writing and sending mail.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct ComposeSettings {
    /// Account used for new mail outside any account's folders; `""` for none.
    /// It is not checked against the accounts here: a missing account falls
    /// back to the first one when a draft is created.
    pub default_account_id: String,
    /// One of 0 (send at once), 5, 10, 20 or 30.
    #[ts(type = "number")]
    pub undo_send_seconds: u32,
}

/// Local MCP access for other programs on this computer. Only the desktop
/// host's dedicated command changes it, so a generic settings save can never
/// open or close the endpoint without the server being started or stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct McpSettings {
    /// Off until the user turns it on.
    pub enabled: bool,
}

impl ComposeSettings {
    /// Trimmed account id; `None` for input that is not one.
    fn normalized_account(&self) -> Option<String> {
        let id = self.default_account_id.trim();
        (id.chars().count() <= MAX_ACCOUNT_ID_CHARS && !id.chars().any(char::is_control))
            .then(|| id.to_owned())
    }
}

/// What closing the main window does on platforms with a notification area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CloseBehavior {
    #[default]
    Ask,
    Background,
    Quit,
}

/// Language preference is validated on input; a string lets field validation
/// report `general.language` rather than a generic deserialization error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralSettings {
    #[ts(type = "LanguagePreference")]
    #[serde(deserialize_with = "deserialize_language")]
    pub language: String,
    /// Download new versions automatically and install on a normal application
    /// exit. Update checks always run; the desktop never restarts by itself.
    pub auto_update: bool,
    /// Ask until the user chooses whether closing should keep MailX running.
    /// The desktop host applies platform and tray availability constraints.
    pub close_behavior: CloseBehavior,
    /// Optional macOS menu bar shortcut; the Dock remains available.
    pub show_menu_bar_icon: bool,
    /// Hide the main window only on a registered desktop login launch. The
    /// desktop host still requires an available Dock/tray recovery entry.
    pub silent_start: bool,
}

fn deserialize_language<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    // Keep malformed values in the normal field-validation path so IPC gets a
    // useful field name without echoing any of the original input.
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            auto_update: true,
            close_behavior: CloseBehavior::Ask,
            show_menu_bar_icon: true,
            silent_start: false,
        }
    }
}

fn valid_language(value: &str) -> bool {
    matches!(value, "system" | "zh-CN" | "en")
}

/// Section updates merge under the store lock so simultaneous saves cannot
/// overwrite another settings page with a stale frontend snapshot.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    #[ts(optional)]
    pub general: Option<GeneralSettings>,
    #[ts(optional)]
    pub appearance: Option<AppearanceSettings>,
    #[ts(optional)]
    pub security: Option<SecuritySettings>,
    #[ts(optional)]
    pub layout: Option<LayoutSettingsPatch>,
    #[ts(optional)]
    pub notifications: Option<NotificationSettings>,
    #[ts(optional)]
    pub compose: Option<ComposeSettings>,
    #[ts(optional)]
    pub translation: Option<TranslationSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub general: GeneralSettings,
    pub appearance: AppearanceSettings,
    pub security: SecuritySettings,
    pub layout: LayoutSettings,
    pub notifications: NotificationSettings,
    pub compose: ComposeSettings,
    pub mcp: McpSettings,
    pub translation: TranslationSettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub settings: AppSettings,
    /// False until a settings file exists on disk.
    pub stored: bool,
    /// The previous file could not be used and was moved to `settings.json.bak`.
    pub recovered: bool,
    pub effective_locale: SupportedLocale,
    /// Monotonic within this settings-store lifetime; never persisted.
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChanged {
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    version: u32,
    #[serde(default)]
    settings: AppSettings,
}

#[derive(Serialize)]
struct SettingsFileRef<'a> {
    version: u32,
    settings: &'a AppSettings,
}

impl AppSettings {
    /// Validates client input and returns its canonical form.
    pub fn normalized(self) -> Result<Self, AppError> {
        if !valid_language(&self.general.language) {
            return Err(AppError::validation("general.language"));
        }
        let appearance = self.appearance;
        if !(NAV_WIDTH_MIN..=NAV_WIDTH_MAX).contains(&self.layout.nav_width) {
            return Err(AppError::validation("layout.navWidth"));
        }
        if !(LIST_WIDTH_MIN..=LIST_WIDTH_MAX).contains(&self.layout.list_width) {
            return Err(AppError::validation("layout.listWidth"));
        }
        let layout = LayoutSettings {
            account_order: self
                .layout
                .normalized_order()
                .ok_or_else(|| AppError::validation("layout.accountOrder"))?,
            ..self.layout
        };
        let notifications = NotificationSettings {
            muted_accounts: self
                .notifications
                .normalized_muted()
                .ok_or_else(|| AppError::validation("notifications.mutedAccounts"))?,
            ..self.notifications
        };
        if !UNDO_SEND_SECONDS.contains(&self.compose.undo_send_seconds) {
            return Err(AppError::validation("compose.undoSendSeconds"));
        }
        let compose = ComposeSettings {
            default_account_id: self
                .compose
                .normalized_account()
                .ok_or_else(|| AppError::validation("compose.defaultAccountId"))?,
            ..self.compose
        };
        Ok(Self {
            general: self.general,
            security: self.security,
            layout,
            notifications,
            compose,
            mcp: self.mcp,
            translation: self.translation.normalized()?,
            appearance: AppearanceSettings {
                accent_color: normalize_accent_color(&appearance.accent_color)
                    .ok_or_else(|| AppError::validation("appearance.accentColor"))?,
                interface_font: normalize_font_families(&appearance.interface_font)
                    .ok_or_else(|| AppError::validation("appearance.interfaceFont"))?,
                mail_font: normalize_font_families(&appearance.mail_font)
                    .ok_or_else(|| AppError::validation("appearance.mailFont"))?,
                ..appearance
            },
        })
    }

    /// Hand-edited files fall back per field instead of discarding everything.
    fn sanitized(self) -> Self {
        let defaults = AppearanceSettings::default();
        let appearance = self.appearance;
        let notifications = NotificationSettings {
            muted_accounts: self.notifications.normalized_muted().unwrap_or_default(),
            ..self.notifications
        };
        let compose = ComposeSettings {
            default_account_id: self.compose.normalized_account().unwrap_or_default(),
            undo_send_seconds: if UNDO_SEND_SECONDS.contains(&self.compose.undo_send_seconds) {
                self.compose.undo_send_seconds
            } else {
                0
            },
        };
        Self {
            general: if valid_language(&self.general.language) {
                self.general
            } else {
                GeneralSettings {
                    language: GeneralSettings::default().language,
                    ..self.general
                }
            },
            security: self.security,
            notifications,
            compose,
            mcp: self.mcp,
            translation: self.translation.sanitized(),
            layout: LayoutSettings {
                nav_width: self.layout.nav_width.clamp(NAV_WIDTH_MIN, NAV_WIDTH_MAX),
                nav_collapsed: self.layout.nav_collapsed,
                list_width: self.layout.list_width.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX),
                reader_collapsed: self.layout.reader_collapsed,
                account_order: self.layout.normalized_order().unwrap_or_default(),
            },
            appearance: AppearanceSettings {
                accent_color: normalize_accent_color(&appearance.accent_color)
                    .unwrap_or(defaults.accent_color),
                interface_font: normalize_font_families(&appearance.interface_font)
                    .unwrap_or(defaults.interface_font),
                mail_font: normalize_font_families(&appearance.mail_font)
                    .unwrap_or(defaults.mail_font),
                ..appearance
            },
        }
    }
}

fn normalize_accent_color(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return Some(String::new());
    }
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// Splits on commas, trims and unquotes each family, and drops empty entries.
/// Characters that could escape a quoted CSS family name are rejected.
fn normalize_font_families(value: &str) -> Option<String> {
    let mut families = Vec::new();
    for raw in value.split(',') {
        let mut family = raw.trim();
        for quote in ['"', '\''] {
            if let Some(inner) = family
                .strip_prefix(quote)
                .and_then(|rest| rest.strip_suffix(quote))
            {
                family = inner.trim();
                break;
            }
        }
        if family.is_empty() {
            continue;
        }
        let unsafe_char = |c: char| c.is_control() || "\"'\\;{}()<>".contains(c);
        if family.chars().count() > MAX_FONT_FAMILY_CHARS || family.chars().any(unsafe_char) {
            return None;
        }
        families.push(family);
    }
    (families.len() <= MAX_FONT_FAMILIES).then(|| families.join(", "))
}

const DEFAULT_MAIL_FONTS: &str = "-apple-system, BlinkMacSystemFont, \"Segoe UI\", \"PingFang SC\", \"Hiragino Sans GB\", \"Microsoft YaHei UI\", \"Microsoft YaHei\", \"Noto Sans CJK SC\", \"Noto Sans SC\", Roboto, \"Helvetica Neue\", Arial, sans-serif";
const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "math",
    "emoji",
    "fangsong",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
];

/// Pill-like toggle for a folded quote (`render::fold_quotes`). The marker is
/// hidden for WebKit/Blink and Gecko; hover and keyboard focus are visible.
const QUOTE_FOLD_CSS: &str = concat!(
    "details.mailx-quote{margin:12px 0 0}",
    "details.mailx-quote>summary{display:inline-block;list-style:none;cursor:pointer;user-select:none;",
    "padding:0 12px;border-radius:999px;background:#eceff1;color:#5f6368;font-size:12px;line-height:22px}",
    "details.mailx-quote>summary::-webkit-details-marker{display:none}",
    "details.mailx-quote>summary::before{content:\"\\00b7\\00b7\\00b7  \"}",
    "details.mailx-quote>summary:hover{background:#dde1e4;color:#3c4043}",
    "details.mailx-quote>summary:focus-visible{outline:2px solid #1a73e8;outline-offset:2px}",
    "details.mailx-quote[open]>summary{margin-bottom:12px}",
);

/// Base style for the message document: only what the message itself does not
/// specify. Family names were validated by `normalized`, so quoting is safe.
pub fn mail_body_css(appearance: &AppearanceSettings) -> String {
    let families: Vec<&str> = appearance
        .mail_font
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty() && !family.chars().any(|c| "\"'\\;{}()<>".contains(c)))
        .collect();
    let mut font = if families.is_empty() {
        DEFAULT_MAIL_FONTS.to_owned()
    } else {
        let mut css: Vec<String> = families
            .iter()
            .map(|family| {
                if GENERIC_FAMILIES.contains(&family.to_ascii_lowercase().as_str()) {
                    (*family).to_owned()
                } else {
                    format!("\"{family}\"")
                }
            })
            .collect();
        if !families
            .iter()
            .any(|family| GENERIC_FAMILIES.contains(&family.to_ascii_lowercase().as_str()))
        {
            css.push("sans-serif".into());
        }
        css.join(", ")
    };
    font.insert_str(0, "font-family:");
    let size = match appearance.font_size {
        FontSize::Small => "13px",
        FontSize::Default => "14px",
        FontSize::Large => "16px",
    };
    format!(
        "html{{background:white;color:black}}body{{margin:0;{font};font-size:{size};line-height:1.5}}.mailx-page{{box-sizing:border-box;min-height:100vh;padding:16px;overflow-wrap:break-word}}.mailx-body{{box-sizing:border-box;min-height:100vh;margin:-16px;padding:16px}}img{{max-width:100%;height:auto}}{QUOTE_FOLD_CSS}"
    )
}

struct State {
    settings: AppSettings,
    stored: bool,
    recovered: bool,
    revision: u64,
}

impl State {
    fn snapshot(&self, system_locale: SupportedLocale) -> SettingsSnapshot {
        SettingsSnapshot {
            settings: self.settings.clone(),
            stored: self.stored,
            recovered: self.recovered,
            effective_locale: SupportedLocale::resolve(
                &self.settings.general.language,
                system_locale,
            ),
            revision: self.revision,
        }
    }
}

pub struct SettingsStore {
    directory: PathBuf,
    state: Mutex<State>,
    system_locale: SupportedLocale,
}

fn storage_error() -> AppError {
    AppError::new(ErrorCode::StorageUnavailable)
}

impl SettingsStore {
    /// Loads settings without creating the directory or file. Only I/O errors
    /// other than a missing file fail; unusable content is moved aside.
    pub fn open(data_directory: &Path) -> Result<Self, AppError> {
        Self::open_with_locale(data_directory, SupportedLocale::En)
    }

    /// Hosts resolve the system language once; the core never calls platform APIs.
    pub fn open_with_locale(
        data_directory: &Path,
        system_locale: SupportedLocale,
    ) -> Result<Self, AppError> {
        let path = data_directory.join(SETTINGS_FILE_NAME);
        let state = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => State {
                settings: AppSettings::default(),
                stored: false,
                recovered: false,
                revision: 0,
            },
            Err(_) => return Err(storage_error()),
            Ok(metadata) => match read_file(&path, &metadata) {
                Some(settings) => State {
                    settings,
                    stored: true,
                    recovered: false,
                    revision: 0,
                },
                None => {
                    fs::rename(&path, data_directory.join(BACKUP_FILE_NAME))
                        .map_err(|_| storage_error())?;
                    State {
                        settings: AppSettings::default(),
                        stored: false,
                        recovered: true,
                        revision: 0,
                    }
                }
            },
        };
        Ok(Self {
            directory: data_directory.to_owned(),
            state: Mutex::new(state),
            system_locale,
        })
    }

    pub fn get(&self) -> Result<SettingsSnapshot, AppError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| storage_error())?
            .snapshot(self.system_locale))
    }

    pub fn patch(&self, patch: SettingsPatch) -> Result<SettingsSnapshot, AppError> {
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        let settings = AppSettings {
            general: patch
                .general
                .unwrap_or_else(|| state.settings.general.clone()),
            appearance: patch
                .appearance
                .unwrap_or_else(|| state.settings.appearance.clone()),
            security: patch
                .security
                .unwrap_or_else(|| state.settings.security.clone()),
            layout: patch
                .layout
                .map(|layout| layout.merge(&state.settings.layout))
                .unwrap_or_else(|| state.settings.layout.clone()),
            notifications: patch
                .notifications
                .unwrap_or_else(|| state.settings.notifications.clone()),
            compose: patch
                .compose
                .unwrap_or_else(|| state.settings.compose.clone()),
            mcp: state.settings.mcp,
            translation: patch
                .translation
                .unwrap_or_else(|| state.settings.translation.clone()),
        }
        .normalized()?;
        write_file(&self.directory, &settings)?;
        *state = State {
            revision: state.revision.saturating_add(1),
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot(self.system_locale))
    }

    /// Remembers the native close prompt's choice without replacing a general
    /// settings section captured before another window's settings edit.
    pub fn set_close_behavior(
        &self,
        close_behavior: CloseBehavior,
    ) -> Result<SettingsSnapshot, AppError> {
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        let mut settings = state.settings.clone();
        settings.general.close_behavior = close_behavior;
        write_file(&self.directory, &settings)?;
        *state = State {
            revision: state.revision.saturating_add(1),
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot(self.system_locale))
    }

    /// Turns local MCP access on or off. This is the only writer of
    /// `mcp.enabled`; `patch` and `update` keep the stored value.
    pub fn set_mcp_enabled(&self, enabled: bool) -> Result<SettingsSnapshot, AppError> {
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        let mut settings = state.settings.clone();
        settings.mcp.enabled = enabled;
        write_file(&self.directory, &settings)?;
        *state = State {
            revision: state.revision.saturating_add(1),
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot(self.system_locale))
    }

    /// Replaces all settings except `mcp`. The file is written atomically
    /// before the in-memory copy changes, so a failed write leaves both untouched.
    pub fn update(&self, settings: AppSettings) -> Result<SettingsSnapshot, AppError> {
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        let settings = AppSettings {
            mcp: state.settings.mcp,
            ..settings
        }
        .normalized()?;
        write_file(&self.directory, &settings)?;
        *state = State {
            revision: state.revision.saturating_add(1),
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot(self.system_locale))
    }
}

fn read_file(path: &Path, metadata: &fs::Metadata) -> Option<AppSettings> {
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let mut json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(file) = json.as_object_mut()
        && !file.contains_key("settings")
    {
        file.insert("settings".into(), serde_json::json!({}));
    }
    // Existing settings files predate language selection. Preserve their UI
    // language; invalid language fields recover independently of other settings.
    if let Some(settings) = json
        .get_mut("settings")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(translation) = settings.get_mut("translation") {
            if translation.is_object() {
                repair_translation(translation);
            } else {
                settings.remove("translation");
            }
        }
        match settings.get_mut("general") {
            None => {
                settings.insert("general".into(), serde_json::json!({"language": "zh-CN"}));
            }
            Some(general) => {
                let language = general.get("language").and_then(serde_json::Value::as_str);
                let value = match language {
                    None if general.is_object() && general.get("language").is_none() => "zh-CN",
                    Some(value) if valid_language(value) => value,
                    _ => "system",
                }
                .to_owned();
                // Only the language is normalized; other fields keep their
                // stored value and fall back to their own default if invalid.
                if !general.is_object() {
                    *general = serde_json::json!({});
                }
                if let Some(general) = general.as_object_mut() {
                    general.insert("language".into(), value.into());
                    // The old switch controlled checks. Preserve an explicit opt-out
                    // as manual downloads while checks become unconditional.
                    let legacy_auto_update = general.remove("autoCheckUpdates");
                    if !general.contains_key("autoUpdate")
                        && let Some(value) = legacy_auto_update
                    {
                        general.insert("autoUpdate".into(), value);
                    }
                    for key in ["autoUpdate", "showMenuBarIcon", "silentStart"] {
                        if general.get(key).is_some_and(|value| !value.is_boolean()) {
                            general.remove(key);
                        }
                    }
                    if general.get("closeBehavior").is_some_and(|value| {
                        serde_json::from_value::<CloseBehavior>(value.clone()).is_err()
                    }) {
                        general.remove("closeBehavior");
                    }
                }
            }
        }
    }
    let file: SettingsFile = serde_json::from_value(json).ok()?;
    (file.version == CURRENT_VERSION).then(|| file.settings.sanitized())
}

fn write_file(directory: &Path, settings: &AppSettings) -> Result<(), AppError> {
    fs::create_dir_all(directory).map_err(|_| storage_error())?;
    let mut text = serde_json::to_string_pretty(&SettingsFileRef {
        version: CURRENT_VERSION,
        settings,
    })
    .map_err(|_| AppError::new(ErrorCode::Internal))?;
    text.push('\n');
    let temporary = directory.join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| storage_error())?;
        file.write_all(text.as_bytes())
            .map_err(|_| storage_error())?;
        file.sync_all().map_err(|_| storage_error())?;
        drop(file);
        fs::rename(&temporary, directory.join(SETTINGS_FILE_NAME)).map_err(|_| storage_error())?;
        sync_directory(directory);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Best effort: the rename is already durable on most filesystems.
fn sync_directory(directory: &Path) {
    if cfg!(unix) {
        let _ = File::open(directory).and_then(|directory| directory.sync_all());
    }
}

#[cfg(test)]
mod tests;
