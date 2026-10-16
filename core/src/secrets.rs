//! Host-owned secret storage. No secret-reading application command is exposed.
use serde::{Deserialize, Deserializer};
use std::fmt;
use zeroize::Zeroizing;

/// An input/backend-only secret; intentionally does not implement Serialize or TS.
#[derive(Clone)]
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for SecretValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretStoreError {
    Locked,
    Unavailable,
    Failed,
}

/// `Ok(None)` means a confirmed absent entry, never temporary unavailability.
/// Implementations must persist `set` before returning, and make `delete` idempotent.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError>;
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError>;
    fn delete(&self, key: &str) -> Result<(), SecretStoreError>;
}
