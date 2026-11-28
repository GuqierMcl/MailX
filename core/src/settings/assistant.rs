//! The built-in assistant's settings (`docs/ai-runtime/README.md` section 7).
//!
//! Changed through the normal settings patch like every other section. The
//! consent is the only field the assistant service also writes (when the host
//! grants or revokes it).
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::AiConsent;
use crate::{AppError, ai::AiModelRef};

/// Custom instructions, in characters.
pub const MAX_INSTRUCTIONS_CHARS: usize = 2000;
const MAX_EXCLUDED_ACCOUNTS: usize = 64;
/// Senders whose mail is always summarized.
pub const MAX_SUMMARY_SENDERS: usize = 500;
const MAX_ADDRESS_CHARS: usize = 320;

/// How an action category the assistant may do on its own is handled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AssistantMode {
    /// Run without asking.
    #[default]
    Direct,
    /// Ask the user for every call.
    Ask,
}

/// How sending (and cancelling a queued send) is handled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AssistantSendMode {
    /// Ask the user for every send.
    #[default]
    Confirm,
    /// Send without asking.
    Direct,
}

/// How long an email summary is (`docs/architecture/ai-chat-ui.md` section 5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SummaryStyle {
    /// One or two sentences.
    #[default]
    Brief,
    /// A short paragraph.
    Detailed,
    /// Bullet points, then what the reader needs to do.
    Points,
}

impl SummaryStyle {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Detailed => "detailed",
            Self::Points => "points",
        }
    }
}

/// The language an email summary is written in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SummaryLanguage {
    /// The interface language.
    #[default]
    Interface,
    /// The language the email is written in.
    Original,
    /// One of the translation target codes (`translate::languages::TARGETS`).
    Fixed { code: String },
}

impl SummaryLanguage {
    fn valid(&self) -> bool {
        match self {
            Self::Fixed { code } => crate::translate::languages::is_target(code),
            Self::Interface | Self::Original => true,
        }
    }
}

/// Email summaries in the reader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailSummarySettings {
    /// Summarize every email the user opens. Off by default: it sends each
    /// opened message to the model.
    pub auto: bool,
    /// Lowercased sender addresses whose mail is always summarized, whatever
    /// `auto` says.
    pub senders: Vec<String>,
    /// The style the panel last used.
    pub style: SummaryStyle,
    /// The language the panel last used.
    pub language: SummaryLanguage,
    /// The summary card shows only its first line.
    pub collapsed: bool,
}

/// AI action-item (calendar event / task) extraction in the reader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionItemSettings {
    /// Automatically extract events and tasks when L1 local rules hit. Off by default.
    pub auto_extract: bool,
    /// User chose "Do not ask again" on the first-hit guidance banner.
    pub prompt_dismissed: bool,
}

fn canonical_address(address: &str) -> Option<String> {
    let address = address.trim().to_lowercase();
    let (local, domain) = address.split_once('@')?;
    (!local.is_empty()
        && !domain.is_empty()
        && address.chars().count() <= MAX_ADDRESS_CHARS
        && !address.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then_some(address)
}

impl EmailSummarySettings {
    fn normalized(self) -> Result<Self, AppError> {
        let mut seen = HashSet::new();
        let mut senders = Vec::with_capacity(self.senders.len());
        for sender in &self.senders {
            let sender = canonical_address(sender)
                .ok_or_else(|| AppError::validation("assistant.summary.senders"))?;
            if seen.insert(sender.clone()) {
                senders.push(sender);
            }
        }
        if senders.len() > MAX_SUMMARY_SENDERS {
            return Err(AppError::validation("assistant.summary.senders"));
        }
        if !self.language.valid() {
            return Err(AppError::validation("assistant.summary.language"));
        }
        Ok(Self { senders, ..self })
    }

    fn sanitized(self) -> Self {
        let mut seen = HashSet::new();
        let senders = self
            .senders
            .iter()
            .filter_map(|sender| canonical_address(sender))
            .filter(|sender| seen.insert(sender.clone()))
            .take(MAX_SUMMARY_SENDERS)
            .collect();
        let language = if self.language.valid() {
            self.language
        } else {
            SummaryLanguage::Interface
        };
        Self {
            senders,
            language,
            ..self
        }
    }

    /// Whether mail from `address` is always summarized.
    pub fn always_for(&self, address: &str) -> bool {
        let address = address.trim().to_lowercase();
        self.senders.contains(&address)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistantSettings {
    /// Off: no run starts, whatever asks for it.
    pub enabled: bool,
    /// `None` follows `ai.defaultModel`.
    pub model: Option<AiModelRef>,
    /// The connection and base URL the user agreed to send mail content to.
    pub consented: Option<AiConsent>,
    /// Account ids the assistant cannot see.
    pub excluded_accounts: Vec<String>,
    /// Flags, archive and refresh.
    pub organize: AssistantMode,
    /// Changes to drafts the assistant created itself.
    pub ai_drafts: AssistantMode,
    pub send: AssistantSendMode,
    /// Appended to the system prompt, at most 2000 characters.
    pub instructions: String,
    /// Keep chat conversations on this device so they can be reopened.
    /// Turning it off stops saving new turns; what is already saved stays
    /// until the user deletes it.
    pub save_history: bool,
    /// Let the assistant read a mail's HTML structure (headings, lists,
    /// tables, links) instead of its plain text only. Off by default: it sends
    /// far more of the message, including markup other people wrote.
    pub read_html: bool,
    /// Email summaries in the reader.
    pub summary: EmailSummarySettings,
    /// Calendar event and task extraction in the reader.
    pub action_items: ActionItemSettings,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            consented: None,
            excluded_accounts: Vec::new(),
            organize: AssistantMode::Direct,
            ai_drafts: AssistantMode::Direct,
            send: AssistantSendMode::Confirm,
            instructions: String::new(),
            save_history: true,
            read_html: false,
            summary: EmailSummarySettings::default(),
            action_items: ActionItemSettings::default(),
        }
    }
}

fn canonical_accounts(ids: &[String]) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let id = Uuid::parse_str(id).ok()?.to_string();
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    (out.len() <= MAX_EXCLUDED_ACCOUNTS).then_some(out)
}

