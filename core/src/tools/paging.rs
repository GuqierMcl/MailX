//! Bounded, filter-bound cursors shared by local metadata tools.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use super::ToolError;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    pub filter: String,
    pub at: String,
    pub id: String,
}

pub(super) fn decode(value: Option<&str>, filter: &str) -> Result<Option<Cursor>, ToolError> {
    let invalid = || ToolError::invalid("cursor", "invalid or different filters");
    value
        .map(|value| {
            if value.is_empty() || value.len() > 4096 {
                return Err(invalid());
            }
            let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
            let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if cursor.filter != filter
                || cursor.id.is_empty()
                || cursor.id.len() > 256
                || chrono::DateTime::parse_from_rfc3339(&cursor.at).is_err()
            {
                return Err(invalid());
            }
            Ok(cursor)
        })
        .transpose()
}

pub(super) fn encode(filter: String, at: String, id: String) -> Result<String, ToolError> {
    serde_json::to_vec(&Cursor { filter, at, id })
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|_| super::ToolError::new(super::ToolErrorCode::Internal))
}
