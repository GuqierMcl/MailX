//! Clients for the supported translation services. Hosts, paths and headers are
//! fixed per service; the only inputs are the key, the Azure region and the
//! route. An answer is just a list of strings: whether it fits its source
//! segment is decided later by `translate::apply`.
use crate::{ErrorCode, net::NetRoute, secrets::SecretValue, settings::TranslationProviderId};

mod deepl;
mod google;
mod http;
mod microsoft;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use http::Base;

/// An answer longer than `RATIO * source + SLACK` bytes is not a translation.
const MAX_GROWTH_RATIO: usize = 10;
const MAX_GROWTH_SLACK: usize = 1000;

pub(crate) struct TranslateRequest<'a> {
    /// Wire strings (see `translate::wire`) or plain escaped text.
    pub texts: &'a [String],
    /// A MailX code from `translate::languages::TARGETS`.
    pub target: &'a str,
}

pub(crate) struct TranslateResponse {
    pub texts: Vec<String>,
    /// Lowercase code the service reported for the source, if any.
    pub detected_source: Option<String>,
}

/// What one HTTP request may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_items: usize,
    /// UTF-8 bytes of the wire strings, markup included, before JSON quoting
    /// and escaping; budgets sit below the documented caps to leave room for that.
    pub max_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct Context<'a> {
    pub key: &'a SecretValue,
    /// Azure resource region; empty for a global resource.
    pub microsoft_region: &'a str,
    pub route: &'a NetRoute,
    /// Tests point the client at a local plain-HTTP listener.
    #[cfg(test)]
    pub base: Option<Base>,
}

impl Context<'_> {
    fn base_or(&self, default: impl FnOnce() -> http::Base) -> http::Base {
        #[cfg(test)]
        if let Some(base) = &self.base {
            return base.clone();
        }
        default()
    }
}

pub(crate) fn limits(provider: TranslationProviderId) -> Limits {
    match provider {
        // Documented cap: 128 KiB request body; 100,000 bytes of text leave
        // room for quoting and escaping. The item count is not documented
        // (UNVERIFIED: 50 is the long-standing value).
        TranslationProviderId::Deepl => Limits {
            max_items: 50,
            max_bytes: 100_000,
        },
        // Documented cap: 1000 elements and 50,000 characters per request;
        // bytes are an upper bound for characters, so 40,000 stays safe.
        TranslationProviderId::Microsoft => Limits {
            max_items: 1000,
            max_bytes: 40_000,
        },
        // Documented cap: 128 strings and 100,000 bytes per request body;
        // 80,000 bytes of text leave room for quoting and escaping.
        TranslationProviderId::Google => Limits {
            max_items: 128,
            max_bytes: 80_000,
        },
    }
}

pub(crate) async fn translate(
    provider: TranslationProviderId,
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    if request.texts.is_empty() {
        return Ok(TranslateResponse {
            texts: Vec::new(),
            detected_source: None,
        });
    }
    let response = match provider {
        TranslationProviderId::Deepl => deepl::translate(context, request).await,
        TranslationProviderId::Microsoft => microsoft::translate(context, request).await,
        TranslationProviderId::Google => google::translate(context, request).await,
    }?;
    check_shape(request.texts, &response)?;
    Ok(response)
}

fn check_shape(sources: &[String], response: &TranslateResponse) -> Result<(), ErrorCode> {
    if response.texts.len() != sources.len() {
        return Err(ErrorCode::TranslationInvalidResponse);
    }
    let oversized = sources
        .iter()
        .zip(&response.texts)
        .any(|(source, text)| text.len() > source.len() * MAX_GROWTH_RATIO + MAX_GROWTH_SLACK);
    if oversized {
        return Err(ErrorCode::TranslationInvalidResponse);
    }
    Ok(())
}

/// The mapped service code, or `LanguageUnsupported` for a code MailX does not offer.
fn require_target(code: Option<&'static str>) -> Result<&'static str, ErrorCode> {
    code.ok_or(ErrorCode::TranslationLanguageUnsupported)
}

fn normalize_detected(code: Option<String>) -> Option<String> {
    code.map(|code| code.trim().to_ascii_lowercase())
        .filter(|code| !code.is_empty() && code.len() <= 16)
}
