---
title: Writing and sending
description: Compose messages, manage drafts and attachments, set up undo send, and check the outbox.
---

Click **Compose** at the top of the sidebar — or click Reply, Reply all or Forward on a message — to start writing.

## Where you write

- New messages open in a separate compose window by default so you can look at other mail while writing.
- Prefer to write inside the main window? Choose **Main window** under **Settings → Sending → Open composer in**.
- Hold `Shift` while clicking **Compose** or a reply button to use the other option just once.

## Recipients, attachments and drafts

- **Pick the sending mailbox**: when you have several accounts, choose which one to send from at the top; small text beneath shows the address recipients will see.
- **Recipient suggestions**: start typing a name or address and MailX suggests people from mail already on your computer. Drag addresses freely between To, Cc and Bcc.
- **Attachments and pictures**: attach files, insert pictures or paste screenshots directly. Attachments and inline images together default to a **20 MB** limit (your provider may set a lower one).
- **Missing attachment reminder**: if your message mentions an attachment and none is added, MailX reminds you on the first click of Send so you can attach one or send anyway.
- **Local drafts**: MailX saves drafts on your computer as you type. Open **Drafts** in the sidebar to preview, continue editing or discard them. Drafts stay on this computer and do not sync to other devices.

## Confirming and undoing a send

MailX adds two safeguards against sending by mistake:

1. **Second-click confirmation**: double-check the recipients, sending mailbox and attachments, then confirm on the second click.
2. **Undo send**: under **Settings → Sending**, set a **5, 10, 20 or 30 second** delay (off by default) so you can cancel before the message leaves your computer.

:::caution
Undo only works while a message is still waiting in the local queue. Once your mail server accepts the message, it cannot be recalled.
:::

## Outbox and Sent

- **Outbox**: shows messages that are queued, failed or have an uncertain outcome.
- If a message shows an **uncertain outcome**, the connection dropped at a critical moment and your server may already have accepted it. MailX will not resend it on its own — check with the recipient or your provider's webmail Sent folder before retrying manually.
- **Sent**: right after sending, MailX shows a local copy. Once your provider's saved copy syncs back, MailX merges them automatically so you don't see duplicates.
