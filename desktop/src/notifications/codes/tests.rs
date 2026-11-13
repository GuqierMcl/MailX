use super::*;
use mailx_core::mail::verification_code::{Detection, detect};
use mailx_core::{MailAddress, NewMailMessage, NotificationSound};

fn mail() -> NewMail {
    NewMail {
        account_id: "account".into(),
        account_name: "Personal".into(),
        count: 5,
        messages: (1..=5)
            .map(|i| NewMailMessage {
                email_id: format!("mail-{i}"),
                from: MailAddress {
                    name: Some(format!("Sender {i}")),
                    address: format!("s{i}@example.test"),
                },
                subject: if i == 5 {
                    "Your verification code".into()
                } else {
                    format!("Message {i}")
                },
                received_at: "2026-11-13T10:00:00Z".into(),
            })
            .collect(),
    }
}

fn found() -> HashMap<String, Candidate> {
    let Detection::Found { candidate } = detect("", "Verification code: 001234").unwrap() else {
        panic!("fixture");
    };
    HashMap::from([("mail-5".into(), candidate)])
}

#[test]
fn code_beyond_the_old_three_message_limit_has_its_own_notice() {
    let deliveries = plan_batch(
        &mail(),
        &NotificationSettings::default(),
        false,
        SupportedLocale::En,
        &found(),
    );
    assert_eq!(deliveries.len(), 2);
    assert_eq!(deliveries[0].shown.email_id.as_deref(), Some("mail-5"));
    assert!(
        deliveries[0]
            .shown
            .body
            .starts_with("Verification code: 001234\n")
    );
    assert!(deliveries[0].shown.title.contains("Personal"));
    assert_eq!(deliveries[0].code.as_ref().unwrap().code, "001234");
    assert!(deliveries[1].shown.body.starts_with("4 new messages"));
    assert!(!deliveries[1].shown.body.contains("Sender 5"));
    assert!(deliveries[1].code.is_none());
}

#[test]
fn final_settings_and_focus_can_suppress_completed_detection() {
    let base = NotificationSettings::default();
    for settings in [
        NotificationSettings {
            enabled: false,
            ..base.clone()
        },
        NotificationSettings {
            muted_accounts: vec!["account".into()],
            ..base.clone()
        },
    ] {
        assert!(plan_batch(&mail(), &settings, false, SupportedLocale::En, &found()).is_empty());
    }
    assert!(plan_batch(&mail(), &base, true, SupportedLocale::En, &found()).is_empty());
    let private = NotificationSettings {
        show_preview: false,
        ..base
    };
    let deliveries = plan_batch(&mail(), &private, false, SupportedLocale::En, &found());
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].shown.body, "5 new messages");
    assert!(deliveries[0].code.is_none());
    assert!(deliveries[0].shown.email_id.is_none());
}

#[test]
fn failures_and_ambiguous_results_use_one_ordinary_notification() {
    let deliveries = plan_batch(
        &mail(),
        &NotificationSettings::default(),
        false,
        SupportedLocale::En,
        &HashMap::new(),
    );
    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].shown.body.starts_with("5 new messages"));
    assert!(deliveries[0].code.is_none());
}

#[test]
fn a_late_settings_read_cannot_restore_revoked_notification_content() {
    let mut policy = None;
    let private = NotificationSettings {
        show_preview: false,
        ..NotificationSettings::default()
    };
    apply_policy(&mut policy, 2, &private, SupportedLocale::ZhCn);
    apply_policy(
        &mut policy,
        1,
        &NotificationSettings::default(),
        SupportedLocale::En,
    );
    let policy = policy.unwrap();
    let deliveries = plan_batch(&mail(), &policy.settings, false, policy.locale, &found());
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].shown.body, "5 封新邮件");
    assert!(deliveries[0].code.is_none());
}

#[test]
fn mixed_and_code_only_batches_sound_once_and_use_current_language() {
    let settings = NotificationSettings {
        sound: NotificationSound::Chime,
        ..NotificationSettings::default()
    };
    let deliveries = plan_batch(&mail(), &settings, false, SupportedLocale::ZhCn, &found());
    assert!(deliveries[0].shown.body.starts_with("验证码：001234\n"));
    assert_eq!(
        deliveries[0].shown.own_sound,
        Some(NotificationSound::Chime)
    );
    assert_eq!(deliveries[1].shown.own_sound, None);
    assert_eq!(deliveries[1].shown.system_sound, None);
    let mut one = mail();
    one.messages.retain(|message| message.email_id == "mail-5");
    one.count = 1;
    assert_eq!(
        plan_batch(&one, &settings, false, SupportedLocale::En, &found()).len(),
        1
    );
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn tokens_are_opaque_bounded_expiring_and_not_recreated_after_restart() {
    let now = std::time::Instant::now();
    let mut store = ActionStore::default();
    let token = store.insert("account".into(), "mail".into(), "001234".into(), now);
    assert!(uuid::Uuid::parse_str(&token).is_ok());
    assert!(!token.contains("001234"));
    assert_eq!(store.get(&token, now).unwrap().code, "001234");
    assert!(
        store
            .get(&token, now + super::super::CODE_ACTION_TTL)
            .is_none()
    );
    for _ in 0..130 {
        store.insert("account".into(), "mail".into(), "001234".into(), now);
    }
    assert_eq!(store.entries.len(), 128);
    assert!(ActionStore::default().get(&token, now).is_none());
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn disabling_preview_or_muting_revokes_copy_and_consumption_is_explicit() {
    let now = std::time::Instant::now();
    for settings in [
        NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        },
        NotificationSettings {
            show_preview: false,
            ..NotificationSettings::default()
        },
        NotificationSettings {
            muted_accounts: vec!["account".into()],
            ..NotificationSettings::default()
        },
    ] {
        let mut store = ActionStore::default();
        let token = store.insert("account".into(), "mail".into(), "001234".into(), now);
        store.retain_allowed(&settings);
        assert!(store.get(&token, now).is_none());
    }
    let mut store = ActionStore::default();
    let token = store.insert("account".into(), "mail".into(), "001234".into(), now);
    store.retain_allowed(&NotificationSettings::default());
    assert!(store.get(&token, now).is_some());
    store.remove(&token);
    assert!(store.get(&token, now).is_none());
}
