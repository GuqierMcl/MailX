use super::*;
use tempfile::TempDir;

fn appearance(accent_color: &str, interface_font: &str) -> AppSettings {
    AppSettings {
        appearance: AppearanceSettings {
            accent_color: accent_color.into(),
            interface_font: interface_font.into(),
            ..AppearanceSettings::default()
        },
    }
}

fn file_text(directory: &TempDir, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

#[test]
fn missing_file_yields_defaults_without_creating_anything() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();

    let snapshot = store.get().unwrap();
    assert_eq!(snapshot.settings, AppSettings::default());
    assert!(!snapshot.stored);
    assert!(!snapshot.recovered);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn update_persists_readable_json_that_reloads() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let mut settings = appearance("#3B82F6", "Inter, 'PingFang SC'");
    settings.appearance.theme = ThemeMode::Dark;
    settings.appearance.font_size = FontSize::Large;

    let snapshot = store.update(settings).unwrap();
    assert!(snapshot.stored);
    assert_eq!(snapshot.settings.appearance.accent_color, "#3b82f6");
    assert_eq!(
        snapshot.settings.appearance.interface_font,
        "Inter, PingFang SC"
    );

    let text = file_text(&directory, SETTINGS_FILE_NAME);
    assert!(text.contains("\"version\": 1"));
    assert!(text.contains("\"theme\": \"dark\""));
    assert!(text.ends_with('\n'));
    let reloaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(reloaded.settings, snapshot.settings);
    assert!(reloaded.stored);
    // No temporary file is left behind.
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn update_creates_a_missing_data_directory() {
    let directory = TempDir::new().unwrap();
    let nested = directory.path().join("not-yet-created");
    let store = SettingsStore::open(&nested).unwrap();

    store.update(AppSettings::default()).unwrap();
    assert!(nested.join(SETTINGS_FILE_NAME).is_file());
}

#[test]
fn font_families_are_split_trimmed_and_unquoted_in_order() {
    for (input, expected) in [
        ("", ""),
        ("   ", ""),
        (" , ,", ""),
        ("Inter", "Inter"),
        (
            " \"SF Pro Text\" ,  'PingFang SC',, system-ui ",
            "SF Pro Text, PingFang SC, system-ui",
        ),
        ("思源黑体, sans-serif", "思源黑体, sans-serif"),
    ] {
        assert_eq!(
            normalize_font_families(input).as_deref(),
            Some(expected),
            "{input:?}"
        );
    }
}

#[test]
fn font_families_that_could_escape_css_are_rejected() {
    let too_many = vec!["A"; MAX_FONT_FAMILIES + 1].join(",");
    let too_long = "A".repeat(MAX_FONT_FAMILY_CHARS + 1);
    for input in [
        "Inter\"; color: red",
        "a}body{display:none",
        "url(x)",
        "back\\slash",
        "line\nbreak",
        "<style>",
        "'unbalanced",
        too_many.as_str(),
        too_long.as_str(),
    ] {
        assert_eq!(normalize_font_families(input), None, "{input:?}");
    }
}

#[test]
fn invalid_updates_are_rejected_with_the_field_and_change_nothing() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();

    for (settings, field) in [
        (appearance("blue", ""), "appearance.accentColor"),
        (appearance("#12345", ""), "appearance.accentColor"),
        (appearance("", "Inter; x"), "appearance.interfaceFont"),
    ] {
        let error = store.update(settings).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some(field));
    }
    assert!(!store.get().unwrap().stored);
    assert!(!directory.path().join(SETTINGS_FILE_NAME).exists());
}

#[test]
fn unreadable_files_are_moved_aside_and_defaults_are_used() {
    for content in [
        "{ not json",
        r#"{"version": 99, "settings": {}}"#,
        r#"{"settings": {}}"#,
        r#"{"version": 1, "settings": {"appearance": {"theme": "sepia"}}}"#,
    ] {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join(SETTINGS_FILE_NAME), content).unwrap();

        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert_eq!(snapshot.settings, AppSettings::default(), "{content}");
        assert!(snapshot.recovered);
        assert!(!snapshot.stored);
        assert_eq!(file_text(&directory, BACKUP_FILE_NAME), content);
        assert!(!directory.path().join(SETTINGS_FILE_NAME).exists());

        // The next save starts a clean file and clears the notice.
        let saved = store.update(AppSettings::default()).unwrap();
        assert!(!saved.recovered);
        assert_eq!(file_text(&directory, BACKUP_FILE_NAME), content);
    }
}

#[test]
fn hand_edited_files_keep_valid_fields_and_reset_invalid_strings() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r##"{
  "version": 1,
  "settings": {
    "appearance": {
      "theme": "light",
      "accentColor": "#GGGGGG",
      "interfaceFont": "  'Noto Sans SC' , serif",
      "futureOption": true
    },
    "futureGroup": {}
  }
}"##,
    )
    .unwrap();

    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(!snapshot.recovered);
    let appearance = snapshot.settings.appearance;
    assert_eq!(appearance.theme, ThemeMode::Light);
    assert_eq!(appearance.accent_color, "");
    assert_eq!(appearance.interface_font, "Noto Sans SC, serif");
    assert_eq!(appearance.font_size, FontSize::Default);
}

#[test]
fn snapshot_serializes_as_camel_case_for_the_frontend() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let value = serde_json::to_value(store.get().unwrap()).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "settings": {
                "appearance": {
                    "theme": "system",
                    "accentColor": "",
                    "interfaceFont": "",
                    "fontSize": "default"
                }
            },
            "stored": false,
            "recovered": false
        })
    );
}
