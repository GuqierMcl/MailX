---
title: Troubleshooting & FAQ
description: Fix connection issues, understand local search, back up your data and report problems.
---

## A mailbox won't connect or stopped syncing

If an account shows a red exclamation mark, open **Accounts** in the sidebar, pick the mailbox and click **Verify connection** at the bottom. MailX will show which step failed. Common causes:

1. **Expired app password or code**: changing your webmail password on Gmail or iCloud revokes old app passwords. Generate a new one and update it in MailX.
2. **Client access not enabled**: providers like QQ, 163 and Sina require turning on client access in webmail settings; work mailboxes (Feishu, Lark, Microsoft 365) may require administrator approval.
3. **Network or proxy issue**: if the account uses a proxy, make sure the proxy is reachable. MailX never falls back to a direct connection when a proxy fails.

## Why can't I find very old emails in search?

To keep MailX fast and light on disk space, each folder's first sync downloads the most recent **30 days** of mail, and search only checks **content already stored on this computer** (unopened older bodies and text inside attachments are not searched). For older archives, check your provider's webmail.

## A sent email says "uncertain outcome"

This means the network dropped at a critical moment and MailX cannot tell whether the server accepted the message. To avoid sending a duplicate, MailX will not retry automatically. Check with the recipient or your provider's webmail Sent folder first, then retry manually from the **Outbox** if needed.

## Where is my data stored? How do I back it up?

MailX updates your local data automatically after an upgrade, and older versions may not open data prepared by a newer version. To keep a rollback copy before upgrading:

1. **Quit MailX completely** first (don't just close the window into the background).
2. Copy the entire data folder (not just a single database file):

| System | Default data folder |
| --- | --- |
| macOS | `~/Library/Application Support/MailX` |
| Windows | `%APPDATA%\MailX` |
| Linux | `~/.local/share/MailX` (or `MailX` inside `XDG_DATA_HOME`) |

:::note
Passwords and authorization codes live in your operating system's password store, so copying this folder does not copy credentials. It is intended for rolling back on the same computer rather than migrating to another machine.
:::

## How to report a problem

Open **Settings → About** for direct links to [GitHub Issues](https://github.com/GuqierMcl/MailX/issues). When reporting a problem, please include:

- your MailX version and operating system;
- which mail or AI provider you are using;
- what you did and what happened.

If diagnostic logs are helpful, look in the `logs/` subfolder of your data directory (`diagnostics.jsonl`). Logs only record startup and sync status — no passwords, email addresses or message contents — and a short excerpt around the issue is plenty.

:::caution
Never post passwords, authorization codes, real mail content or personal email addresses in public issues. To report a security vulnerability, use the repository's private security channel.
:::
