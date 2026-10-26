use super::*;
use tempfile::TempDir;

fn appearance(accent_color: &str, interface_font: &str) -> AppSettings {
    AppSettings {
        general: GeneralSettings::default(),
        security: SecuritySettings::default(),
        layout: LayoutSettings::default(),
        notifications: NotificationSettings::default(),
        compose: ComposeSettings::default(),
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
fn language_selection_resolves_with_the_host_and_preserves_old_files() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open_with_locale(directory.path(), SupportedLocale::En).unwrap();
    let fresh = store.get().unwrap();
    assert_eq!(fresh.settings.general.language, "system");
    assert_eq!(fresh.effective_locale, SupportedLocale::En);
    assert_eq!(fresh.revision, 0);
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"appearance":{"theme":"dark"}}}"#,
    )
    .unwrap();
    let legacy = SettingsStore::open_with_locale(directory.path(), SupportedLocale::En)
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(legacy.settings.general.language, "zh-CN");
    assert_eq!(legacy.effective_locale, SupportedLocale::ZhCn);
    assert_eq!(legacy.settings.appearance.theme, ThemeMode::Dark);
    assert!(!legacy.recovered);
}

#[test]
fn invalid_saved_language_recovers_only_that_field() {
    for invalid in [
        serde_json::json!("fr"),
        serde_json::json!(17),
        serde_json::Value::Null,
    ] {
        let directory = TempDir::new().unwrap();
        let value = serde_json::json!({"version":1,"settings":{"general":{"language":invalid},"appearance":{"theme":"dark"}}});
        fs::write(directory.path().join(SETTINGS_FILE_NAME), value.to_string()).unwrap();
        let snapshot = SettingsStore::open_with_locale(directory.path(), SupportedLocale::ZhCn)
            .unwrap()
            .get()
            .unwrap();
        assert_eq!(snapshot.settings.general.language, "system");
        assert_eq!(snapshot.effective_locale, SupportedLocale::ZhCn);
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
        assert!(!snapshot.recovered);
    }
}

#[test]
fn malformed_ipc_language_reports_the_specific_field() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    for invalid in [
        serde_json::json!("fr"),
        serde_json::json!(17),
        serde_json::Value::Null,
    ] {
        let patch: SettingsPatch =
            serde_json::from_value(serde_json::json!({"general":{"language":invalid}})).unwrap();
        let error = store.patch(patch).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some("general.language"));
    }
    assert_eq!(store.get().unwrap().revision, 0);
}

