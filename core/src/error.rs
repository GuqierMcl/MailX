use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Validation,
    NotFound,
    Conflict,
    IdempotencyConflict,
    Unsupported,
    StorageUnavailable,
    StorageIncompatible,
    AlreadyOpen,
    SecretStoreLocked,
    SecretStoreUnavailable,
    SecretMissing,
    Internal,
}

/// Public errors use fixed text and field names, never lower-level error strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub field: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        let message = match code {
            ErrorCode::Validation => "配置参数无效，请检查标记的字段。",
            ErrorCode::NotFound => "账号不存在。",
            ErrorCode::Conflict => "配置已被修改，请重新加载后重试。",
            ErrorCode::IdempotencyConflict => "请求标识已用于其他内容，请重新提交。",
            ErrorCode::Unsupported => "当前版本尚不支持此认证方式。",
            ErrorCode::StorageUnavailable => "无法读写本地数据，请检查可用空间与访问权限。",
            ErrorCode::StorageIncompatible => "数据库版本或迁移校验不兼容，原有数据已保留。",
            ErrorCode::AlreadyOpen => "此数据目录已由另一个 MailX 实例打开。",
            ErrorCode::SecretStoreLocked => "系统凭据库已锁定，请解锁后重试。",
            ErrorCode::SecretStoreUnavailable => "系统凭据库暂不可用，请稍后重试。",
            ErrorCode::SecretMissing => "所需凭证不存在，请重新输入。",
            ErrorCode::Internal => "操作未完成，请重试。",
        };
        Self {
            code,
            message: message.to_owned(),
            field: None,
        }
    }
    pub fn validation(field: &str) -> Self {
        Self {
            field: Some(field.to_owned()),
            ..Self::new(ErrorCode::Validation)
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}
impl From<rusqlite::Error> for AppError {
    fn from(_: rusqlite::Error) -> Self {
        Self::new(ErrorCode::StorageUnavailable)
    }
}
impl From<crate::secrets::SecretStoreError> for AppError {
    fn from(value: crate::secrets::SecretStoreError) -> Self {
        Self::new(match value {
            crate::secrets::SecretStoreError::Locked => ErrorCode::SecretStoreLocked,
            _ => ErrorCode::SecretStoreUnavailable,
        })
    }
}

/// Transport envelope; the host creates the request ID, never the client principal.
#[derive(Debug, Serialize, TS)]
#[serde(untagged)]
pub enum ApiResult<T> {
    Success(ApiSuccess<T>),
    Failure(ApiFailure),
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApiSuccess<T> {
    #[ts(type = "true")]
    ok: bool,
    data: T,
    request_id: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApiFailure {
    #[ts(type = "false")]
    ok: bool,
    error: AppError,
    request_id: String,
}

impl<T> ApiResult<T> {
    pub fn from_result(result: Result<T, AppError>, request_id: String) -> Self {
        match result {
            Ok(data) => Self::Success(ApiSuccess {
                ok: true,
                data,
                request_id,
            }),
            Err(error) => Self::Failure(ApiFailure {
                ok: false,
                error,
                request_id,
            }),
        }
    }
}
