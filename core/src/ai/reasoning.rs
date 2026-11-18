//! What a thinking model reasoned before it answered. The protocol decoders
//! capture it as it streams; the assistant shows it, keeps it with the reply
//! and sends it back on later requests, because some servers require that for
//! tool calls (they reject the next request without it).
//!
//! The rule is protocol-level and has no provider list: send back exactly what
//! this server sent, in the form it sent it, and nothing else.
//!
//! - OpenAI-compatible: the text arrives in `delta.reasoning_content` or
//!   `delta.reasoning`; it goes back on the same assistant message under the
//!   same field name. A server that never sent a field never gets one.
//! - Anthropic: `thinking` blocks (text and `signature`) and
//!   `redacted_thinking` blocks (opaque `data`) go back unchanged, in their
//!   original place among the reply's text and `tool_use` blocks.
//!
//! Signatures and opaque data belong to the server and model that made them,
//! so reasoning is only sent back to the same protocol, connection, address
//! and model ([`ReasoningOrigin`]); after a switch it is left out.

use serde::{Deserialize, Serialize};

use super::settings::AiProtocol;

/// Where a reply's reasoning came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningOrigin {
    pub protocol: AiProtocol,
    pub connection_id: String,
    pub base_url: String,
    pub model: String,
}

/// The OpenAI-compatible delta field the reasoning arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReasoningField {
    /// `reasoning_content` (DeepSeek and others).
    #[serde(rename = "reasoning_content")]
    ReasoningContent,
    /// `reasoning` as a string (OpenRouter, vLLM and others).
    #[serde(rename = "reasoning")]
    Reasoning,
}

impl ReasoningField {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ReasoningContent => "reasoning_content",
            Self::Reasoning => "reasoning",
        }
    }
}

/// One content block of an Anthropic reply, in the order received. Text and
/// tool use only mark their place (the text and calls live on the message);
/// thinking blocks carry everything the server sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum AnthropicPart {
    Thinking {
        thinking: String,
        /// `None` when the server sent no signature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
    /// The next `bytes` bytes of the message text.
    Text {
        bytes: usize,
    },
    ToolUse {
        id: String,
    },
}

/// Opaque data to send back, exactly as the server sent it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ReasoningEcho {
    /// OpenAI-compatible: one text field on the assistant message.
    Field { field: ReasoningField, text: String },
    /// Anthropic: the reply's blocks in order, at least one of them thinking.
    Blocks { parts: Vec<AnthropicPart> },
}

impl ReasoningEcho {
    /// The readable reasoning, in order; empty for redacted thinking.
    pub(crate) fn shown_text(&self) -> String {
        match self {
            Self::Field { text, .. } => text.clone(),
            Self::Blocks { parts } => parts
                .iter()
                .filter_map(|part| match part {
                    AnthropicPart::Thinking { thinking, .. } => Some(thinking.as_str()),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Everything that goes back to the server, for the context estimate.
    pub(crate) fn sent_text(&self) -> String {
        match self {
            Self::Field { text, .. } => text.clone(),
            Self::Blocks { parts } => parts
                .iter()
                .map(|part| match part {
                    AnthropicPart::Thinking {
                        thinking,
                        signature,
                    } => format!("{thinking}{}", signature.as_deref().unwrap_or("")),
                    AnthropicPart::RedactedThinking { data } => data.clone(),
                    AnthropicPart::Text { .. } | AnthropicPart::ToolUse { .. } => String::new(),
                })
                .collect(),
        }
    }
}

/// One stretch of thinking as the interface showed it, for saved replies.
/// Positions are within the assistant message the reasoning belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningSpan {
    /// Bytes of [`ReasoningEcho::shown_text`] this stretch covers, following
    /// the previous stretches.
    pub bytes: usize,
    pub duration_ms: u64,
    /// Bytes of the message text written before it began.
    pub text_at: usize,
    /// Tool calls of the message that came before it.
    pub tools_before: usize,
}

/// The reasoning of one assistant message: opaque echo data plus what the
/// interface needs to show it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantReasoning {
    pub origin: ReasoningOrigin,
    /// `None` when nothing came that could be sent back (empty reasoning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<ReasoningEcho>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<ReasoningSpan>,
}

impl AssistantReasoning {
    /// What to send back on a request to `origin`; `None` for another server
    /// or model.
    pub(crate) fn echo_for(&self, origin: &ReasoningOrigin) -> Option<&ReasoningEcho> {
        (self.origin == *origin)
            .then_some(self.echo.as_ref())
            .flatten()
    }

    /// Each stretch with its readable text.
    pub(crate) fn shown(&self) -> Vec<(&ReasoningSpan, String)> {
        let text = self
            .echo
            .as_ref()
            .map(ReasoningEcho::shown_text)
            .unwrap_or_default();
        let mut from = 0usize;
        self.spans
            .iter()
            .map(|span| {
                let to = from.saturating_add(span.bytes);
                let part = text.get(from..to).unwrap_or_default().to_owned();
                from = to;
                (span, part)
            })
            .collect()
    }
}

/// `reasoning` of a message, when it may go to `origin`.
pub(crate) fn echo_for<'a>(
    reasoning: Option<&'a AssistantReasoning>,
    origin: &ReasoningOrigin,
) -> Option<&'a ReasoningEcho> {
    reasoning.and_then(|r| r.echo_for(origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(model: &str) -> ReasoningOrigin {
        ReasoningOrigin {
            protocol: AiProtocol::OpenaiCompatible,
            connection_id: "c".into(),
            base_url: "https://x/v1".into(),
            model: model.into(),
        }
    }

    #[test]
    fn echo_only_goes_back_to_the_same_origin() {
        let reasoning = AssistantReasoning {
            origin: origin("a"),
            echo: Some(ReasoningEcho::Field {
                field: ReasoningField::ReasoningContent,
                text: "hm".into(),
            }),
            spans: Vec::new(),
        };
        assert!(reasoning.echo_for(&origin("a")).is_some());
        assert!(reasoning.echo_for(&origin("b")).is_none());
        let mut other = origin("a");
        other.protocol = AiProtocol::Anthropic;
        assert!(reasoning.echo_for(&other).is_none());
        let mut other = origin("a");
        other.connection_id = "d".into();
        assert!(reasoning.echo_for(&other).is_none());
        let mut other = origin("a");
        other.base_url = "https://y/v1".into();
        assert!(reasoning.echo_for(&other).is_none());
    }

    #[test]
    fn stretches_split_the_shown_text_and_redacted_shows_nothing() {
        let reasoning = AssistantReasoning {
            origin: origin("a"),
            echo: Some(ReasoningEcho::Blocks {
                parts: vec![
                    AnthropicPart::Thinking {
                        thinking: "想一下".into(),
                        signature: Some("sig".into()),
                    },
                    AnthropicPart::ToolUse { id: "t".into() },
                    AnthropicPart::RedactedThinking { data: "x".into() },
                    AnthropicPart::Thinking {
                        thinking: "again".into(),
                        signature: None,
                    },
                ],
            }),
            spans: vec![
                ReasoningSpan {
                    bytes: 9,
                    duration_ms: 1,
                    text_at: 0,
                    tools_before: 0,
                },
                ReasoningSpan {
                    bytes: 5,
                    duration_ms: 2,
                    text_at: 0,
                    tools_before: 1,
                },
            ],
        };
        let shown: Vec<String> = reasoning.shown().into_iter().map(|(_, t)| t).collect();
        assert_eq!(shown, ["想一下", "again"]);
        let echo = reasoning.echo.as_ref().unwrap();
        assert_eq!(echo.sent_text(), "想一下sigxagain");
    }
}
