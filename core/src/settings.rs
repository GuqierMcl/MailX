//! Application preferences stored as `settings.json` in the data directory.
//!
//! The file is meant to be readable and hand-editable, so it is pretty-printed
//! JSON. Every field has a default; an unreadable or newer-version file is moved
//! aside to `settings.json.bak` instead of being overwritten in place. Settings
//! never hold secrets and do not depend on account storage being open.

use crate::SupportedLocale;
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

/// Window layout preferences, in CSS pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct LayoutSettings {
    /// Width of the navigation sidebar, `220..=420`.
    pub nav_width: u32,
    /// Width of the message list beside the reader, `300..=2000`, applied when
    /// the window opens. In a window that is too narrow for it, list and reader
    /// shrink together, each down to its own minimum.
    pub list_width: u32,
    /// The reader beside the message list is folded away. Unlike the sidebar,
    /// this survives a restart: the window comes back as it was left.
    pub reader_collapsed: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            nav_width: NAV_WIDTH_DEFAULT,
            list_width: LIST_WIDTH_DEFAULT,
            reader_collapsed: false,
        }
    }
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

impl ComposeSettings {
    /// Trimmed account id; `None` for input that is not one.
    fn normalized_account(&self) -> Option<String> {
        let id = self.default_account_id.trim();
        (id.chars().count() <= MAX_ACCOUNT_ID_CHARS && !id.chars().any(char::is_control))
            .then(|| id.to_owned())
    }
}

/// Language preference is validated on input; a string lets field validation
/// report `general.language` rather than a generic deserialization error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralSettings {
    #[ts(type = "LanguagePreference")]
    #[serde(deserialize_with = "deserialize_language")]
    pub language: String,
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
    pub general: Option<GeneralSettings>,
    pub appearance: Option<AppearanceSettings>,
    pub security: Option<SecuritySettings>,
    pub layout: Option<LayoutSettings>,
    pub notifications: Option<NotificationSettings>,
    pub compose: Option<ComposeSettings>,
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
            layout: self.layout,
            notifications,
            compose,
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
                GeneralSettings::default()
            },
            security: self.security,
            notifications,
            compose,
            layout: LayoutSettings {
                nav_width: self.layout.nav_width.clamp(NAV_WIDTH_MIN, NAV_WIDTH_MAX),
                list_width: self.layout.list_width.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX),
                reader_collapsed: self.layout.reader_collapsed,
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
        "html{{background:white;color:black}}body{{margin:0;{font};font-size:{size};line-height:1.5}}.mailx-page{{box-sizing:border-box;min-height:100vh;padding:16px;overflow-wrap:break-word}}.mailx-body{{box-sizing:border-box;min-height:100vh;margin:-16px;padding:16px}}img{{max-width:100%;height:auto}}"
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
                .unwrap_or_else(|| state.settings.layout.clone()),
            notifications: patch
                .notifications
                .unwrap_or_else(|| state.settings.notifications.clone()),
            compose: patch
                .compose
                .unwrap_or_else(|| state.settings.compose.clone()),
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

    /// Replaces all settings. The file is written atomically before the
    /// in-memory copy changes, so a failed write leaves both untouched.
    pub fn update(&self, settings: AppSettings) -> Result<SettingsSnapshot, AppError> {
        let settings = settings.normalized()?;
        let mut state = self.state.lock().map_err(|_| storage_error())?;
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
                *general = serde_json::json!({"language": value});
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
