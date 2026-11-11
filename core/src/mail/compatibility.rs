//! Optional adaptations for provider-specific message headers.
//!
//! The sync and storage paths consume this interface without knowing provider
//! names or private header fields. Implementations are selected from the actual
//! IMAP endpoint, never from an account label or sender address.

/// A versioned interpretation of metadata returned by an IMAP endpoint.
pub(crate) trait ProviderCompatibility: Send + Sync {
    /// Changes whenever stored header metadata needs to be fetched again.
    fn policy_key(&self) -> &'static str;

    /// Additional, trusted header field names to request alongside RFC headers.
    fn extra_header_fields(&self) -> &'static [&'static str];

    /// An original Message-ID retained by the provider, without angle brackets.
    /// Implementations must reject ambiguous input and preserve case.
    fn original_message_id(&self, message: &mail_parser::Message<'_>) -> Option<String>;
}

pub(crate) struct StandardCompatibility;

impl ProviderCompatibility for StandardCompatibility {
    fn policy_key(&self) -> &'static str {
        // v2: headers now include In-Reply-To and References for conversation
        // threading, so cached messages are fetched again once.
        "default:v2"
    }

    fn extra_header_fields(&self) -> &'static [&'static str] {
        &[]
    }

    fn original_message_id(&self, _message: &mail_parser::Message<'_>) -> Option<String> {
        None
    }
}
