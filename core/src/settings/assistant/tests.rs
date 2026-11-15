use std::fs;

use tempfile::TempDir;
use uuid::Uuid;

use crate::settings::{
    AiConsent, AssistantMode, AssistantSendMode, AssistantSettings, GeneralSettings,
    MAX_INSTRUCTIONS_CHARS, SETTINGS_FILE_NAME, SettingsPatch, SettingsStore,
};

fn assistant(edit: impl FnOnce(&mut AssistantSettings)) -> AssistantSettings {
    let mut settings = AssistantSettings::default();
    edit(&mut settings);
    settings
}

const ID_A: &str = "11111111-1111-4111-8111-111111111111";
const ID_B: &str = "22222222-2222-4222-8222-222222222222";

#[test]
fn defaults_apply_to_old_files() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"general":{"language":"en"}}}"#,
    )
    .unwrap();
    let loaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    let defaults = AssistantSettings::default();
    assert_eq!(loaded.settings.assistant, defaults);
    assert!(defaults.enabled);
    assert_eq!(defaults.organize, AssistantMode::Direct);
    assert_eq!(defaults.ai_drafts, AssistantMode::Direct);
    assert_eq!(defaults.send, AssistantSendMode::Confirm);
    assert_eq!(defaults.model, None);
    assert_eq!(defaults.consented, None);
}

#[test]
fn patch_persists_and_reloads_without_touching_other_sections() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let edited = assistant(|a| {
        a.enabled = false;
        a.organize = AssistantMode::Ask;
        a.ai_drafts = AssistantMode::Ask;
        a.send = AssistantSendMode::Direct;
        a.excluded_accounts = vec![ID_A.to_uppercase(), ID_B.into(), ID_A.into()];
        a.instructions = "  Call me Sam.  ".into();
    });
    let snapshot = store
        .patch(SettingsPatch {
            assistant: Some(edited),
            ..SettingsPatch::default()
        })
        .unwrap();
    let saved = snapshot.settings.assistant;
    assert!(!saved.enabled);
    assert_eq!(saved.excluded_accounts, vec![ID_A, ID_B]);
    assert_eq!(saved.instructions, "Call me Sam.");
    assert_eq!(snapshot.settings.general, GeneralSettings::default());
    let text = fs::read_to_string(directory.path().join(SETTINGS_FILE_NAME)).unwrap();
    assert!(text.contains("\"aiDrafts\": \"ask\"") && text.contains("\"send\": \"direct\""));
    let reloaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(reloaded.settings.assistant.organize, AssistantMode::Ask);
    assert_eq!(reloaded.settings.assistant.excluded_accounts.len(), 2);
}

fn consent(base_url: &str) -> AiConsent {
    AiConsent {
        connection_id: ID_A.into(),
        base_url: base_url.into(),
    }
}

