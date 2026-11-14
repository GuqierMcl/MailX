use super::*;
use tempfile::TempDir;

fn appearance(accent_color: &str, interface_font: &str) -> AppSettings {
    AppSettings {
        general: GeneralSettings::default(),
        mail: MailSettings::default(),
        security: SecuritySettings::default(),
        layout: LayoutSettings::default(),
        notifications: NotificationSettings::default(),
        mcp: McpSettings::default(),
        compose: ComposeSettings::default(),
        translation: TranslationSettings::default(),
        ai: AiSettings::default(),
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
    assert_eq!(appearance.mailbox_label, MailboxLabel::Name);
}

#[test]
fn mailbox_label_persists_and_a_document_without_it_loads_as_name() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    store
        .patch(SettingsPatch {
            appearance: Some(AppearanceSettings {
                mailbox_label: MailboxLabel::Icon,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    let text = file_text(&directory, SETTINGS_FILE_NAME);
    assert!(text.contains("\"mailboxLabel\": \"icon\""));
    let restored = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(
        restored.settings.appearance.mailbox_label,
        MailboxLabel::Icon
    );

    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{ "appearance": { "theme": "dark" } }"#,
    )
    .unwrap();
    let legacy = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(legacy.settings.appearance.mailbox_label, MailboxLabel::Name);
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
                "ai": { "connections": [], "defaultModel": null, "translationModel": null, "route": { "kind": "direct" } },
                "general": { "language": "system", "autoUpdate": true, "closeBehavior": "ask", "showMenuBarIcon": true, "silentStart": false, "singleKeyShortcuts": true, "startupView": "inbox" },
                "mail": { "trashVerificationMailAfterCopy": false },
                "appearance": {
                    "theme": "system",
                    "accentColor": "",
                    "interfaceFont": "",
                    "mailFont": "",
                    "mailTheme": "follow",
                    "fontSize": "default",
                    "previewLines": "one",
                    "mailboxLabel": "name"
                },
                "security": { "remoteImages": "ask", "senderAvatars": false },
                "layout": { "navWidth": 256, "navCollapsed": false, "listWidth": 420, "readerCollapsed": false, "accountOrder": [] },
                "notifications": {
                    "enabled": true,
                    "showPreview": true,
                    "whenFocused": false,
                    "sound": "system",
                    "mutedAccounts": []
                },
                "compose": { "defaultAccountId": "", "undoSendSeconds": 0 },
                "mcp": { "enabled": false },
                "translation": {
                    "provider": "microsoftFree",
                    "microsoftRegion": "",
                    "route": { "kind": "direct" },
                    "targetLanguage": "auto",
                    "suggest": true,
                    "neverSuggest": [],
                    "alwaysTranslate": [],
                    "includeQuotes": false,
                    "consentedProvider": null,
                    "consentedAi": null
                }
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
            layout: Some(LayoutSettingsPatch {
                nav_width: Some(300),
                ..LayoutSettingsPatch::default()
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
                layout: Some(LayoutSettingsPatch {
                    nav_width: Some(width),
                    ..LayoutSettingsPatch::default()
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
            layout: Some(LayoutSettingsPatch {
                nav_width: Some(280),
                nav_collapsed: Some(true),
                list_width: Some(520),
                reader_collapsed: Some(true),
                ..LayoutSettingsPatch::default()
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
    // The folded reader and the sidebar's icon rail are restored too.
    assert!(reloaded.reader_collapsed);
    assert!(reloaded.nav_collapsed);
    assert!(file_text(&directory, SETTINGS_FILE_NAME).contains("\"listWidth\": 520"));

    for width in [0, 299, 2001] {
        let error = store
            .patch(SettingsPatch {
                layout: Some(LayoutSettingsPatch {
                    nav_width: Some(280),
                    list_width: Some(width),
                    reader_collapsed: Some(true),
                    ..LayoutSettingsPatch::default()
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
    // Settings from before the options existed start with the reader and the full sidebar shown.
    assert!(!layout.reader_collapsed);
    assert!(!layout.nav_collapsed);
    assert!(layout.account_order.is_empty());

    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"layout":{"listWidth":99999}}}"#,
    )
    .unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(store.get().unwrap().settings.layout.list_width, 2000);
}

#[test]
fn account_order_persists_canonical_ids_without_sorting() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let first = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let second = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let patch = serde_json::from_value(serde_json::json!({
        "layout": { "accountOrder": [format!("  {}  ", first.to_uppercase()), second, first] }
    }))
    .unwrap();
    let saved = store.patch(patch).unwrap();
    assert_eq!(saved.settings.layout.account_order, [first, second]);
    assert_eq!(saved.settings.layout.nav_width, NAV_WIDTH_DEFAULT);
    let reloaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(reloaded.settings, saved.settings);
    assert!(file_text(&directory, SETTINGS_FILE_NAME).contains("\"accountOrder\": ["));

    // An empty array intentionally restores the underlying account-list order.
    let cleared = store
        .patch(
            serde_json::from_value(serde_json::json!({ "layout": { "accountOrder": [] } }))
                .unwrap(),
        )
        .unwrap();
    assert!(cleared.settings.layout.account_order.is_empty());
}

#[test]
fn invalid_account_order_reports_its_field_without_changing_settings_or_disk() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let before = store
        .patch(
            serde_json::from_value(serde_json::json!({ "layout": { "accountOrder": [id] } }))
                .unwrap(),
        )
        .unwrap();
    let before_text = file_text(&directory, SETTINGS_FILE_NAME);
    for invalid in [
        serde_json::json!(["not-a-uuid"]),
        serde_json::json!(["  "]),
        serde_json::json!([id, 17]),
        serde_json::json!("not-an-array"),
        serde_json::json!({}),
        serde_json::Value::Null,
        serde_json::json!(vec![id; MAX_ACCOUNT_ORDER + 1]),
    ] {
        let patch = serde_json::from_value(serde_json::json!({
            "layout": { "accountOrder": invalid, "navWidth": 300 }
        }))
        .unwrap();
        let error = store.patch(patch).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field.as_deref(), Some("layout.accountOrder"));
        assert_eq!(store.get().unwrap(), before);
        assert_eq!(file_text(&directory, SETTINGS_FILE_NAME), before_text);
    }
}

#[test]
fn malformed_saved_account_order_recovers_only_that_field() {
    for invalid in [
        serde_json::json!(["invalid"]),
        serde_json::json!(17),
        serde_json::json!(["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", 17]),
        serde_json::Value::Null,
        serde_json::json!(vec![
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
            MAX_ACCOUNT_ORDER + 1
        ]),
    ] {
        let directory = TempDir::new().unwrap();
        let value = serde_json::json!({
            "version": 1,
            "settings": {
                "layout": { "navWidth": 300, "listWidth": 530, "readerCollapsed": true, "accountOrder": invalid },
                "appearance": { "theme": "dark" }
            }
        });
        fs::write(directory.path().join(SETTINGS_FILE_NAME), value.to_string()).unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(!snapshot.recovered);
        assert!(snapshot.stored);
        assert!(snapshot.settings.layout.account_order.is_empty());
        assert_eq!(snapshot.settings.layout.nav_width, 300);
        assert_eq!(snapshot.settings.layout.list_width, 530);
        assert!(snapshot.settings.layout.reader_collapsed);
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
    }
}

#[test]
fn maximum_account_order_fits_the_settings_file_and_reloads() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let ids: Vec<String> = (0..MAX_ACCOUNT_ORDER)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    let saved = store
        .patch(SettingsPatch {
            layout: Some(LayoutSettingsPatch {
                account_order: Some(ids.clone()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(saved.settings.layout.account_order, ids);
    assert!((file_text(&directory, SETTINGS_FILE_NAME).len() as u64) < MAX_FILE_BYTES);
    assert_eq!(
        SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap()
            .settings,
        saved.settings
    );
}

#[test]
fn concurrent_layout_fields_merge_and_full_layout_input_still_works() {
    use std::sync::{Arc, Barrier};

    let directory = TempDir::new().unwrap();
    let store = Arc::new(SettingsStore::open(directory.path()).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    std::thread::scope(|scope| {
        let widths_store = Arc::clone(&store);
        let widths_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            widths_barrier.wait();
            widths_store
                .patch(
                    serde_json::from_value(serde_json::json!({
                        "layout": { "navWidth": 300, "listWidth": 530, "readerCollapsed": true }
                    }))
                    .unwrap(),
                )
                .unwrap();
        });
        barrier.wait();
        store
            .patch(
                serde_json::from_value(serde_json::json!({
                    "layout": { "accountOrder": [id] }
                }))
                .unwrap(),
            )
            .unwrap();
    });
    let merged = store.get().unwrap();
    assert_eq!(merged.revision, 2);
    assert_eq!(merged.settings.layout.nav_width, 300);
    assert_eq!(merged.settings.layout.list_width, 530);
    assert!(merged.settings.layout.reader_collapsed);
    assert_eq!(merged.settings.layout.account_order, [id]);

    let full = LayoutSettings {
        nav_width: 280,
        list_width: 600,
        ..Default::default()
    };
    let replaced = store
        .patch(SettingsPatch {
            layout: Some(full.clone().into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(replaced.settings.layout, full);
}

#[test]
fn failed_disk_commit_does_not_publish_account_order_or_other_layout_fields() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let before = store.get().unwrap();
    fs::create_dir(directory.path().join(SETTINGS_FILE_NAME)).unwrap();
    let patch = serde_json::from_value(serde_json::json!({
        "layout": { "accountOrder": ["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"], "navWidth": 300 }
    }))
    .unwrap();
    assert!(store.patch(patch).is_err());
    assert_eq!(store.get().unwrap(), before);
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
            layout: Some(LayoutSettings::default().into()),
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
fn auto_update_defaults_on_and_preserves_legacy_opt_out_during_language_recovery() {
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
    assert!(legacy.settings.general.auto_update);

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
        assert!(!snapshot.settings.general.auto_update, "{general}");
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
    assert!(snapshot.settings.general.auto_update);
}

#[test]
fn auto_update_migration_prefers_the_new_field_and_rewrites_only_the_new_name() {
    for (general, expected) in [
        (serde_json::json!({}), true),
        (serde_json::json!({"autoCheckUpdates": true}), true),
        (serde_json::json!({"autoCheckUpdates": false}), false),
        (serde_json::json!({"autoUpdate": false}), false),
        (
            serde_json::json!({"autoCheckUpdates": false, "autoUpdate": true}),
            true,
        ),
        (
            serde_json::json!({"autoCheckUpdates": true, "autoUpdate": false}),
            false,
        ),
        (
            serde_json::json!({"autoCheckUpdates": false, "autoUpdate": "invalid"}),
            true,
        ),
    ] {
        let directory = TempDir::new().unwrap();
        let mut general = general;
        general["language"] = "en".into();
        general["silentStart"] = true.into();
        general["closeBehavior"] = "background".into();
        let value = serde_json::json!({
            "version": 1,
            "settings": {"general": general, "appearance": {"theme": "dark"}}
        });
        fs::write(directory.path().join(SETTINGS_FILE_NAME), value.to_string()).unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert_eq!(snapshot.settings.general.auto_update, expected, "{general}");
        assert_eq!(snapshot.settings.general.language, "en");
        assert!(snapshot.settings.general.silent_start);
        assert_eq!(
            snapshot.settings.general.close_behavior,
            CloseBehavior::Background
        );
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
        assert!(!snapshot.recovered);

        store.update(snapshot.settings.clone()).unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&file_text(&directory, SETTINGS_FILE_NAME)).unwrap();
        let persisted_general = &persisted["settings"]["general"];
        assert_eq!(persisted_general["autoUpdate"], expected);
        assert!(persisted_general.get("autoCheckUpdates").is_none());
        assert_eq!(
            SettingsStore::open(directory.path())
                .unwrap()
                .get()
                .unwrap()
                .settings,
            snapshot.settings
        );
    }
}

#[test]
fn background_preferences_default_without_changing_legacy_language() {
    let directory = TempDir::new().unwrap();
    for general in [serde_json::json!({}), serde_json::json!({"language": "en"})] {
        let value = serde_json::json!({"version": 1, "settings": {"general": general}});
        fs::write(directory.path().join(SETTINGS_FILE_NAME), value.to_string()).unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert_eq!(snapshot.settings.general.close_behavior, CloseBehavior::Ask);
        assert!(snapshot.settings.general.show_menu_bar_icon);
        assert_eq!(
            snapshot.settings.general.language,
            general
                .get("language")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("zh-CN")
        );
        assert!(!snapshot.recovered);
    }
    let explicit_off = serde_json::json!({
        "version": 1,
        "settings": {"general": {"language": "en", "showMenuBarIcon": false}}
    });
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        explicit_off.to_string(),
    )
    .unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(!snapshot.settings.general.show_menu_bar_icon);
}

#[test]
fn saved_background_preferences_recover_independently() {
    let directory = TempDir::new().unwrap();
    let cases = [
        (
            serde_json::json!({"language": 17, "autoCheckUpdates": false, "closeBehavior": "background", "showMenuBarIcon": true}),
            "system",
            false,
            CloseBehavior::Background,
            true,
        ),
        (
            serde_json::json!({"language": "en", "autoCheckUpdates": "invalid", "closeBehavior": "quit", "showMenuBarIcon": true}),
            "en",
            true,
            CloseBehavior::Quit,
            true,
        ),
        (
            serde_json::json!({"language": "en", "autoCheckUpdates": false, "closeBehavior": "invalid", "showMenuBarIcon": true}),
            "en",
            false,
            CloseBehavior::Ask,
            true,
        ),
        (
            serde_json::json!({"language": "en", "closeBehavior": 4, "showMenuBarIcon": null}),
            "en",
            true,
            CloseBehavior::Ask,
            true,
        ),
        (
            serde_json::json!({"language": "en", "closeBehavior": "background", "showMenuBarIcon": "yes"}),
            "en",
            true,
            CloseBehavior::Background,
            true,
        ),
    ];
    for (general, language, auto_update, close_behavior, show_menu_bar_icon) in cases {
        let value = serde_json::json!({"version": 1, "settings": {"general": general, "appearance": {"theme": "dark"}}});
        fs::write(directory.path().join(SETTINGS_FILE_NAME), value.to_string()).unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert_eq!(
            snapshot.settings.general,
            GeneralSettings {
                language: language.into(),
                auto_update,
                close_behavior,
                show_menu_bar_icon,
                silent_start: false,
                single_key_shortcuts: true,
                startup_view: StartupView::Inbox,
            },
            "{general}"
        );
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
        assert!(!snapshot.recovered);
    }
    let settings = AppSettings {
        general: GeneralSettings {
            language: "invalid".into(),
            auto_update: false,
            close_behavior: CloseBehavior::Quit,
            show_menu_bar_icon: true,
            silent_start: true,
            single_key_shortcuts: true,
            startup_view: StartupView::Now,
        },
        ..Default::default()
    }
    .sanitized();
    assert_eq!(settings.general.language, "system");
    assert!(!settings.general.auto_update);
    assert_eq!(settings.general.close_behavior, CloseBehavior::Quit);
    assert!(settings.general.show_menu_bar_icon);
    assert!(settings.general.silent_start);
    assert_eq!(settings.general.startup_view, StartupView::Now);
}

#[test]
fn silent_login_preference_defaults_recovers_and_persists_without_changing_other_settings() {
    for value in [
        serde_json::json!({}),
        serde_json::json!({"silentStart": "yes"}),
    ] {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join(SETTINGS_FILE_NAME),
            serde_json::json!({"version":1,"settings":{"general":value,"appearance":{"theme":"dark"}}}).to_string()).unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert!(!snapshot.settings.general.silent_start);
        assert!(!snapshot.recovered);
        let mut general = snapshot.settings.general;
        general.silent_start = true;
        store
            .patch(SettingsPatch {
                general: Some(general),
                ..Default::default()
            })
            .unwrap();
        let restored = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(restored.settings.general.silent_start);
        assert_eq!(restored.settings.appearance.theme, ThemeMode::Dark);
    }
}

#[test]
fn single_key_shortcuts_default_on_recover_and_persist_off() {
    for value in [
        serde_json::json!({"language": "en", "showMenuBarIcon": false}),
        serde_json::json!({"language": "en", "singleKeyShortcuts": "no"}),
    ] {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join(SETTINGS_FILE_NAME),
            serde_json::json!({"version":1,"settings":{"general":value,"appearance":{"theme":"dark"}}}).to_string()).unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert!(snapshot.settings.general.single_key_shortcuts, "{value}");
        assert_eq!(snapshot.settings.general.language, "en");
        assert!(!snapshot.recovered);
        let mut general = snapshot.settings.general;
        general.single_key_shortcuts = false;
        store
            .patch(SettingsPatch {
                general: Some(general),
                ..Default::default()
            })
            .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&file_text(&directory, SETTINGS_FILE_NAME)).unwrap();
        assert_eq!(
            persisted["settings"]["general"]["singleKeyShortcuts"],
            false
        );
        let restored = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(!restored.settings.general.single_key_shortcuts);
        assert_eq!(restored.settings.general.language, "en");
        assert_eq!(restored.settings.appearance.theme, ThemeMode::Dark);
    }
}

#[test]
fn startup_view_defaults_to_inbox_recovers_and_persists() {
    for (value, expected) in [
        // Files written before the setting existed.
        (
            serde_json::json!({"language": "en", "showMenuBarIcon": false}),
            StartupView::Inbox,
        ),
        (
            serde_json::json!({"language": "en", "startupView": "home"}),
            StartupView::Inbox,
        ),
        (
            serde_json::json!({"language": "en", "startupView": 1}),
            StartupView::Inbox,
        ),
        (
            serde_json::json!({"language": "en", "startupView": "now"}),
            StartupView::Now,
        ),
    ] {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join(SETTINGS_FILE_NAME),
            serde_json::json!({"version":1,"settings":{"general":value,"appearance":{"theme":"dark"}}}).to_string()).unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert_eq!(snapshot.settings.general.startup_view, expected, "{value}");
        assert_eq!(snapshot.settings.general.language, "en");
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
        assert!(!snapshot.recovered);
    }
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let snapshot = store.get().unwrap();
    assert_eq!(snapshot.settings.general.startup_view, StartupView::Inbox);
    let mut general = snapshot.settings.general;
    general.startup_view = StartupView::Now;
    store
        .patch(SettingsPatch {
            general: Some(general),
            ..Default::default()
        })
        .unwrap();
    let persisted: serde_json::Value =
        serde_json::from_str(&file_text(&directory, SETTINGS_FILE_NAME)).unwrap();
    assert_eq!(persisted["settings"]["general"]["startupView"], "now");
    let restored = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(restored.settings.general.startup_view, StartupView::Now);
    // Client input with an unknown value is rejected, not silently changed.
    assert!(
        serde_json::from_value::<SettingsPatch>(
            serde_json::json!({"general": {"startupView": "home"}})
        )
        .is_err()
    );
}

#[test]
fn verification_mail_cleanup_defaults_off_repairs_invalid_values_and_persists() {
    assert!(!MailSettings::default().trash_verification_mail_after_copy);
    for value in [
        None,
        Some(serde_json::json!({})),
        Some(serde_json::json!({"trashVerificationMailAfterCopy": "yes"})),
        Some(serde_json::json!({"trashVerificationMailAfterCopy": null})),
        Some(serde_json::Value::Null),
    ] {
        let directory = TempDir::new().unwrap();
        let mut settings =
            serde_json::json!({"general":{"language":"en"},"appearance":{"theme":"dark"}});
        if let Some(value) = value {
            settings["mail"] = value;
        }
        fs::write(
            directory.path().join(SETTINGS_FILE_NAME),
            serde_json::json!({"version":1,"settings":settings}).to_string(),
        )
        .unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let snapshot = store.get().unwrap();
        assert!(!snapshot.settings.mail.trash_verification_mail_after_copy);
        assert!(!snapshot.recovered);
        let mut mail = snapshot.settings.mail;
        mail.trash_verification_mail_after_copy = true;
        store
            .patch(SettingsPatch {
                mail: Some(mail),
                ..Default::default()
            })
            .unwrap();
        let reopened = SettingsStore::open(directory.path()).unwrap();
        let restored = reopened.get().unwrap();
        assert!(restored.settings.mail.trash_verification_mail_after_copy);
        assert_eq!(restored.settings.general.language, "en");
        assert_eq!(restored.settings.appearance.theme, ThemeMode::Dark);
        let mut mail = restored.settings.mail;
        mail.trash_verification_mail_after_copy = false;
        reopened
            .patch(SettingsPatch {
                mail: Some(mail),
                ..Default::default()
            })
            .unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&file_text(&directory, SETTINGS_FILE_NAME)).unwrap();
        assert_eq!(
            saved["settings"]["mail"]["trashVerificationMailAfterCopy"],
            false
        );
        assert!(
            saved["settings"]["general"]
                .get("trashVerificationMailAfterCopy")
                .is_none()
        );
    }
}

#[test]
fn malformed_verification_cleanup_ipc_is_rejected() {
    for value in [
        serde_json::json!("yes"),
        serde_json::json!(1),
        serde_json::Value::Null,
    ] {
        assert!(
            serde_json::from_value::<SettingsPatch>(serde_json::json!({
                "mail": {"trashVerificationMailAfterCopy": value}
            }))
            .is_err()
        );
    }
}

#[test]
fn native_close_choice_preserves_current_settings_and_versions_the_commit() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    // Another window's edit lands while the native close prompt is open.
    let latest = store
        .patch(SettingsPatch {
            general: Some(GeneralSettings {
                language: "en".into(),
                auto_update: false,
                show_menu_bar_icon: true,
                ..Default::default()
            }),
            appearance: Some(AppearanceSettings {
                theme: ThemeMode::Dark,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    let chosen = store.set_close_behavior(CloseBehavior::Background).unwrap();
    let mut expected = latest.settings;
    expected.general.close_behavior = CloseBehavior::Background;
    assert_eq!(chosen.settings, expected);
    assert_eq!(chosen.revision, latest.revision + 1);
    assert!(chosen.stored);
    assert!(!chosen.recovered);
    let restored = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(restored.settings, expected);
    assert_eq!(restored.revision, 0);
    assert_eq!(restored.effective_locale, SupportedLocale::En);
}

#[test]
fn failed_native_close_choice_does_not_change_memory_or_revision() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let before = store.get().unwrap();
    fs::create_dir(directory.path().join(SETTINGS_FILE_NAME)).unwrap();
    assert!(store.set_close_behavior(CloseBehavior::Quit).is_err());
    assert_eq!(store.get().unwrap(), before);
}

#[test]
fn mcp_is_off_by_default_and_only_its_own_setter_changes_it() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let initial = store.get().unwrap();
    assert!(!initial.settings.mcp.enabled);

    let enabled = store.set_mcp_enabled(true).unwrap();
    assert!(enabled.settings.mcp.enabled);
    assert_eq!(enabled.revision, initial.revision + 1);

    // A whole-object save from the UI cannot flip the switch.
    let mut sent = enabled.settings.clone();
    sent.mcp.enabled = false;
    let saved = store.update(sent).unwrap();
    assert!(saved.settings.mcp.enabled);

    let reopened = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(reopened.settings.mcp.enabled);
    assert!(!store.set_mcp_enabled(false).unwrap().settings.mcp.enabled);
}

fn translation(edit: impl FnOnce(&mut TranslationSettings)) -> TranslationSettings {
    let mut settings = TranslationSettings::default();
    edit(&mut settings);
    settings
}

fn proxy(host: &str, port: u16) -> ServiceRoute {
    ServiceRoute::Proxy {
        proxy_kind: ProxyKind::Socks5,
        host: host.into(),
        port,
        username: String::new(),
    }
}

fn proxy_as(username: &str) -> ServiceRoute {
    match proxy("proxy.example", 1080) {
        ServiceRoute::Proxy {
            proxy_kind,
            host,
            port,
            ..
        } => ServiceRoute::Proxy {
            proxy_kind,
            host,
            port,
            username: username.into(),
        },
        ServiceRoute::Direct => unreachable!(),
    }
}

fn field_of(settings: TranslationSettings) -> Option<String> {
    settings.normalized().err().and_then(|error| error.field)
}

#[test]
fn translation_defaults_use_the_keyless_service_and_stay_private() {
    let defaults = TranslationSettings::default();
    assert_eq!(
        defaults.provider,
        Some(TranslationProviderId::MicrosoftFree)
    );
    assert_eq!(defaults.consented_provider, None);
    assert_eq!(defaults.route, ServiceRoute::Direct);
    assert_eq!(defaults.target_language, "auto");
    assert!(defaults.suggest && !defaults.include_quotes);
    assert!(defaults.never_suggest.is_empty() && defaults.always_translate.is_empty());
    assert_eq!(AppSettings::default().translation, defaults);
}

#[test]
fn translation_settings_serialize_in_the_documented_shape() {
    let settings = translation(|s| {
        s.provider = Some(TranslationProviderId::Microsoft);
        s.route = proxy("proxy.example", 1080);
    });
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["provider"], "microsoft");
    assert_eq!(json["microsoftRegion"], "");
    assert_eq!(json["targetLanguage"], "auto");
    assert_eq!(
        json["route"],
        serde_json::json!({"kind": "proxy", "proxyKind": "socks5", "host": "proxy.example", "port": 1080, "username": ""})
    );
    assert_eq!(
        serde_json::to_value(ServiceRoute::Direct).unwrap(),
        serde_json::json!({"kind": "direct"})
    );
}

#[test]
fn translation_input_is_validated_per_field() {
    assert_eq!(
        field_of(translation(|s| s.target_language = "xx".into())).as_deref(),
        Some("translation.targetLanguage")
    );
    assert_eq!(
        field_of(translation(|s| s.target_language = "zh-hans".into())).as_deref(),
        Some("translation.targetLanguage")
    );
    for ok in ["auto", "zh-Hans", "nb"] {
        assert!(
            translation(|s| s.target_language = ok.into())
                .normalized()
                .is_ok()
        );
    }
    for bad in [
        vec!["E".to_owned()],
        vec!["EN".into()],
        vec!["en1".into()],
        vec!["abcd".into()],
        vec!["zh-Hans".into()],
    ] {
        assert_eq!(
            field_of(translation(|s| s.never_suggest = bad.clone())).as_deref(),
            Some("translation.neverSuggest")
        );
        assert_eq!(
            field_of(translation(|s| s.always_translate = bad.clone())).as_deref(),
            Some("translation.alwaysTranslate")
        );
    }
    let many: Vec<String> = (0..65u8)
        .map(|n| format!("{}{}", char::from(b'a' + n / 26), char::from(b'a' + n % 26)))
        .collect();
    assert!(many.iter().all(|c| (2..=3).contains(&c.len())));
    assert_eq!(
        field_of(translation(|s| s.never_suggest = many.clone())).as_deref(),
        Some("translation.neverSuggest")
    );
    for bad in ["West", "west europe", "west_europe", &"a".repeat(41)] {
        assert_eq!(
            field_of(translation(|s| s.microsoft_region = bad.into())).as_deref(),
            Some("translation.microsoftRegion")
        );
    }
    for ok in ["", "westeurope", "east-us-2", &"a".repeat(40)] {
        assert!(
            translation(|s| s.microsoft_region = ok.into())
                .normalized()
                .is_ok()
        );
    }
}

#[test]
fn language_lists_are_sorted_unique_and_exclusive() {
    let normalized = translation(|s| {
        s.never_suggest = vec!["fr".into(), "de".into(), "fr".into()];
        s.always_translate = vec!["ja".into(), "ko".into()];
    })
    .normalized()
    .unwrap();
    assert_eq!(normalized.never_suggest, ["de", "fr"]);
    assert_eq!(
        field_of(translation(|s| {
            s.never_suggest = vec!["fr".into()];
            s.always_translate = vec!["de".into(), "fr".into()];
        }))
        .as_deref(),
        Some("translation.alwaysTranslate")
    );
}

#[test]
fn translation_proxy_needs_a_plain_host_and_a_port() {
    assert!(
        translation(|s| s.route = proxy("127.0.0.1", 8080))
            .normalized()
            .is_ok()
    );
    for host in ["", "a b", "a\tb", "a\nb", &"h".repeat(256)] {
        assert_eq!(
            field_of(translation(|s| s.route = proxy(host, 80))).as_deref(),
            Some("translation.route.host"),
            "{host:?}"
        );
    }
    assert!(
        translation(|s| s.route = proxy(&"h".repeat(255), 80))
            .normalized()
            .is_ok()
    );
    assert_eq!(
        field_of(translation(|s| s.route = proxy("host", 0))).as_deref(),
        Some("translation.route.port")
    );
}

#[test]
fn translation_proxy_username_follows_the_account_proxy_rule() {
    for ok in ["", "alice", " padded ", "名字", &"u".repeat(255)] {
        let normalized = translation(|s| s.route = proxy_as(ok))
            .normalized()
            .unwrap();
        assert_eq!(normalized.route, proxy_as(ok), "kept as typed: {ok:?}");
    }
    for bad in [
        "   ",
        "\t",
        "a\nb",
        "a\u{0}b",
        &"u".repeat(256),
        &"名".repeat(86),
    ] {
        assert_eq!(
            field_of(translation(|s| s.route = proxy_as(bad))).as_deref(),
            Some("translation.route.username"),
            "{bad:?}"
        );
    }
}

#[test]
fn a_route_saved_without_a_username_still_loads() {
    let route: ServiceRoute = serde_json::from_str(
        r#"{"kind":"proxy","proxyKind":"httpConnect","host":"p","port":8080}"#,
    )
    .unwrap();
    assert!(matches!(route, ServiceRoute::Proxy { username, .. } if username.is_empty()));
}

#[test]
fn a_bad_stored_username_is_dropped_but_the_proxy_stays() {
    let directory = TempDir::new().unwrap();
    for (username, expected) in [(r#""bad\nname""#, ""), ("42", ""), (r#""alice""#, "alice")] {
        fs::write(
            directory.path().join(SETTINGS_FILE_NAME),
            format!(
                r#"{{"version":1,"settings":{{"translation":{{
                "route":{{"kind":"proxy","proxyKind":"socks5","host":"p","port":1080,"username":{username}}}}}}}}}"#
            ),
        )
        .unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(!snapshot.recovered);
        assert_eq!(
            snapshot.settings.translation.route,
            ServiceRoute::Proxy {
                proxy_kind: ProxyKind::Socks5,
                host: "p".into(),
                port: 1080,
                username: expected.into(),
            }
        );
        assert!(snapshot.settings.translation.clone().normalized().is_ok());
    }
}

#[test]
fn patching_translation_replaces_only_that_section() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    store
        .patch(SettingsPatch {
            appearance: Some(AppearanceSettings {
                accent_color: "#112233".into(),
                ..AppearanceSettings::default()
            }),
            ..SettingsPatch::default()
        })
        .unwrap();
    let chosen = translation(|s| {
        s.provider = Some(TranslationProviderId::Google);
        s.consented_provider = Some(TranslationProviderId::Google);
        s.never_suggest = vec!["fr".into()];
    });
    let snapshot = store
        .patch(SettingsPatch {
            translation: Some(chosen.clone()),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(snapshot.settings.translation, chosen);
    assert_eq!(snapshot.settings.appearance.accent_color, "#112233");
    let rejected = store.patch(SettingsPatch {
        translation: Some(translation(|s| s.target_language = "xx".into())),
        ..SettingsPatch::default()
    });
    assert_eq!(
        rejected.unwrap_err().field.as_deref(),
        Some("translation.targetLanguage")
    );
    let reopened = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(reopened.get().unwrap().settings.translation, chosen);
}

#[test]
fn a_hand_edited_translation_section_falls_back_per_field() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"appearance":{"theme":"dark"},"translation":{
            "provider":"nonsense","consentedProvider":"deepl","microsoftRegion":"Bad Region",
            "route":{"kind":"proxy","proxyKind":"socks5","host":"","port":1},
            "targetLanguage":"xx","suggest":false,"neverSuggest":["fr","DE","fr"],
            "alwaysTranslate":["fr","ja"],"includeQuotes":"yes"}}}"#,
    )
    .unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(
        !snapshot.recovered,
        "one bad field must not discard the file"
    );
    assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
    let t = snapshot.settings.translation;
    // An unknown service loses only itself and falls back to the default.
    assert_eq!(t.provider, Some(TranslationProviderId::MicrosoftFree));
    assert_eq!(t.consented_provider, Some(TranslationProviderId::Deepl));
    assert_eq!(t.microsoft_region, "");
    assert_eq!(t.route, ServiceRoute::Direct);
    assert_eq!(t.target_language, "auto");
    assert!(!t.suggest, "a valid field keeps its value");
    assert!(!t.include_quotes);
    // "DE" is invalid so the whole list falls back; "fr" is kept out of the never list.
    assert!(t.never_suggest.is_empty());
    assert_eq!(t.always_translate, ["fr", "ja"]);
    // Whatever was loaded passes validation as is.
    assert!(t.clone().normalized().is_ok());
}

