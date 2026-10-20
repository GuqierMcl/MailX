//! Application preferences stored as `settings.json` in the data directory.
//!
//! The file is meant to be readable and hand-editable, so it is pretty-printed
//! JSON. Every field has a default; an unreadable or newer-version file is moved
//! aside to `settings.json.bak` instead of being overwritten in place. Settings
//! never hold secrets and do not depend on account storage being open.

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
}

/// Section updates merge under the store lock so simultaneous saves cannot
/// overwrite another settings page with a stale frontend snapshot.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub appearance: Option<AppearanceSettings>,
    pub security: Option<SecuritySettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub security: SecuritySettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub settings: AppSettings,
    /// False until a settings file exists on disk.
    pub stored: bool,
    /// The previous file could not be used and was moved to `settings.json.bak`.
    pub recovered: bool,
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
        let appearance = self.appearance;
        Ok(Self {
            security: self.security,
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
        Self {
            security: self.security,
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
}

impl State {
    fn snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot {
            settings: self.settings.clone(),
            stored: self.stored,
            recovered: self.recovered,
        }
    }
}

pub struct SettingsStore {
    directory: PathBuf,
    state: Mutex<State>,
}

fn storage_error() -> AppError {
    AppError::new(ErrorCode::StorageUnavailable)
}

impl SettingsStore {
    /// Loads settings without creating the directory or file. Only I/O errors
    /// other than a missing file fail; unusable content is moved aside.
    pub fn open(data_directory: &Path) -> Result<Self, AppError> {
        let path = data_directory.join(SETTINGS_FILE_NAME);
        let state = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => State {
                settings: AppSettings::default(),
                stored: false,
                recovered: false,
            },
            Err(_) => return Err(storage_error()),
            Ok(metadata) => match read_file(&path, &metadata) {
                Some(settings) => State {
                    settings,
                    stored: true,
                    recovered: false,
                },
                None => {
                    fs::rename(&path, data_directory.join(BACKUP_FILE_NAME))
                        .map_err(|_| storage_error())?;
                    State {
                        settings: AppSettings::default(),
                        stored: false,
                        recovered: true,
                    }
                }
            },
        };
        Ok(Self {
            directory: data_directory.to_owned(),
            state: Mutex::new(state),
        })
    }

    pub fn get(&self) -> Result<SettingsSnapshot, AppError> {
        Ok(self.state.lock().map_err(|_| storage_error())?.snapshot())
    }

    pub fn patch(&self, patch: SettingsPatch) -> Result<SettingsSnapshot, AppError> {
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        let settings = AppSettings {
            appearance: patch
                .appearance
                .unwrap_or_else(|| state.settings.appearance.clone()),
            security: patch
                .security
                .unwrap_or_else(|| state.settings.security.clone()),
        }
        .normalized()?;
        write_file(&self.directory, &settings)?;
        *state = State {
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot())
    }

    /// Replaces all settings. The file is written atomically before the
    /// in-memory copy changes, so a failed write leaves both untouched.
    pub fn update(&self, settings: AppSettings) -> Result<SettingsSnapshot, AppError> {
        let settings = settings.normalized()?;
        let mut state = self.state.lock().map_err(|_| storage_error())?;
        write_file(&self.directory, &settings)?;
        *state = State {
            settings,
            stored: true,
            recovered: false,
        };
        Ok(state.snapshot())
    }
}

fn read_file(path: &Path, metadata: &fs::Metadata) -> Option<AppSettings> {
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let file: SettingsFile = serde_json::from_str(&text).ok()?;
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
