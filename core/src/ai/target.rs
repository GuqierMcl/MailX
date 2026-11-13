//! Everything one AI call needs, resolved from the settings: where it goes,
//! what it signs with and which model it asks. Built by `AiService`; the
//! translation clients only consume it.

use std::fmt;

use super::settings::AiProtocol;
use crate::{net::NetRoute, secrets::SecretValue, settings::AiConsent};

pub struct AiModelTarget {
    /// The connection id and normalized base URL; what consent binds to.
    pub binding: AiConsent,
    pub(crate) protocol: AiProtocol,
    pub(crate) model_id: String,
    /// The model's output limit in tokens, when the catalog knows it.
    pub(crate) output_length: Option<u32>,
    /// Empty for a connection without a key (local servers).
    pub(crate) key: SecretValue,
    pub(crate) route: NetRoute,
}

impl fmt::Debug for AiModelTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiModelTarget")
            .field("binding", &self.binding)
            .field("protocol", &self.protocol)
            .field("model_id", &self.model_id)
            .field("output_length", &self.output_length)
            .finish_non_exhaustive()
    }
}

impl AiModelTarget {
    pub(crate) fn new(
        binding: AiConsent,
        protocol: AiProtocol,
        model_id: String,
        output_length: Option<u32>,
        key: String,
        route: NetRoute,
    ) -> Self {
        Self {
            binding,
            protocol,
            model_id,
            output_length,
            key: SecretValue::new(key),
            route,
        }
    }
}
