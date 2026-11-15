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
        Ok(Self {
            model,
            consented,
            excluded_accounts,
            instructions,
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
        "enabled" => v.is_boolean(),
        "model" => v.is_null() || serde_json::from_value::<AiModelRef>(v.clone()).is_ok(),
        "consented" => v.is_null() || serde_json::from_value::<AiConsent>(v.clone()).is_ok(),
        "excludedAccounts" => v
            .as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string)),
        "organize" | "aiDrafts" => serde_json::from_value::<AssistantMode>(v.clone()).is_ok(),
        "send" => serde_json::from_value::<AssistantSendMode>(v.clone()).is_ok(),
        "instructions" => v.is_string(),
        _ => true,
    });
}

#[cfg(test)]
mod tests;
