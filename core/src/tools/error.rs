//! Tool failures with stable codes and messages that are safe to show a model.
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{AppError, ErrorCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolErrorCode {
    /// No such tool, or not visible to this audience.
    UnknownTool,
    InvalidInput,
    /// The grant lacks the action.
    NotPermitted,
    /// Also reported for resources outside the grant's account scope.
    NotFound,
    BodyNotCached,
    /// An on-demand body download did not finish within the time limit.
    DownloadTimeout,
    /// An on-demand body download failed (network, authentication, proxy,
    /// TLS or protocol problem).
    DownloadFailed,
    /// The message is larger than the body download limit.
    BodyTooLarge,
    /// Storage or the core is not ready; retrying later may work.
    Unavailable,
    Internal,
}

impl ToolErrorCode {
    fn message(self) -> &'static str {
        match self {
            Self::UnknownTool => "Unknown tool.",
            Self::InvalidInput => "The input is invalid.",
            Self::NotPermitted => "This action is not permitted.",
            Self::NotFound => "The requested item was not found.",
            Self::BodyNotCached => {
                "The message body is not stored on this device. Ask the user to open the message in MailX first."
            }
            Self::DownloadTimeout => "Downloading the message body took too long. Try again later.",
            Self::DownloadFailed => {
                "The message body could not be downloaded from the mail server. Try again later."
            }
            Self::BodyTooLarge => "The message is too large to download.",
            Self::Unavailable => "MailX is temporarily unavailable. Try again later.",
            Self::Internal => "An internal error occurred.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    /// The offending input field for `invalidInput`.
    pub field: Option<String>,
}

impl ToolError {
    pub fn new(code: ToolErrorCode) -> Self {
        Self {
            code,
            message: code.message().to_owned(),
            field: None,
        }
    }

    /// Invalid input; `detail` must be a fixed string, never mail or server text.
    pub fn invalid(field: &str, detail: &str) -> Self {
        Self {
            code: ToolErrorCode::InvalidInput,
            message: format!("Invalid `{field}`: {detail}."),
            field: Some(field.to_owned()),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

impl From<AppError> for ToolError {
    /// Only the code (and the input field name) survive; app messages may
    /// carry text that is not meant for a model.
    fn from(error: AppError) -> Self {
        match error.code {
            ErrorCode::Validation | ErrorCode::IdempotencyConflict | ErrorCode::Conflict => {
                match error.field.as_deref() {
                    Some(field) if field.len() <= 64 && field.chars().all(is_field_char) => {
                        Self::invalid(field, "value rejected")
                    }
                    _ => Self::new(ToolErrorCode::InvalidInput),
                }
            }
            ErrorCode::NotFound => Self::new(ToolErrorCode::NotFound),
            ErrorCode::BodyNotCached => Self::new(ToolErrorCode::BodyNotCached),
            ErrorCode::Forbidden => Self::new(ToolErrorCode::NotPermitted),
            ErrorCode::StorageUnavailable
            | ErrorCode::StorageIncompatible
            | ErrorCode::CoreNotReady
            | ErrorCode::AlreadyOpen
            | ErrorCode::SecretStoreLocked
            | ErrorCode::SecretStoreUnavailable => Self::new(ToolErrorCode::Unavailable),
            _ => Self::new(ToolErrorCode::Internal),
        }
    }
}

fn is_field_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '_'
}
