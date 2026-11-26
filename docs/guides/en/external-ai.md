---
title: Connect other AI tools
description: Let other AI tools on your computer (such as Claude Code, Cursor or Codex) read and handle your mail locally.
---

In addition to the built-in assistant, MailX can expose a local connection (MCP) so other AI tools on your computer — such as Claude Code, Codex, Cursor, Trae or OpenCode — can search your mail, organize messages, prepare drafts or send email.

This feature is **off by default**.

:::caution[Read before turning this on]
Once the master switch is on, connected local AI tools can:
- read mail and conversations across all your accounts, and mark read, star or archive;
- create and edit drafts, attach local files and export received attachments to your computer;
- **send mail directly, without asking you to confirm each message**.

They **cannot delete mail**. Keep in mind that external tools may pass the mail they read to their own AI provider, so only connect tools you trust.
:::

## How to turn it on and connect

1. Open **Settings → MCP**, turn on the switch and review the authorization prompt.
2. Under **Connect an agent**, pick your tool:
   - **Claude Code, Codex, OpenCode**: MailX adds itself to your personal configuration while keeping your existing settings.
   - **Cursor, Trae**: opens the tool's official import flow so you can confirm inside that app.
   - **Other MCP-compatible tools**: expand manual configuration and copy the command or snippet into your tool.
3. Reload the tool if needed, then click **Check MailX connection** to confirm setup.

## Good to know

- **Keep MailX running**: external tools stay connected even when you close the main window and leave MailX in the background. Quitting MailX completely or turning MCP off disconnects them right away.
- **Connection indicator**: while enabled, an MCP icon appears in the top-right title bar, with a green dot whenever a tool is connected.
- **Turn off anytime**: toggle MCP off under **Settings → MCP** to stop access immediately. Any message already queued in the Outbox must be canceled there.
- **Exported attachments**: files exported by an external tool stay on your computer until you remove them.
