use crate::AppError;
use serde::Serialize;
use ts_rs::TS;

/// Host-owned startup state shared with clients as a read-only DTO.
///
/// The desktop host controls the transitions and service lifetime; keeping
/// this wire type in core makes the generated TypeScript contract come from
/// the same source as `AppError` and the account DTOs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StartupStatus {
    #[serde(rename = "initializing")]
    Initializing { revision: u32 },
    #[serde(rename = "ready")]
    Ready { revision: u32 },
    #[serde(rename = "failed")]
    Failed {
        revision: u32,
        error: AppError,
        #[serde(rename = "canRetry")]
        #[ts(rename = "canRetry")]
        can_retry: bool,
    },
}
