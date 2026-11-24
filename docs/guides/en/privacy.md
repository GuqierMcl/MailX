---
title: Privacy
description: What MailX keeps on your computer, what it connects to, and when it sends anything.
sidebar:
  order: 90
---

MailX has no mail server of its own. Your mail and passwords stay on your own computer; we cannot see them, and we collect no usage data.

## Kept on your computer

- **Mail**: message lists, downloaded messages, drafts and attachments are stored locally. MailX does not encrypt these files itself, so protect your computer with a password.
- **Passwords and keys**: kept in the system's own password store (macOS Keychain, Windows Credential Manager or the Secret Service on Linux) and never shown again.
- **AI chats, email summaries and translations**: stored locally.

## What MailX connects to

| Connection | When | What is sent |
| --- | --- | --- |
| Your mail provider | Signing in, receiving and sending mail | Your sign-in details and your mail |
| Microsoft sign-in | Adding an Outlook or Microsoft 365 account and staying signed in | Sign-in details; your password is typed only in the browser |
| GitHub | Checking for updates at launch and while running; automatic download can be turned off | No mail content |
| A translation service | After you agree to and use translation | The subject and text of the message |
| The AI provider you add | After you agree to and use AI | See below |
| models.dev | Opening AI settings | Downloads the public list of AI providers and models and their icons; no mail content or keys |
| Pictures in mail | Asks first by default | A request to the website that hosts the picture |
| Sender avatars | Off by default | A hash of the sender's address to Gravatar; the sender's domain to DuckDuckGo or Google |

## What AI sends

MailX asks before the first use. After that, to do what you ask, it sends what you type and, from the mail or drafts involved, the text, subject, sender, recipients, date and attachment names. The assistant searches and reads mail to answer you, so several messages may be involved; in Settings you choose which mailboxes it may read. The contents of attachments and pictures are not sent.

If you turn on summarizing emails as you open them, or always summarize certain senders, that mail is sent when you open it. Both are off by default.

With a model that runs on your computer, such as Ollama, mail goes only to that model. You can take back your agreement at any time in Settings.

Each mail account can have its own proxy. If the proxy cannot be reached, MailX stops; it never quietly connects without it.

## Letting other AI tools use your mail

MailX can let other AI tools on this computer read and handle your mail. This is off by default. Once on, connected tools can access all accounts, manage drafts, add and export attachments, and send mail directly without asking for each message; they may pass mail content to their own AI provider. They cannot delete mail. Turn it off in Settings to stop access.
