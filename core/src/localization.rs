//! Host-independent text adapters. Dates, numbers and list joining keep their
//! existing format; the host supplies the effective UI language.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum SupportedLocale {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[default]
    #[serde(rename = "en")]
    En,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum LanguagePreference {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

impl SupportedLocale {
    /// Map a system language tag. Other languages and failures use English.
    pub fn from_system_tag(tag: Option<&str>) -> Self {
        let language = tag
            .unwrap_or_default()
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default();
        if language.eq_ignore_ascii_case("zh") {
            Self::ZhCn
        } else {
            Self::En
        }
    }

    pub fn resolve(preference: &str, system: Self) -> Self {
        match preference {
            "zh-CN" => Self::ZhCn,
            "en" => Self::En,
            _ => system,
        }
    }

    fn text(self, templates: bool, key: &str, values: &[(&str, &str)]) -> String {
        let catalog = catalogs(self);
        let resources = if templates { &catalog.1 } else { &catalog.0 };
        let fallback = catalogs(Self::ZhCn);
        let fallback = if templates { &fallback.1 } else { &fallback.0 };
        let template = resources
            .get(key)
            .or_else(|| fallback.get(key))
            .and_then(Value::as_str)
            .unwrap_or("MailX");
        interpolate(template, values)
    }

    pub fn no_subject(self) -> String {
        self.text(false, "noSubject", &[])
    }
    pub fn new_mail_title(self) -> String {
        self.text(true, "newMailTitle", &[])
    }
    pub fn test_notification(self) -> String {
        self.text(false, "test", &[])
    }

    pub fn new_mail(self, count: u32) -> String {
        let key = if self == Self::En && count == 1 {
            "newMail_one"
        } else {
            "newMail_other"
        };
        self.text(false, key, &[("count", &count.to_string())])
    }

    pub fn new_mail_from(self, count: u32, senders: &str, more: bool) -> String {
        let key = if self == Self::En && count == 1 {
            "newMailFrom_one"
        } else {
            "newMailFrom_other"
        };
        let more = if more {
            self.text(false, "moreSenders", &[])
        } else {
            String::new()
        };
        self.text(
            false,
            key,
            &[
                ("count", &count.to_string()),
                ("senders", senders),
                ("more", &more),
            ],
        )
    }

    pub fn send_result(self, state: crate::OutboxState) -> Option<String> {
        let key = match state {
            crate::OutboxState::Accepted => "sent",
            crate::OutboxState::Failed => "failed",
            crate::OutboxState::Unknown => "unknown",
            _ => return None,
        };
        Some(self.text(false, key, &[]))
    }

    pub(crate) fn reply_intro(self, date: &str, sender: &str) -> String {
        self.text(true, "replyIntro", &[("date", date), ("sender", sender)])
    }

    pub(crate) fn image_alt(self, alt: &str) -> String {
        self.text(true, "imageAlt", &[("alt", alt)])
    }

    pub(crate) fn forward_header(
        self,
        date: &str,
        sender: &str,
        subject: &str,
        recipients: &str,
    ) -> String {
        self.text(
            true,
            "forwardHeader",
            &[
                ("date", date),
                ("sender", sender),
                ("subject", subject),
                ("recipients", recipients),
            ],
        )
    }
}

fn catalogs(locale: SupportedLocale) -> &'static (Value, Value) {
    static ZH: OnceLock<(Value, Value)> = OnceLock::new();
    static EN: OnceLock<(Value, Value)> = OnceLock::new();
    let (cell, notifications, templates) = match locale {
        SupportedLocale::ZhCn => (
            &ZH,
            include_str!("../../locales/zh-CN/notifications.json"),
            include_str!("../../locales/zh-CN/composeTemplates.json"),
        ),
        SupportedLocale::En => (
            &EN,
            include_str!("../../locales/en/notifications.json"),
            include_str!("../../locales/en/composeTemplates.json"),
        ),
    };
    cell.get_or_init(|| {
        (
            serde_json::from_str(notifications).expect("embedded notification resources"),
            serde_json::from_str(templates).expect("embedded compose resources"),
        )
    })
}

/// Process the template once. A sender containing `{{date}}` stays literal.
fn interpolate(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        result.push_str(&remaining[..start]);
        let token = &remaining[start + 2..];
        let Some(end) = token.find("}}") else {
            result.push_str(&remaining[start..]);
            return result;
        };
        let name = &token[..end];
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        }
        remaining = &token[end + 2..];
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_system_language_and_respects_explicit_preference() {
        for tag in ["zh", "zh-CN", "zh-Hant-TW", "ZH_hk.UTF-8"] {
            assert_eq!(
                SupportedLocale::from_system_tag(Some(tag)),
                SupportedLocale::ZhCn
            );
        }
        for tag in [None, Some("fr-FR"), Some("en-GB")] {
            assert_eq!(SupportedLocale::from_system_tag(tag), SupportedLocale::En);
        }
        assert_eq!(
            SupportedLocale::resolve("en", SupportedLocale::ZhCn),
            SupportedLocale::En
        );
    }

    #[test]
    fn text_uses_english_plurals_and_does_not_reinterpret_values() {
        assert_eq!(SupportedLocale::En.new_mail(1), "1 new message");
        assert_eq!(SupportedLocale::En.new_mail(2), "2 new messages");
        assert_eq!(SupportedLocale::ZhCn.new_mail(1), "1 封新邮件");
        assert_eq!(
            SupportedLocale::En.reply_intro("2026-10-25 14:00", "{{date}}"),
            "\nOn 2026-10-25 14:00, {{date}} wrote:"
        );
        for locale in [SupportedLocale::ZhCn, SupportedLocale::En] {
            assert!(!locale.new_mail_from(2, "A、B", false).contains("{{"));
            assert!(
                !locale
                    .forward_header("date", "sender", "subject", "recipients")
                    .contains("{{")
            );
        }
    }
}
