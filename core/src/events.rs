//! Everything the core tells its host without being asked, through one sink.
//!
//! A host implements [`EventSink`] once and decides where each event goes: the
//! desktop forwards sync and write-back events to its window and turns new mail
//! into system notifications; a server host could post the same new-mail event
//! to a webhook. Delivery is a hint, never a source of truth: a missed event
//! only means a missed refresh or notice, so sinks must return quickly and may
//! drop what they cannot deliver.
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use crate::mail::MailAddress;
use crate::send::types::{DraftChanged, OutboxChanged};
use crate::settings::NotificationSettings;
use crate::sync::manager::{AccountSyncState, FlagWriteBack};

/// Messages described one by one; the rest of a batch is only counted.
pub const NEW_MAIL_DETAILS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum CoreEvent {
    /// A local account and its data were removed; every account picker refetches.
    AccountRemoved(crate::accounts::types::AccountRemoval),
    /// An account's sync state changed; readers refetch on new revisions.
    Sync(AccountSyncState),
    /// A read / starred write-back round finished.
    FlagWriteBack(FlagWriteBack),
    /// Unread mail arrived in an inbox since the last completed sync round.
    NewMail(NewMail),
    /// A send queue item changed state (queued, sending, accepted, ...).
    Outbox(OutboxChanged),
    /// A draft was locked, unlocked or deleted by the send queue.
    DraftChanged(DraftChanged),
}

/// Receives every event. Called on the core's worker threads.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &CoreEvent);
}

impl<F: Fn(&CoreEvent) + Send + Sync> EventSink for F {
    fn emit(&self, event: &CoreEvent) {
        self(event)
    }
}

pub type Events = Arc<dyn EventSink>;

/// New unread mail in one account's inbox, found by one sync round. Mail that
/// was already there (a first sync, a rebuilt folder, older mail filled in
/// later) is never reported, and neither is mail read elsewhere meanwhile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewMail {
    pub account_id: String,
    pub account_name: String,
    /// Every new message, including those not described below.
    #[ts(type = "number")]
    pub count: u32,
    /// The newest few, newest first. Empty when previews are turned off.
    pub messages: Vec<NewMailMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewMailMessage {
    pub email_id: String,
    pub from: MailAddress,
    pub subject: String,
    /// RFC 3339 timestamp.
    pub received_at: String,
}

impl NewMail {
    /// What may leave the core under the user's notification settings, for
    /// every host alike: `None` when notifications are off or the account is
    /// muted; without sender and subject when previews are off.
    pub fn for_delivery(&self, settings: &NotificationSettings) -> Option<NewMail> {
        if !settings.enabled
            || self.count == 0
            || settings.muted_accounts.contains(&self.account_id)
        {
            return None;
        }
        let mut delivered = self.clone();
        if !settings.show_preview {
            delivered.messages.clear();
        }
        Some(delivered)
    }

    /// The notification text, the same wherever it is shown: one message names
    /// its sender and subject, several are summed up per account.
    pub fn summary(&self) -> (String, String) {
        self.summary_localized(crate::SupportedLocale::ZhCn)
    }

    pub fn summary_localized(&self, locale: crate::SupportedLocale) -> (String, String) {
        match self.messages.as_slice() {
            [message] if self.count == 1 => {
                let from = message
                    .from
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&message.from.address);
                let subject = message.subject.trim();
                (
                    if from.is_empty() {
                        self.account_name.clone()
                    } else {
                        from.to_owned()
                    },
                    if subject.is_empty() {
                        locale.no_subject()
                    } else {
                        subject.to_owned()
                    },
                )
            }
            [] => (self.account_name.clone(), locale.new_mail(self.count)),
            messages => {
                let mut senders: Vec<&str> = Vec::new();
                for message in messages {
                    let sender = message
                        .from
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&message.from.address);
                    if !sender.is_empty() && !senders.contains(&sender) {
                        senders.push(sender);
                    }
                }
                let body = if senders.is_empty() {
                    locale.new_mail(self.count)
                } else {
                    locale.new_mail_from(
                        self.count,
                        &senders.join("、"),
                        self.count as usize > senders.len(),
                    )
                };
                (self.account_name.clone(), body)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, name: Option<&str>, address: &str, subject: &str) -> NewMailMessage {
        NewMailMessage {
            email_id: id.into(),
            from: MailAddress {
                name: name.map(Into::into),
                address: address.into(),
            },
            subject: subject.into(),
            received_at: "2026-10-22T08:00:00.000Z".into(),
        }
    }

    fn mail(count: u32, messages: Vec<NewMailMessage>) -> NewMail {
        NewMail {
            account_id: "a1".into(),
            account_name: "工作".into(),
            count,
            messages,
        }
    }

    #[test]
    fn one_message_names_its_sender_and_subject() {
        let one = mail(1, vec![message("e", Some("Lin"), "lin@x.io", " 周报 ")]);
        assert_eq!(one.summary(), ("Lin".into(), "周报".into()));
        let bare = mail(1, vec![message("e", Some(" "), "lin@x.io", "")]);
        assert_eq!(bare.summary(), ("lin@x.io".into(), "（无主题）".into()));
    }

    #[test]
    fn several_messages_are_summed_up_per_account_without_repeating_senders() {
        let many = mail(
            5,
            vec![
                message("1", Some("Lin"), "lin@x.io", "a"),
                message("2", None, "ci@github.com", "b"),
                message("3", Some("Lin"), "lin@x.io", "c"),
            ],
        );
        assert_eq!(
            many.summary(),
            (
                "工作".into(),
                "5 封新邮件，来自 Lin、ci@github.com 等".into()
            )
        );
        let exact = mail(
            2,
            vec![
                message("1", Some("A"), "a@x.io", "a"),
                message("2", Some("B"), "b@x.io", "b"),
            ],
        );
        assert_eq!(exact.summary().1, "2 封新邮件，来自 A、B");
    }

    #[test]
    fn delivery_honors_the_switch_muted_accounts_and_hidden_previews() {
        let one = mail(1, vec![message("e", Some("Lin"), "lin@x.io", "周报")]);
        let settings = NotificationSettings::default();
        assert_eq!(one.for_delivery(&settings), Some(one.clone()));

        let off = NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        };
        assert_eq!(one.for_delivery(&off), None);

        let muted = NotificationSettings {
            muted_accounts: vec!["a1".into()],
            ..NotificationSettings::default()
        };
        assert_eq!(one.for_delivery(&muted), None);

        let private = NotificationSettings {
            show_preview: false,
            ..NotificationSettings::default()
        };
        let hidden = one.for_delivery(&private).unwrap();
        assert!(hidden.messages.is_empty());
        // Nothing about the sender or subject is left in what goes out.
        let text = serde_json::to_string(&hidden).unwrap();
        assert!(!text.contains("Lin") && !text.contains("周报"), "{text}");
        assert_eq!(hidden.summary(), ("工作".into(), "1 封新邮件".into()));
    }
}
