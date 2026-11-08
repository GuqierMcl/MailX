use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Validation,
    NotFound,
    Conflict,
    IdempotencyConflict,
    /// The draft belongs to a live send queue item and cannot be changed.
    DraftLocked,
    /// The caller is not allowed to perform this action.
    Forbidden,
    /// Sending already started; it can no longer be canceled.
    AlreadySending,
    /// Deleting for good needs the user's explicit confirmation first.
    ConfirmationRequired,
    /// The deletion already reached the server; it can no longer be undone.
    AlreadyDeleting,
    /// The message body is not downloaded yet; fetch it, then retry.
    BodyNotCached,
    /// The draft's attachments would exceed the total size allowed.
    AttachmentTooLarge,
    /// The draft already carries as many attachments as allowed.
    TooManyAttachments,
    UnsupportedImage,
    Unsupported,
    #[serde(rename = "oauthNetwork")]
    OAuthNetwork,
    #[serde(rename = "oauthProxy")]
    OAuthProxy,
    #[serde(rename = "oauthTls")]
    OAuthTls,
    #[serde(rename = "oauthRateLimited")]
    OAuthRateLimited,
    #[serde(rename = "oauthServerUnavailable")]
    OAuthServerUnavailable,
    #[serde(rename = "oauthTimeout")]
    OAuthTimeout,
    #[serde(rename = "oauthAuthorizationDenied")]
    OAuthAuthorizationDenied,
    #[serde(rename = "oauthSessionExpired")]
    OAuthSessionExpired,
    #[serde(rename = "oauthInvalidResponse")]
    OAuthInvalidResponse,
    #[serde(rename = "oauthReauthorizationRequired")]
    OAuthReauthorizationRequired,
    StorageUnavailable,
    StorageIncompatible,
    AlreadyOpen,
    SecretStoreLocked,
    SecretStoreUnavailable,
    SecretMissing,
    CoreNotReady,
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
            ErrorCode::DraftLocked => "这封邮件已在发送队列中，不能修改。",
            ErrorCode::Forbidden => "当前调用方无权执行此操作。",
            ErrorCode::AlreadySending => "已开始发送，无法撤销。",
            ErrorCode::ConfirmationRequired => "永久删除前需要确认。",
            ErrorCode::AlreadyDeleting => "已开始删除，无法撤销。",
            ErrorCode::BodyNotCached => "邮件正文尚未下载，请先打开这封邮件。",
            ErrorCode::AttachmentTooLarge => "附件总大小超过上限。",
            ErrorCode::TooManyAttachments => "附件数量已达上限。",
            ErrorCode::UnsupportedImage => "不支持该图片，仅支持 PNG、JPEG、GIF 和 WebP 静态图片。",
            ErrorCode::Unsupported => "当前版本尚不支持此认证方式。",
            ErrorCode::OAuthProxy => "无法通过你设置的代理连接邮箱服务商，请检查代理设置。",
            ErrorCode::OAuthTls => "无法与邮箱服务商建立安全连接，请检查网络后重试。",
            ErrorCode::OAuthRateLimited => "尝试次数过多，请稍等几分钟再试。",
            ErrorCode::OAuthServerUnavailable => "邮箱服务商暂时无法登录，请稍后重试。",
            ErrorCode::OAuthNetwork => "暂时连不上邮箱服务商，请检查网络后重试。",
            ErrorCode::OAuthTimeout => "邮箱服务商响应太慢，请稍后重试。",
            ErrorCode::OAuthAuthorizationDenied => {
                "登录时没有允许 MailX 访问邮箱。如需继续，请重新登录并选择允许。"
            }
            ErrorCode::OAuthSessionExpired => "这次登录已超时，请重新登录。",
            ErrorCode::OAuthInvalidResponse => "登录没有完成，请重新登录。",
            ErrorCode::OAuthReauthorizationRequired => "邮箱登录已过期，请重新登录。",
            ErrorCode::StorageUnavailable => "无法读写本地数据，请检查可用空间与访问权限。",
            ErrorCode::StorageIncompatible => "数据库版本或迁移校验不兼容，原有数据已保留。",
            ErrorCode::AlreadyOpen => "此数据目录已由另一个 MailX 实例打开。",
            ErrorCode::SecretStoreLocked => "系统凭据库已锁定，请解锁后重试。",
            ErrorCode::SecretStoreUnavailable => "系统凭据库暂不可用，请稍后重试。",
            ErrorCode::SecretMissing => "所需凭证不存在，请重新输入。",
            ErrorCode::CoreNotReady => "本地核心尚未就绪，请稍候重试。",
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
