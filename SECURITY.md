# Security Policy

English | [简体中文](SECURITY.zh-CN.md)

Released versions are listed in [Releases](https://github.com/GuqierMcl/MailX/releases), and changes are in the [CHANGELOG](CHANGELOG.md).

## Supported versions

Security issues are fixed on `main` and shipped in a new release. Older versions are not patched separately. Include the app version in your report.

## Reporting a vulnerability

Please report privately through the **Report a vulnerability** form on the repository's **Security** tab (GitHub private vulnerability reporting / Security Advisories).

Private reporting is enabled: [Report a vulnerability](https://github.com/GuqierMcl/MailX/security/advisories/new).

Please do not:

- open a public issue, pull request or discussion for a vulnerability;
- include real mailbox credentials, tokens, private keys, or real mail content, attachments and full logs containing real addresses; reproduce with synthetic data;
- test against anyone else's mailbox account or provider.

Include the affected commit or branch, reproduction steps, impact and your assessment where you can. This project is maintained by one person and does not promise a fixed response or fix time, but reports are taken seriously and answered as promptly as possible.

## Scope of interest

These areas sit directly on MailX's security boundary and are especially welcome:

- Mail HTML and body isolation: message content is untrusted and must not be able to run scripts, load remote resources without permission, or gain Tauri IPC access;
- Tauri IPC surface: whether commands and permissions exceed what the UI actually needs;
- Credential storage: credentials should be managed only by the backend SecretStore and must not appear in return values, logs, the database or diagnostics;
- Network routing: protocol connections must use the account's routing, and a failing custom proxy must never fall back to a direct connection;
- Model output and MCP input are equally untrusted and must not extend the permissions the host granted. The current MCP master switch confirmation grants access to mail reading, organizing, draft and attachment management and direct sending for all accounts, with no per-message confirmation; tools must still check the action, account scope, draft version and idempotency key. Turning the switch off must revoke access. MCP does not provide mail deletion.

Design details are in [Network and security](docs/architecture/network-security.md).
