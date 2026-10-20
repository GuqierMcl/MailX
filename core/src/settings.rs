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

/// Empty strings mean "use the built-in default".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceSettings {
    pub theme: ThemeMode,
    /// `""` or a lowercase `#rrggbb` color.
    pub accent_color: String,
    /// `""` or comma-separated font families, tried in order.
    pub interface_font: String,
    pub font_size: FontSize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
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
            appearance: AppearanceSettings {
                accent_color: normalize_accent_color(&appearance.accent_color)
                    .ok_or_else(|| AppError::validation("appearance.accentColor"))?,
                interface_font: normalize_font_families(&appearance.interface_font)
                    .ok_or_else(|| AppError::validation("appearance.interfaceFont"))?,
                ..appearance
            },
        })
    }

    /// Hand-edited files fall back per field instead of discarding everything.
    fn sanitized(self) -> Self {
        let defaults = AppearanceSettings::default();
        let appearance = self.appearance;
        Self {
            appearance: AppearanceSettings {
                accent_color: normalize_accent_color(&appearance.accent_color)
                    .unwrap_or(defaults.accent_color),
                interface_font: normalize_font_families(&appearance.interface_font)
                    .unwrap_or(defaults.interface_font),
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
