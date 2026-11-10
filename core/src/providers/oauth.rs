//! The reviewed OAuth provider list. Provider-specific registrations and
//! identity verifiers are added here and nowhere else; `crate::auth` only
//! defines the neutral flow and the [`OAuthProvider`] trait.
use super::microsoft;
use crate::auth::{OAuthError, OAuthProvider};

/// Every reviewed entry. A provider entry needs a reviewed registration, an
/// identity verifier and real send/receive evidence before it is listed.
/// Entries are identified by `registration().id`, which must be unique even
/// when several entries share one client ID.
fn providers() -> Vec<&'static dyn OAuthProvider> {
    microsoft::providers().to_vec()
}

/// The only way auth and app code reach a provider entry.
pub(crate) fn lookup(id: &str) -> Result<&'static dyn OAuthProvider, OAuthError> {
    lookup_in(&providers(), id)
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
    fn unknown_ids_are_unsupported() {
        for id in ["", "unknown", "https://untrusted.example/authorize"] {
            assert!(matches!(lookup(id), Err(OAuthError::UnsupportedProvider)));
        }
    }

    #[test]
    fn the_list_has_the_reviewed_entries_in_order() {
        let ids: Vec<_> = providers()
            .iter()
            .map(|provider| provider.registration().id.clone())
            .collect();
        assert_eq!(ids, ["microsoft-personal", "microsoft-work"]);
        for id in &ids {
            assert_eq!(&lookup(id).unwrap().registration().id, id);
        }
    }

    #[test]
    fn listed_entries_are_unique_and_valid() {
        let mut seen = std::collections::HashSet::new();
        let mut registrations = std::collections::HashSet::new();
        for provider in providers() {
            let registration = provider.registration();
            registration.validate().unwrap();
            assert!(seen.insert(registration.id.clone()));
            assert!(registrations.insert(registration.registration_id.clone()));
        }
    }
}
