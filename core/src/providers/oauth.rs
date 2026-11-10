//! The reviewed OAuth provider list. Provider-specific registrations and
//! identity verifiers are added here and nowhere else; `crate::auth` only
//! defines the neutral flow and the [`OAuthProvider`] trait.
use crate::auth::{OAuthError, OAuthProvider};

/// No provider ships yet. A provider entry needs a reviewed registration, an
/// identity verifier and real send/receive evidence before it is listed.
/// Entries are identified by `registration().id`, which must be unique even
/// when several entries share one client ID.
static PROVIDERS: &[&'static dyn OAuthProvider] = &[];

/// The only way auth and app code reach a provider entry.
pub(crate) fn lookup(id: &str) -> Result<&'static dyn OAuthProvider, OAuthError> {
    lookup_in(PROVIDERS, id)
}

fn lookup_in<'a>(
    providers: &[&'a dyn OAuthProvider],
    id: &str,
) -> Result<&'a dyn OAuthProvider, OAuthError> {
    providers
        .iter()
        .copied()
        .find(|provider| provider.registration().id == id)
        .ok_or(OAuthError::UnsupportedProvider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_list_is_empty_and_unknown_ids_are_unsupported() {
        assert!(PROVIDERS.is_empty());
        for id in ["", "unknown", "https://untrusted.example/authorize"] {
            assert!(matches!(lookup(id), Err(OAuthError::UnsupportedProvider)));
        }
    }

    #[test]
    fn listed_entries_are_unique_and_valid() {
        let mut seen = std::collections::HashSet::new();
        for provider in PROVIDERS {
            let registration = provider.registration();
            registration.validate().unwrap();
            assert!(seen.insert(registration.id.clone()));
        }
    }
}