#[test]
fn language_commit_is_versioned_and_invalid_input_leaves_it_untouched() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open_with_locale(directory.path(), SupportedLocale::ZhCn).unwrap();
    let selected = store
        .patch(SettingsPatch {
            general: Some(GeneralSettings {
                language: "en".into(),
                ..GeneralSettings::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(selected.revision, 1);
    assert_eq!(selected.effective_locale, SupportedLocale::En);
    let error = store
        .patch(SettingsPatch {
            general: Some(GeneralSettings {
                language: "xx".into(),
                ..GeneralSettings::default()
            }),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.field.as_deref(), Some("general.language"));
    assert_eq!(store.get().unwrap(), selected);
    let edited = store
        .patch(SettingsPatch {
            appearance: Some(AppearanceSettings {
                theme: ThemeMode::Dark,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(edited.revision, 2);
    assert_eq!(edited.settings.general.language, "en");
    let persisted: serde_json::Value =
        serde_json::from_str(&file_text(&directory, SETTINGS_FILE_NAME)).unwrap();
    assert!(persisted.get("revision").is_none());
    assert!(persisted.get("effectiveLocale").is_none());
    assert_eq!(
        SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn failed_disk_commit_does_not_publish_a_new_language_or_revision() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open_with_locale(directory.path(), SupportedLocale::ZhCn).unwrap();
    let before = store.get().unwrap();
    fs::create_dir(directory.path().join(SETTINGS_FILE_NAME)).unwrap();
    assert!(
        store
            .patch(SettingsPatch {
                general: Some(GeneralSettings {
                    language: "en".into(),
                    ..GeneralSettings::default()
                }),
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(store.get().unwrap(), before);
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
    // Settings saved before the option existed get the one-line default.
    assert_eq!(appearance.preview_lines, PreviewLines::One);
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
                "general": { "language": "system", "autoCheckUpdates": true },
                "appearance": {
                    "theme": "system",
                    "accentColor": "",
                    "interfaceFont": "",
                    "mailFont": "",
                    "mailTheme": "follow",
                    "fontSize": "default",
                    "previewLines": "one"
                },
                "security": { "remoteImages": "ask", "senderAvatars": false },
                "layout": { "navWidth": 256, "listWidth": 420, "readerCollapsed": false },
                "notifications": {
                    "enabled": true,
                    "showPreview": true,
                    "whenFocused": false,
                    "sound": "system",
                    "mutedAccounts": []
                },
                "compose": { "defaultAccountId": "", "undoSendSeconds": 0 }
            },
            "stored": false,
            "recovered": false,
            "effectiveLocale": "en",
            "revision": 0
        })
    );
}

#[test]
fn older_settings_default_to_ask_and_section_patches_preserve_other_settings() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"appearance":{"theme":"dark"}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(
        store.get().unwrap().settings.security.remote_images,
        RemoteImagePolicy::Ask
    );
    store
        .patch(SettingsPatch {
            security: Some(SecuritySettings {
                remote_images: RemoteImagePolicy::Automatic,
                sender_avatars: true,
            }),
            ..Default::default()
        })
        .unwrap();
    store
        .patch(SettingsPatch {
            appearance: Some(AppearanceSettings {
                font_size: FontSize::Large,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    let restored = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(
        restored.settings.security.remote_images,
        RemoteImagePolicy::Automatic
    );
    assert_eq!(restored.settings.appearance.font_size, FontSize::Large);
}

#[test]
fn mail_body_css_uses_defaults_user_fonts_and_font_size() {
    let default = mail_body_css(&AppearanceSettings::default());
    assert!(default.contains("font-family:-apple-system"), "{default}");
    assert!(default.contains("font-size:14px"));
    assert!(default.contains("overflow-wrap:break-word"));
    assert!(!default.contains("anywhere"));

    let custom = mail_body_css(&AppearanceSettings {
        mail_font: "LXGW WenKai, Georgia".into(),
        font_size: FontSize::Large,
        ..AppearanceSettings::default()
    });
    assert!(
        custom.contains("font-family:\"LXGW WenKai\", \"Georgia\", sans-serif;font-size:16px"),
        "{custom}"
    );

    let generic = mail_body_css(&AppearanceSettings {
        mail_font: "Georgia, serif".into(),
        ..AppearanceSettings::default()
    });
    assert!(
        generic.contains("font-family:\"Georgia\", serif;"),
        "{generic}"
    );
}

#[test]
fn mail_font_is_validated_like_the_interface_font() {
    let mut settings = AppSettings::default();
    settings.appearance.mail_font = " 'Noto Serif SC' , , Georgia".into();
    assert_eq!(
        settings.clone().normalized().unwrap().appearance.mail_font,
        "Noto Serif SC, Georgia"
    );
    settings.appearance.mail_font = "Bad;Font".into();
    assert!(settings.normalized().is_err());
}

#[test]
fn nav_width_defaults_persists_and_is_validated() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(store.get().unwrap().settings.layout.nav_width, 256);

    let saved = store
        .patch(SettingsPatch {
            layout: Some(LayoutSettings {
                nav_width: 300,
                ..LayoutSettings::default()
            }),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(saved.settings.layout.nav_width, 300);
    assert!(file_text(&directory, SETTINGS_FILE_NAME).contains("\"navWidth\": 300"));
    assert_eq!(
        SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap()
            .settings
            .layout
            .nav_width,
        300
    );

    for width in [0, 219, 421] {
        let error = store
            .patch(SettingsPatch {
                layout: Some(LayoutSettings {
                    nav_width: width,
                    ..LayoutSettings::default()
                }),
                ..SettingsPatch::default()
            })
            .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("layout.navWidth"));
    }
    assert_eq!(store.get().unwrap().settings.layout.nav_width, 300);
}

#[test]
fn hand_edited_nav_width_is_clamped_on_load() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"layout":{"navWidth":9999}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(store.get().unwrap().settings.layout.nav_width, 420);
}

#[test]
fn notifications_default_on_and_muted_accounts_are_cleaned_or_rejected() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"notifications":{"sound":"chime","mutedAccounts":["b"," a ","b",""]}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let loaded = store.get().unwrap().settings.notifications;
    assert!(loaded.enabled && loaded.show_preview && !loaded.when_focused);
    assert_eq!(loaded.sound, NotificationSound::Chime);
    // A hand-edited list with a blank entry is unusable as a whole: nothing muted.
    assert!(loaded.muted_accounts.is_empty());

    let saved = store
        .patch(SettingsPatch {
            notifications: Some(NotificationSettings {
                muted_accounts: vec!["b".into(), " a ".into(), "b".into()],
                when_focused: true,
                ..NotificationSettings::default()
            }),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(saved.settings.notifications.muted_accounts, ["a", "b"]);
    assert!(saved.settings.notifications.when_focused);

    let error = store
        .patch(SettingsPatch {
            notifications: Some(NotificationSettings {
                muted_accounts: vec!["bad\nid".into()],
                ..NotificationSettings::default()
            }),
            ..SettingsPatch::default()
        })
        .unwrap_err();
    assert_eq!(error.field.as_deref(), Some("notifications.mutedAccounts"));
    assert_eq!(
        store.get().unwrap().settings.notifications.muted_accounts,
        ["a", "b"]
    );
}

#[test]
fn list_width_defaults_persists_alongside_nav_width_and_is_validated() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(store.get().unwrap().settings.layout.list_width, 420);

    store
        .patch(SettingsPatch {
            layout: Some(LayoutSettings {
                nav_width: 280,
                list_width: 520,
                reader_collapsed: true,
            }),
            ..SettingsPatch::default()
        })
        .unwrap();
    let reloaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap()
        .settings
        .layout;
    assert_eq!((reloaded.nav_width, reloaded.list_width), (280, 520));
    // The folded reader is restored too.
    assert!(reloaded.reader_collapsed);
    assert!(file_text(&directory, SETTINGS_FILE_NAME).contains("\"listWidth\": 520"));

    for width in [0, 299, 2001] {
        let error = store
            .patch(SettingsPatch {
                layout: Some(LayoutSettings {
                    nav_width: 280,
                    list_width: width,
                    reader_collapsed: true,
                }),
                ..SettingsPatch::default()
            })
            .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("layout.listWidth"));
    }
    assert_eq!(store.get().unwrap().settings.layout.list_width, 520);
}

#[test]
fn settings_saved_before_the_list_width_existed_get_the_default_and_wild_values_are_clamped() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"layout":{"navWidth":300}}}"#,
    )
    .unwrap();
    let layout = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap()
        .settings
        .layout;
    assert_eq!((layout.nav_width, layout.list_width), (300, 420));
    // Settings from before the option existed start with the reader shown.
    assert!(!layout.reader_collapsed);

    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"layout":{"listWidth":99999}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(store.get().unwrap().settings.layout.list_width, 2000);
}

#[test]
fn compose_defaults_apply_to_old_files_and_patches_keep_other_sections() {
    let directory = TempDir::new().unwrap();
    assert_eq!(
        ComposeSettings::default(),
        ComposeSettings {
            default_account_id: String::new(),
            undo_send_seconds: 0
        }
    );
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"appearance":{"theme":"dark"}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(
        store.get().unwrap().settings.compose,
        ComposeSettings::default()
    );

    let compose = ComposeSettings {
        default_account_id: "  account-1 ".into(),
        undo_send_seconds: 20,
    };
    let saved = store
        .patch(SettingsPatch {
            compose: Some(compose),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(saved.settings.compose.default_account_id, "account-1");
    assert_eq!(saved.settings.compose.undo_send_seconds, 20);
    assert_eq!(saved.settings.appearance.theme, ThemeMode::Dark);

    store
        .patch(SettingsPatch {
            layout: Some(LayoutSettings::default()),
            ..Default::default()
        })
        .unwrap();
    let reloaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(reloaded.settings.compose.undo_send_seconds, 20);
    assert_eq!(reloaded.settings.compose.default_account_id, "account-1");
}

#[test]
fn invalid_compose_patches_are_rejected_and_change_nothing() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    for seconds in [1, 4, 6, 15, 31, 60, u32::MAX] {
        let error = store
            .patch(SettingsPatch {
                compose: Some(ComposeSettings {
                    undo_send_seconds: seconds,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some("compose.undoSendSeconds"));
    }
    for seconds in UNDO_SEND_SECONDS {
        store
            .patch(SettingsPatch {
                compose: Some(ComposeSettings {
                    undo_send_seconds: seconds,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();
    }
    let error = store
        .patch(SettingsPatch {
            compose: Some(ComposeSettings {
                default_account_id: "a\nb".into(),
                undo_send_seconds: 5,
            }),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.field.as_deref(), Some("compose.defaultAccountId"));
    assert_eq!(
        store.get().unwrap().settings.compose.undo_send_seconds,
        UNDO_SEND_SECONDS[4]
    );
}

#[test]
fn hand_edited_invalid_compose_values_fall_back_to_defaults() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"compose":{"defaultAccountId":"keep-me","undoSendSeconds":7}}}"#,
    )
    .unwrap();
    let compose = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap()
        .settings
        .compose;
    assert_eq!(compose.undo_send_seconds, 0);
    assert_eq!(compose.default_account_id, "keep-me");
}

#[test]
fn update_check_preference_defaults_on_and_survives_language_recovery() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join(SETTINGS_FILE_NAME);
    fs::write(
        &path,
        r#"{"version":1,"settings":{"appearance":{"theme":"dark"}}}"#,
    )
    .unwrap();
    let legacy = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(legacy.settings.general.auto_check_updates);

    for general in [
        r#"{"language":"zh-CN","autoCheckUpdates":false}"#,
        r#"{"language":"xx","autoCheckUpdates":false}"#,
        r#"{"autoCheckUpdates":false}"#,
    ] {
        let text = format!(r#"{{"version":1,"settings":{{"general":{general}}}}}"#);
        fs::write(&path, text).unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(!snapshot.settings.general.auto_check_updates, "{general}");
        assert!(!snapshot.recovered);
    }

    fs::write(
        &path,
        r#"{"version":1,"settings":{"general":{"language":"en","autoCheckUpdates":"no"}}}"#,
    )
    .unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(snapshot.settings.general.language, "en");
    assert!(snapshot.settings.general.auto_check_updates);
}
