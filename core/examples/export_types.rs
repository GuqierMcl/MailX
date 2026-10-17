//! Print the public TypeScript contract. The caller decides the output path.
use mailx_core::*;
use ts_rs::TS;

fn main() {
    println!(
        "// Generated from Rust by `cargo run -p mailx-core --example export_types`. Do not edit.\n"
    );
    macro_rules! export { ($($ty:ty),* $(,)?) => { $(println!("export {}\n", <$ty>::decl());)* }; }
    export!(
        ErrorCode,
        AppError,
        StartupStatus,
        ApiSuccess<String>,
        ApiFailure,
        ApiResult<String>,
        TlsMode,
        ConnectionConfig,
        SecretUpdate,
        AuthenticationMode,
        PasswordAuthenticationInput,
        OAuth2Configuration,
        AuthenticationInput,
        ProxyKind,
        DnsPolicy,
        ProxyAuthenticationInput,
        RouteInput,
        AccountConfigInput,
        CreateAccountRequest,
        UpdateAccountRequest,
        CredentialTrust,
        AuthenticationSummary,
        PasswordAuthenticationSummary,
        ProxyAuthenticationSummary,
        RouteConfig,
        ConnectionStatus,
        Account,
        FolderRole,
        Folder,
        MailAddress,
        BodyState,
        EmailSummary,
        EmailListQuery,
        Coverage,
        EmailPage,
        AttachmentSummary,
        EmailBody,
        EmailDetail,
    );
}