#[test]
fn a_patch_or_update_cannot_set_or_clear_the_consent() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    // Not even an invalid value is looked at.
    let forged = store
        .patch(SettingsPatch {
            assistant: Some(assistant(|a| {
                a.send = AssistantSendMode::Direct;
                a.consented = Some(consent("https://evil.example.test/v1"));
            })),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(forged.settings.assistant.send, AssistantSendMode::Direct);
    assert_eq!(forged.settings.assistant.consented, None);
    let mut all = forged.settings.clone();
    all.assistant.consented = Some(consent("https://evil.example.test/v1"));
    assert_eq!(
        store.update(all).unwrap().settings.assistant.consented,
        None
    );

    // The assistant's own path sets it; patches then keep it, even when they
    // carry none or another.
    store
        .update_assistant(|a| {
            a.consented = Some(consent("https://api.example.test/v1/"));
            Ok(())
        })
        .unwrap();
    for sent in [None, Some(consent("https://other.example.test"))] {
        let after = store
            .patch(SettingsPatch {
                assistant: Some(assistant(|a| a.consented = sent)),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(
            after.settings.assistant.consented,
            Some(consent("https://api.example.test/v1")),
        );
    }
    let mut all = store.get().unwrap().settings;
    all.assistant.consented = None;
    assert!(
        store
            .update(all)
            .unwrap()
            .settings
            .assistant
            .consented
            .is_some()
    );
}

#[test]
fn invalid_input_is_rejected_with_its_field() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    let rejected = |edit: Box<dyn FnOnce(&mut AssistantSettings)>| {
        store
            .patch(SettingsPatch {
                assistant: Some(assistant(edit)),
                ..SettingsPatch::default()
            })
            .unwrap_err()
    };
    let too_long = rejected(Box::new(|a| {
        a.instructions = "x".repeat(MAX_INSTRUCTIONS_CHARS + 1)
    }));
    assert_eq!(too_long.field.as_deref(), Some("assistant.instructions"));
    let exact = assistant(|a| a.instructions = "好".repeat(MAX_INSTRUCTIONS_CHARS));
    assert!(exact.normalized().is_ok());
    let bad_account = rejected(Box::new(|a| a.excluded_accounts = vec!["nope".into()]));
    assert_eq!(
        bad_account.field.as_deref(),
        Some("assistant.excludedAccounts")
    );
    let many = rejected(Box::new(|a| {
        a.excluded_accounts = (0..65).map(|_| Uuid::new_v4().to_string()).collect()
    }));
    assert_eq!(many.field.as_deref(), Some("assistant.excludedAccounts"));
    assert_eq!(
        store.get().unwrap().settings.assistant,
        AssistantSettings::default()
    );
}

#[test]
fn hand_edited_values_are_repaired_per_field() {
    let directory = TempDir::new().unwrap();
    let long = "y".repeat(MAX_INSTRUCTIONS_CHARS + 50);
    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        format!(
            r#"{{"version":1,"settings":{{"general":{{"language":"en"}},"assistant":{{
                "enabled":"yes","organize":"ask","aiDrafts":"sometimes","send":"direct",
                "excludedAccounts":["{ID_A}","{ID_A}","junk"],
                "consented":{{"connectionId":"bad","baseUrl":"https://x.test"}},
                "model":5,"instructions":"{long}"}}}}}}"#
        ),
    )
    .unwrap();
    let loaded = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    let a = loaded.settings.assistant;
    assert!(a.enabled, "a wrong type falls back to the default");
    assert_eq!(a.organize, AssistantMode::Ask);
    assert_eq!(a.ai_drafts, AssistantMode::Direct);
    assert_eq!(a.send, AssistantSendMode::Direct);
    assert_eq!(a.excluded_accounts, vec![ID_A]);
    assert_eq!(a.consented, None);
    assert_eq!(a.model, None);
    assert_eq!(a.instructions.chars().count(), MAX_INSTRUCTIONS_CHARS);

    fs::write(
        directory.path().join(SETTINGS_FILE_NAME),
        r#"{"version":1,"settings":{"general":{"language":"en"},"assistant":"x"}}"#,
    )
    .unwrap();
    let again = SettingsStore::open(directory.path())
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(again.settings.assistant, AssistantSettings::default());
}

#[test]
fn update_assistant_changes_only_that_section_and_validates() {
    let directory = TempDir::new().unwrap();
    let store = SettingsStore::open(directory.path()).unwrap();
    store
        .update_assistant(|a| {
            a.send = AssistantSendMode::Direct;
            Ok(())
        })
        .unwrap();
    let after = store.get().unwrap();
    assert_eq!(after.settings.assistant.send, AssistantSendMode::Direct);
    assert_eq!(after.revision, 1);
    assert!(
        store
            .update_assistant(|a| {
                a.instructions = "x".repeat(MAX_INSTRUCTIONS_CHARS + 1);
                Ok(())
            })
            .is_err()
    );
    assert_eq!(store.get().unwrap().revision, 1);
}
