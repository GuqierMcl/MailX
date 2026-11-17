//! How much a model may think before it answers, and what each provider wants
//! in the request body to say so. This is the one place that knows about
//! provider differences; the protocol clients only add what it returns.
//!
//! Thinking models count their reasoning tokens against the output cap, so a
//! short answer (the Now page's line) can run out of tokens before any text
//! appears. Servers that do not know a parameter may reject the request, so a
//! provider is listed here only once its documentation has been checked.

use serde_json::{Value, json};

use super::settings::AiProtocol;

/// What a request asks of the model's thinking. A request, not a guarantee: a
/// provider without a known switch still thinks as it pleases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Reasoning {
    /// Send nothing; the model keeps its own default.
    #[default]
    Default,
    /// As little thinking as the provider allows.
    Off,
}

/// Extra top-level body fields for `reasoning` (name, value); empty when the
/// provider has no verified switch or nothing is asked.
///
/// - `deepseek` on the OpenAI-compatible protocol: `thinking: {type: disabled}`
///   (<https://api-docs.deepseek.com/guides/thinking_mode>).
/// - Anthropic protocol: thinking is off unless requested, so nothing is sent.
pub(crate) fn body_fields(
    protocol: AiProtocol,
    provider_id: &str,
    reasoning: Reasoning,
) -> Vec<(&'static str, Value)> {
    match (reasoning, protocol, provider_id) {
        (Reasoning::Off, AiProtocol::OpenaiCompatible, "deepseek") => {
            vec![("thinking", json!({"type": "disabled"}))]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_verified_provider_gets_a_field_and_only_when_off() {
        let off = |protocol, id| body_fields(protocol, id, Reasoning::Off);
        assert_eq!(
            off(AiProtocol::OpenaiCompatible, "deepseek"),
            vec![("thinking", json!({"type": "disabled"}))]
        );
        assert!(
            body_fields(AiProtocol::OpenaiCompatible, "deepseek", Reasoning::Default).is_empty()
        );
        assert!(off(AiProtocol::OpenaiCompatible, "openai").is_empty());
        assert!(off(AiProtocol::OpenaiCompatible, "custom").is_empty());
        assert!(off(AiProtocol::Anthropic, "deepseek").is_empty());
    }
}
