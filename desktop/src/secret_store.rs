use std::sync::Arc;

use mailx_core::secrets::{SecretStore, SecretStoreError, SecretValue};

/// Only native, durable credential stores are enabled in Cargo.toml. Never use
/// keyring's mock/default fallback as a production backend.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("MailX needs a native SecretStore adapter on this platform");

/// The credential store the application uses. macOS asks for permission per
/// keychain item, so there every secret lives in one item (see `secret_vault`);
/// Windows and Linux have no such per-item prompt and keep one item per secret.
pub(crate) fn system_store(service: String) -> Arc<dyn SecretStore> {
    let per_item = SystemSecretStore::new(service);
    #[cfg(target_os = "macos")]
    {
        Arc::new(crate::secret_vault::VaultSecretStore::new(Arc::new(
            per_item,
        )))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(per_item)
    }
}

pub(crate) struct SystemSecretStore {
    service: String,
}

impl SystemSecretStore {
    pub(crate) fn new(service: String) -> Self {
        Self { service }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretStoreError> {
        keyring::Entry::new(&self.service, key).map_err(map_error)
    }
}

impl SecretStore for SystemSecretStore {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(SecretValue::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        self.entry(key)?
            .set_password(value.expose_secret())
            .map_err(map_error)
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }
}

fn map_error(error: keyring::Error) -> SecretStoreError {
    // The provider's Display/Debug and BadEncoding payload may contain secrets.
    // Inspect only typed status codes; never forward or log the source error.
    #[cfg(target_os = "macos")]
    if let keyring::Error::PlatformFailure(source) | keyring::Error::NoStorageAccess(source) =
        &error
        && let Some(status) = source.downcast_ref::<security_framework::base::Error>()
        && matches!(status.code(), -25308 | -25293 | -128)
    {
        return SecretStoreError::Locked;
    }
    match error {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretStoreError::Unavailable
        }
        _ => SecretStoreError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_are_redacted() {
        let error = map_error(keyring::Error::BadEncoding(b"synthetic-secret".to_vec()));
        assert_eq!(error, SecretStoreError::Failed);
        assert!(!format!("{error:?}").contains("synthetic-secret"));
        let error = map_error(keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("synthetic-secret"),
        )));
        assert_eq!(error, SecretStoreError::Unavailable);
    }

    /// Explicit opt-in: the vault on top of the real macOS keychain. It writes a
    /// synthetic legacy item, then checks the lazy migration, the next-start
    /// cleanup and deletion, and removes everything it created.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an unlocked native credential store"]
    fn native_vault_migrates_and_cleans_up() {
        use crate::secret_vault::VaultSecretStore;
        let service = format!("com.mailxapp.test.{}", uuid::Uuid::new_v4());
        let per_item = || Arc::new(SystemSecretStore::new(service.clone()));
        let key = "mailx/synthetic/entry";
        let secret = SecretValue::new(uuid::Uuid::new_v4().to_string());
        let result = (|| {
            per_item().set(key, &secret)?;
            let first = VaultSecretStore::new(per_item());
            assert_eq!(
                first
                    .get(key)?
                    .ok_or(SecretStoreError::Failed)?
                    .expose_secret(),
                secret.expose_secret()
            );
            assert!(
                per_item().get(key)?.is_some(),
                "old item kept for one start"
            );
            let second = VaultSecretStore::new(per_item());
            assert_eq!(
                second
                    .get(key)?
                    .ok_or(SecretStoreError::Failed)?
                    .expose_secret(),
                secret.expose_secret()
            );
            assert!(
                per_item().get(key)?.is_none(),
                "old item removed next start"
            );
            second.delete(key)?;
            assert!(VaultSecretStore::new(per_item()).get(key)?.is_none());
            Ok::<_, SecretStoreError>(())
        })();
        // Cleanup of the synthetic items, whatever happened above.
        let raw = per_item();
        raw.delete(key).expect("synthetic item removed");
        raw.delete("vault").expect("synthetic vault removed");
        result.expect("native vault roundtrip succeeded");
    }

    /// Explicit opt-in: creates and removes a synthetic native-store entry.
    #[test]
    #[ignore = "requires an unlocked native credential store"]
    fn native_secret_store_roundtrip() {
        let store = SystemSecretStore::new(format!("com.mailxapp.test.{}", uuid::Uuid::new_v4()));
        let key = "synthetic-entry";
        let value = SecretValue::new(uuid::Uuid::new_v4().to_string());
        assert!(store.get(key).expect("native store readable").is_none());
        let result = (|| {
            store.set(key, &value)?;
            let restored = store.get(key)?.ok_or(SecretStoreError::Failed)?;
            assert_eq!(restored.expose_secret(), value.expose_secret());
            Ok::<_, SecretStoreError>(())
        })();
        store.delete(key).expect("synthetic credential removed");
        store.delete(key).expect("delete is idempotent");
        assert!(store.get(key).expect("native store readable").is_none());
        result.expect("native roundtrip succeeded");
    }
}
