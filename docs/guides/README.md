# Getting Started with MailX

English | [简体中文](zh-CN/README.md)

This page covers the basics of the desktop app. Published installers are listed in [Releases](https://github.com/GuqierMcl/MailX/releases); tested combinations are in the [validation record](../development/validation.md), and upcoming features in the [roadmap](../roadmap/README.md).

## Install and add an account

1. Pick the installer for your platform in Releases and open it as described in that release's install notes. macOS has separate Apple silicon and Intel builds; Windows has an x64 installer; Linux has x64 AppImage, deb and rpm. The installers are not code-signed yet, and you can check your download against the `SHA256SUMS.txt` of the same release.
2. Prepare client access on your provider's website, enabling IMAP / SMTP where required, and get a client password, authorization code or app-specific password. MailX has built-in entries for QQ Mail / Foxmail, 163, 126, yeah.net, 139 and iCloud; any other account takes manual server and port settings. OAuth sign-in is not supported yet.
3. Add the account in MailX with its address and client credential; if you need a proxy, set SOCKS5 or HTTP CONNECT for that account. You can test incoming and outgoing sign-in before saving and see each result. A successful sign-in test only means authentication works — confirm actual delivery with a test recipient of your own.
4. After saving, wait for the sync. The top of the sidebar collects Inbox, Starred, Drafts and Sent from all accounts, with Archive, Junk and Trash under More; with several accounts, hover a row and use its arrow to list each account. Each account is collapsed below that; open it to see all of its folders, including Inbox and Sent. To give the mail more room, collapse the sidebar from the title bar or with Cmd/Ctrl+B: it becomes a narrow column of icons with the same places, tooltips that show unread counts, and menus for More and each account. Your choice is remembered. If the provider refuses the connection, check that client access is enabled, that the authorization code is valid, and your network routing.

### iCloud Mail

First [create an iCloud Mail address](https://support.apple.com/guide/icloud/create-a-primary-icloudcom-email-address-mmdd8d1c5c/icloud) if you do not already have one. Enable two-factor authentication for your Apple Account, then generate an [app-specific password](https://support.apple.com/en-us/102654) at `account.apple.com` under Sign-In and Security → App-Specific Passwords. Enter that password in MailX; use your iCloud Mail address rather than an unrelated email address used to sign in to your Apple Account.

Choose iCloud for an `@icloud.com`, `@me.com` or `@mac.com` address. MailX fills in `imap.mail.me.com:993` with implicit TLS and `smtp.mail.me.com:587` with STARTTLS, sharing the app-specific password. Both usernames default to your full email address. If incoming sign-in fails, Advanced settings lets you change only the IMAP username to the part before `@`, as [Apple's server guide](https://support.apple.com/en-us/102525) suggests; keep the full address for SMTP.

Changing or resetting your Apple Account password revokes all app-specific passwords. Generate a new one and update the account connection in MailX to restore access. Apple Account authorization sign-in is not available in MailX. Use manual IMAP / SMTP setup for an iCloud+ custom domain; aliases have no separate sender selector, and MailX does not automatically merge addresses that share one mailbox. Mail Drop is not available, so Apple's ordinary message-size limit still applies to attachments and inline images.

## Reading, search and organizing

The first sync of each folder starts from the last 30 days and fills up to the newest 1,000 message headers in that window per round, keeping previously cached history. This is not a full download of the mailbox. Accounts that support IMAP IDLE listen for inbox changes; other folders and accounts without IDLE keep polling.

Automatic sync runs quietly: account rows keep their unread counts without a loading animation. When you refresh manually, only the refresh button you use shows progress. A newly added account shows a subdued text hint while first fetching browsable content; accounts with cached content do not repeat that hint on normal startup. Sync problems still show a static warning so you can check the account.

Opening a message downloads and caches its body, so cached content can be read offline later. Search looks at message headers, previews, cached bodies and attachment names on this computer; bodies that were never downloaded are not fully searchable, and attachment contents are not searched. Results do not represent everything on the server.

Hover or focus a message to show quick actions for starring, archiving or restoring, and deleting. The small ring at the left toggles read status; the reading toolbar uses the same ring. An empty ring means read, and a ring with a blue dot means unread. Local sent copies offer a confirmed removal from this device instead.

Read and starred states write back to the server. Archive only removes the message from the inbox on this computer; find it under More → Archive or under Archived in the account, and it is unchanged for other clients. A server folder that is itself called Archive stays under its account with the server's name. Delete first offers a short undo window, then moves the message to the server's Trash; in Trash, or when the account has no such folder, a confirmation permanently deletes it. Moving messages to arbitrary server folders is not supported yet.

Remote images ask first by default, and attachments can be saved as local files. MailX does not encrypt the mail cache or attachments as a whole; on a shared computer rely on the operating system account and disk protection.

## Composing and sending

New mail, reply, reply all and forward use a separate compose window, and drafts save locally as you type. You can attach files, insert pictures or paste screenshots; attachments and inline images together default to a 20 MiB limit, and providers may impose a lower one. Drafts do not sync to other clients.

Check the recipients, sending account, body and attachments, then confirm sending with the second click as prompted. For an undo window, choose 5, 10, 20 or 30 seconds in Settings → Sending; the default waits not at all. Undo only applies to items still waiting in the queue and cannot recall mail the server has accepted.

The outbox shows queued, failed and unknown outcomes. An unknown outcome means the server may have accepted the message, and MailX will not resend it on its own; check the recipient or the provider's Sent folder first, then decide whether to retry manually. The server accepting an SMTP submission does not mean the recipient received the message.

The Sent folder shows a local copy until MailX matches it to the provider's saved copy, then shows only the server copy. QQ Mail may assign a new message identifier; MailX also checks the original identifier preserved by QQ to avoid showing both copies. Existing duplicates are reconciled as mail headers sync, without sending the message again. If no match can be confirmed, the local copy stays. Whether the provider saves sent mail automatically depends on its behaviour; MailX does not upload local copies to the server yet.

## Connecting external AI clients (MCP)

MailX includes a local MCP server. A built-in AI assistant or model settings are not available yet. MCP is off by default.

1. Open Settings → MCP, turn it on and read the authorization confirmation. The master switch gives connected local clients reading, mail organizing, draft and local file attachment management, and direct sending for all accounts, **with no per-message send confirmation**.
2. Click Copy client configuration and add it to a compatible MCP client. The configuration uses the bridge installed with the app; copy it again if you move the installation. Follow each client's own documentation for importing it.
3. Keep MailX running; the settings page shows the state and connected clients. It keeps working when the main window is closed and the app stays in the background. Fully quitting the app or turning MCP off makes the client connection unavailable. Turning MCP off does not cancel queued sends; handle those in the outbox.

A client may send the mail it reads to the AI service it uses, so choose clients and models with your privacy in mind. There is no per-client or per-account authorization yet, and MCP cannot delete mail. Compatibility of clients, platforms and tools is recorded in the [validation record](../development/validation.md).

## Background operation, launch at login and updates

Closing the main window can keep MailX in the background or quit; it asks each time by default, and you can change that in General settings. On macOS, the MailX menu bar icon is on by default and can be turned off; the Dock remains a recovery entry when the main window is hidden. Launch at login and start silently are both off by default; start silently only applies when the app starts automatically at login. When the system or desktop environment has no usable background entry, the main window appears instead of running with no way to get back.

The app checks GitHub for new versions, and you can turn automatic checks off or check manually on the About page. macOS, Windows and the AppImage support in-app updates; for deb / rpm, install the new package from the release page. System code signing and update package signing are two different things, and the installers may still trigger a first-run prompt from the system.

## Upgrading and keeping your data

The app runs any database migrations it needs at launch; see the release notes of each version for what changes. Older versions usually cannot open a migrated database, so reinstalling an older build is not a way to roll data back.

To keep a rollback copy before upgrading, **quit MailX completely** first, then copy the entire MailX data directory, including the database, any SQLite side files, settings and blob files — do not copy only `mailx.sqlite3`:

| Platform | Default data directory |
| --- | --- |
| macOS | `~/Library/Application Support/MailX` |
| Windows | `%APPDATA%\MailX` |
| Linux | `~/.local/share/MailX` (or the directory set by `XDG_DATA_HOME`) |

Passwords and authorization codes live in the system credential store, so copying the data directory does not copy credentials and is not a verified cross-machine migration or full restore. Automatic backup and one-click restore are not available yet; restoring an old backup can also involve mail that was already sent, so do not overwrite a working installation with an old copy.

If something goes wrong, describe the version, system, provider and steps in [Issues](https://github.com/GuqierMcl/MailX/issues), using made-up addresses and test mail; do not attach passwords, authorization codes, real mail or full private logs. Report security vulnerabilities through the private [security channel](../../SECURITY.md).