#[test]
fn a_translation_section_of_the_wrong_type_is_ignored() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"translation":[1,2]}}"#,
    )
    .unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(!snapshot.recovered);
    assert_eq!(
        snapshot.settings.translation,
        TranslationSettings::default()
    );
}

#[test]
fn a_saved_off_choice_survives_and_a_missing_one_gets_the_default() {
    let load = |translation: &str| {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join(SETTINGS_FILE_NAME),
            format!(r#"{{"version":1,"settings":{{"translation":{translation}}}}}"#),
        )
        .unwrap();
        SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap()
            .settings
            .translation
    };
    assert_eq!(load(r#"{"provider":null}"#).provider, None);
    assert_eq!(
        load(r#"{"suggest":false}"#).provider,
        Some(TranslationProviderId::MicrosoftFree)
    );
    assert_eq!(
        load(r#"{"provider":"googleFree"}"#).provider,
        Some(TranslationProviderId::GoogleFree)
    );
}

#[test]
fn only_the_browser_channels_work_without_a_key() {
    let keyless: Vec<_> = TranslationProviderId::ALL
        .into_iter()
        .filter(|p| !p.needs_key())
        .collect();
    assert_eq!(
        keyless,
        [
            TranslationProviderId::MicrosoftFree,
            TranslationProviderId::GoogleFree,
            TranslationProviderId::Ai
        ]
    );
    assert_eq!(
        serde_json::to_value(TranslationProviderId::GoogleFree).unwrap(),
        "googleFree"
    );
}

// ------------------------------------------------------------------- ai

fn ai_connection(name: &str) -> crate::ai::AiConnection {
    crate::ai::AiConnection {
        id: Uuid::new_v4().to_string(),
        provider_id: "custom".into(),
        name: name.into(),
        protocol: crate::ai::AiProtocol::OpenaiCompatible,
        base_url: "https://api.example.com/v1".into(),
        use_proxy: false,
    }
}

#[test]
fn only_the_ai_update_changes_the_ai_section() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let connection = ai_connection("A");
    let id = connection.id.clone();
    store
        .update_ai(|ai| {
            ai.connections.push(connection);
            Ok(())
        })
        .unwrap();

    // The general patch has no `ai` field, so a client cannot send one.
    assert!(serde_json::from_str::<SettingsPatch>(r#"{"ai":{"connections":[]}}"#).is_err());
    let patched = store
        .patch(serde_json::from_str::<SettingsPatch>(r#"{"appearance":{"theme":"dark"}}"#).unwrap())
        .unwrap();
    assert_eq!(patched.settings.ai.connections.len(), 1);
    // `update` replaces everything else but keeps `ai`.
    let replaced = store.update(AppSettings::default()).unwrap();
    assert_eq!(replaced.settings.ai.connections[0].id, id);
    let reopened = SettingsStore::open(directory.path()).unwrap();
    assert_eq!(reopened.get().unwrap().settings.ai.connections[0].id, id);
}

#[test]
fn a_failed_ai_update_changes_nothing() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let before = store.get().unwrap();
    let error = store
        .update_ai(|ai| {
            ai.connections.push(ai_connection("  "));
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.field.as_deref(), Some("ai.connections"));
    let error: Result<(), AppError> = store.update_ai(|_| Err(AppError::validation("x")));
    assert!(error.is_err());
    assert_eq!(store.get().unwrap(), before);
    assert!(!directory.path().join(SETTINGS_FILE_NAME).exists());
}

#[test]
fn a_damaged_ai_section_loses_only_the_damaged_part() {
    let directory = TempDir::new().unwrap();
    let good = ai_connection("Good");
    let other = ai_connection("Other");
    let text = serde_json::json!({
        "version": 1,
        "settings": {
            "appearance": {"theme": "dark"},
            "ai": {
                "connections": [good, {"id": 7}, "junk", other],
                "defaultModel": {"connectionId": good.id, "modelId": "m", "name": "M", "contextLength": null, "outputLength": null},
                "route": {"kind": "proxy", "proxyKind": "socks5", "host": "p.example", "port": 1080, "username": 5}
            }
        }
    });
    fs::write(directory.path().join(SETTINGS_FILE_NAME), text.to_string()).unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(!snapshot.recovered);
    assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
    let ai = snapshot.settings.ai;
    assert_eq!(ai.connections.len(), 2);
    assert_eq!(ai.default_model.unwrap().connection_id, good.id);
    // A bad username is dropped; the proxy itself stays.
    assert!(
        matches!(ai.route, ServiceRoute::Proxy { ref host, ref username, .. } if host == "p.example" && username.is_empty())
    );

    // Wrong types all the way down: the rest of the file survives.
    for ai in [
        serde_json::json!("text"),
        serde_json::json!(null),
        serde_json::json!({"connections": 5, "defaultModel": 3, "route": "x"}),
        serde_json::json!({"connections": [{"id": "not-a-uuid", "providerId": "x", "name": "n", "protocol": "anthropic", "baseUrl": "ftp://x", "useProxy": false}]}),
    ] {
        let text = serde_json::json!({
            "version": 1,
            "settings": {"appearance": {"theme": "dark"}, "ai": ai}
        });
        fs::write(directory.path().join(SETTINGS_FILE_NAME), text.to_string()).unwrap();
        let snapshot = SettingsStore::open(directory.path())
            .unwrap()
            .get()
            .unwrap();
        assert!(!snapshot.recovered, "{ai}");
        assert_eq!(snapshot.settings.appearance.theme, ThemeMode::Dark);
        assert_eq!(snapshot.settings.ai, AiSettings::default(), "{ai}");
    }
}

#[test]
fn a_default_model_on_a_missing_connection_is_dropped_when_loading() {
    let directory = TempDir::new().unwrap();
    let text = serde_json::json!({
        "version": 1,
        "settings": {"ai": {
            "connections": [],
            "defaultModel": {"connectionId": Uuid::new_v4().to_string(), "modelId": "m", "name": "M", "contextLength": 1, "outputLength": 2}
        }}
    });
    fs::write(directory.path().join(SETTINGS_FILE_NAME), text.to_string()).unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(snapshot.settings.ai.default_model, None);
}

#[test]
fn the_shared_route_keeps_its_wire_shape() {
    let route: ServiceRoute = serde_json::from_str(
        r#"{"kind":"proxy","proxyKind":"httpConnect","host":"h","port":8080,"username":"u"}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&route).unwrap(),
        serde_json::json!({"kind":"proxy","proxyKind":"httpConnect","host":"h","port":8080,"username":"u"})
    );
    assert_eq!(route.validate("ai.route"), Ok(()));
    let bad = ServiceRoute::Proxy {
        proxy_kind: ProxyKind::Socks5,
        host: "h".into(),
        port: 0,
        username: String::new(),
    };
    assert_eq!(
        bad.validate("ai.route").unwrap_err().field.as_deref(),
        Some("ai.route.port")
    );
}

const AI_ID: &str = "6f1f3d3e-8a40-4a39-9d3c-0f5f7a6b2c11";

fn consent(id: &str, url: &str) -> AiConsent {
    AiConsent {
        connection_id: id.into(),
        base_url: url.into(),
    }
}

#[test]
fn the_ai_consent_is_validated_and_normalized_with_the_ai_url_rules() {
    assert_eq!(TranslationSettings::default().consented_ai, None);
    let ok = translation(|s| {
        s.provider = Some(TranslationProviderId::Ai);
        s.consented_ai = Some(consent(AI_ID, " https://api.example.com/v1/ "));
    })
    .normalized()
    .unwrap();
    assert_eq!(
        ok.consented_ai,
        Some(consent(AI_ID, "https://api.example.com/v1"))
    );
    for bad in [
        consent("not-a-uuid", "https://api.example.com/v1"),
        consent(AI_ID, "ftp://api.example.com"),
        consent(AI_ID, ""),
        consent(AI_ID, "https://user:pw@api.example.com/v1"),
    ] {
        assert_eq!(
            field_of(translation(|s| s.consented_ai = Some(bad))).as_deref(),
            Some("translation.consentedAi")
        );
    }
    let json = serde_json::to_value(translation(|s| {
        s.provider = Some(TranslationProviderId::Ai);
        s.consented_ai = Some(consent(AI_ID, "https://api.example.com/v1"));
    }))
    .unwrap();
    assert_eq!(json["provider"], "ai");
    assert_eq!(
        json["consentedAi"],
        serde_json::json!({"connectionId": AI_ID, "baseUrl": "https://api.example.com/v1"})
    );
    assert!(!TranslationProviderId::Ai.needs_key());
    assert_eq!(TranslationProviderId::Ai.as_str(), "ai");
}

#[test]
fn a_hand_edited_ai_consent_and_translation_model_are_repaired() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"translation":{
                "provider":"ai","consentedProvider":"ai","consentedAi":{"connectionId":5}},
              "ai":{"connections":[],"translationModel":{"connectionId":7}}}}"#,
    )
    .unwrap();
    let snapshot = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert!(!snapshot.recovered);
    let t = snapshot.settings.translation;
    assert_eq!(t.provider, Some(TranslationProviderId::Ai));
    assert_eq!(t.consented_provider, Some(TranslationProviderId::Ai));
    assert_eq!(t.consented_ai, None);
    assert_eq!(snapshot.settings.ai.translation_model, None);

    // A well-formed value with an unusable URL loses only itself.
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        format!(
            r#"{{"version":1,"settings":{{"translation":{{
                "provider":"ai","consentedAi":{{"connectionId":"{AI_ID}","baseUrl":"ftp://x"}}}}}}}}"#
        ),
    )
    .unwrap();
    let t = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap()
        .settings
        .translation;
    assert_eq!(t.consented_ai, None);
    assert!(t.normalized().is_ok());
}
