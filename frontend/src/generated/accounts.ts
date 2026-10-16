// Generated from Rust by `cargo run -p mailx-core --example export_types`. Do not edit.

export type ErrorCode = "validation" | "notFound" | "conflict" | "idempotencyConflict" | "unsupported" | "storageUnavailable" | "storageIncompatible" | "alreadyOpen" | "secretStoreLocked" | "secretStoreUnavailable" | "secretMissing" | "internal";

export type AppError = { code: ErrorCode, message: string, field: string | null, };

export type ApiSuccess<T> = { ok: true, data: T, requestId: string, };

export type ApiFailure = { ok: false, error: AppError, requestId: string, };

export type ApiResult<T> = ApiSuccess<T> | ApiFailure;

export type TlsMode = "implicit" | "startTls";

export type ConnectionConfig = { host: string, port: number, tls: TlsMode, username: string, };

export type SecretUpdate = { "action": "keep" } | { "action": "replace", value: string, };

export type AuthenticationMode = "shared" | "separate";

export type PasswordAuthenticationInput = { "mode": "shared", password: SecretUpdate, } | { "mode": "separate", imapPassword: SecretUpdate, smtpPassword: SecretUpdate, };

export type OAuth2Configuration = { providerId: string, issuer: string, clientRegistrationId: string, authorizationSubject: string | null, targetMailboxResource: string | null, };

export type AuthenticationInput = { "kind": "password", } & ({ "mode": "shared", password: SecretUpdate, } | { "mode": "separate", imapPassword: SecretUpdate, smtpPassword: SecretUpdate, }) | { "kind": "oauth2", configuration: OAuth2Configuration, };

export type ProxyKind = "socks5" | "httpConnect";

export type DnsPolicy = "proxy";

export type ProxyAuthenticationInput = { username: string, password: SecretUpdate, };

export type RouteInput = { "kind": "direct" } | { "kind": "inherit" } | { "kind": "custom", proxyKind: ProxyKind, host: string, port: number, dns: DnsPolicy, authentication: ProxyAuthenticationInput | null, };

export type AccountConfigInput = { displayName: string, emailAddress: string, providerId: string | null, imap: ConnectionConfig, smtp: ConnectionConfig, authentication: AuthenticationInput, route: RouteInput, };

export type CreateAccountRequest = { requestId: string, config: AccountConfigInput, };

export type UpdateAccountRequest = { accountId: string, expectedRevision: number, config: AccountConfigInput, };

export type CredentialTrust = "trusted" | "needsConfirmation";

export type AuthenticationSummary = { id: string, generation: number, passwordConfigured: boolean, trust: CredentialTrust, };

export type PasswordAuthenticationSummary = { mode: AuthenticationMode, imap: AuthenticationSummary, smtp: AuthenticationSummary, };

export type ProxyAuthenticationSummary = { username: string, passwordConfigured: boolean, trust: CredentialTrust, };

export type RouteConfig = { "kind": "direct" } | { "kind": "inherit" } | { "kind": "custom", proxyKind: ProxyKind, host: string, port: number, dns: DnsPolicy, authentication: ProxyAuthenticationSummary | null, };

export type ConnectionStatus = "unverified";

export type Account = { id: string, revision: number, displayName: string, emailAddress: string, providerId: string | null, mailboxSourceId: string, imap: ConnectionConfig, smtp: ConnectionConfig, authentication: PasswordAuthenticationSummary, route: RouteConfig, routeRevision: number,
/**
 * The initial application default is direct at revision 1.
 */
defaultRouteRevision: number | null, connectionStatus: ConnectionStatus, createdAt: string, updatedAt: string, };
