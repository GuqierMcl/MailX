use mailx_core::secrets::{SecretStore, SecretStoreError, SecretValue};

/// Only native, durable credential stores are enabled in Cargo.toml. Never use
/// keyring's mock/default fallback as a production backend.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("MailX needs a native SecretStore adapter on this platform");

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

    /// Explicit opt-in: creates and removes a synthetic native-store entry.
    #[test]
    #[ignore = "requires an unlocked native credential store"]
    fn native_secret_store_roundtrip() {
        let store = SystemSecretStore::new(format!("dev.mailx.test.{}", uuid::Uuid::new_v4()));
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
