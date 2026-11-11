//! Contact aggregation rules (see `storage/contacts.rs`).
use super::AccountService;
use crate::{AppError, mail::ContactAggregation};

/// A sender address as stored and compared: trimmed and ASCII-lowercased.
/// Rejects values that cannot be an address before SQL sees them.
pub(crate) fn normalize_sender_address(value: &str, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > 320
        || !value.contains('@')
        || value.chars().any(char::is_control)
    {
        return Err(AppError::validation(field));
    }
    Ok(value.to_ascii_lowercase())
}

impl AccountService {
    pub fn list_contact_aggregations(&self) -> Result<Vec<ContactAggregation>, AppError> {
        self.storage.list_contact_aggregations()
    }

    /// Idempotent: setting an existing rule or removing a missing one succeeds.
    pub fn set_contact_aggregation(
        &mut self,
        address: &str,
        aggregated: bool,
    ) -> Result<(), AppError> {
        let address = normalize_sender_address(address, "address")?;
        self.storage.set_contact_aggregation(&address, aggregated)
    }
}