impl AssistantSettings {
    /// Validates client input and returns its canonical form.
    pub(crate) fn normalized(self) -> Result<Self, AppError> {
        let model = match self.model {
            Some(model) => Some(
                model
                    .normalized()
                    .ok_or_else(|| AppError::validation("assistant.model"))?,
            ),
            None => None,
        };
        let consented = match self.consented {
            Some(consent) => Some(
                consent
                    .normalized()
                    .ok_or_else(|| AppError::validation("assistant.consented"))?,
            ),
            None => None,
        };
        let excluded_accounts = canonical_accounts(&self.excluded_accounts)
            .ok_or_else(|| AppError::validation("assistant.excludedAccounts"))?;
        let instructions = self.instructions.trim().to_owned();
        if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            return Err(AppError::validation("assistant.instructions"));
        }
        let summary = self.summary.normalized()?;
        Ok(Self {
            model,
            consented,
            excluded_accounts,
            instructions,
            summary,
            ..self
        })
    }

    /// For stored files: keeps what is usable and truncates what is too long.
    pub(crate) fn sanitized(self) -> Self {
        let mut seen = HashSet::new();
        let excluded_accounts = self
            .excluded_accounts
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .map(|id| id.to_string())
            .filter(|id| seen.insert(id.clone()))
            .take(MAX_EXCLUDED_ACCOUNTS)
            .collect();
        Self {
            model: self.model.and_then(AiModelRef::normalized),
            consented: self.consented.and_then(AiConsent::normalized),
            excluded_accounts,
            instructions: self
                .instructions
                .trim()
                .chars()
                .take(MAX_INSTRUCTIONS_CHARS)
                .collect::<String>()
                .trim()
                .to_owned(),
            summary: self.summary.sanitized(),
            ..self
        }
    }
}

/// Drops fields of the wrong type or with unknown values, so a hand-edited
/// file loses only that field.
pub(super) fn repair_assistant(value: &mut serde_json::Value) {
    let Some(section) = value.as_object_mut() else {
        return;
    };
    section.retain(|key, v| match key.as_str() {
        "enabled" | "saveHistory" | "readHtml" => v.is_boolean(),
        "model" => v.is_null() || serde_json::from_value::<AiModelRef>(v.clone()).is_ok(),
        "consented" => v.is_null() || serde_json::from_value::<AiConsent>(v.clone()).is_ok(),
        "excludedAccounts" => v
            .as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string)),
        "organize" | "aiDrafts" => serde_json::from_value::<AssistantMode>(v.clone()).is_ok(),
        "send" => serde_json::from_value::<AssistantSendMode>(v.clone()).is_ok(),
        "instructions" => v.is_string(),
        "summary" => repair_summary(v),
        "actionItems" => repair_action_items(v),
        _ => true,
    });
}

fn repair_action_items(value: &mut serde_json::Value) -> bool {
    let Some(section) = value.as_object_mut() else {
        return false;
    };
    section.retain(|key, v| match key.as_str() {
        "autoExtract" | "promptDismissed" => v.is_boolean(),
        _ => true,
    });
    true
}

/// Keeps a usable `summary` section, dropping its broken fields; `false`
/// drops the whole section.
fn repair_summary(value: &mut serde_json::Value) -> bool {
    let Some(section) = value.as_object_mut() else {
        return false;
    };
    section.retain(|key, v| match key.as_str() {
        "auto" | "collapsed" => v.is_boolean(),
        "senders" => v
            .as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string)),
        "style" => serde_json::from_value::<SummaryStyle>(v.clone()).is_ok(),
        "language" => serde_json::from_value::<SummaryLanguage>(v.clone()).is_ok(),
        _ => true,
    });
    true
}

#[cfg(test)]
mod tests;
