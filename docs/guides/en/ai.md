---
title: Using the AI assistant
description: Connect an AI provider or local model to ask about your mail, summarize long messages and help you write.
---

AI in MailX is completely optional. If you don't add a provider, AI stays off and everything else works as usual. Before MailX sends any mail content to an AI provider for the first time, it asks for your consent, which you can revoke anytime in Settings.

## Step 1: Connect an AI provider

1. Open **Settings → Integrations → AI** and click **Add provider**.
2. Pick a provider from the list (or choose **Custom** for a compatible endpoint), paste your key and click **Connect**.
3. Choose your **Default model** and click **Test** to send one short test message (no mail content is sent).

:::tip[Want to keep your mail completely on your own computer?]
Connect a local model service such as **Ollama** or **LM Studio**. No key is required, and your mail never leaves your machine.
:::

## Ask about your mail

With the assistant turned on, you can chat with it to find, organize or draft replies:

- **Where to open it**: type directly in the box at the bottom of **Now**, or click the assistant button at the bottom right of any mail page (`Cmd/Ctrl + J`) to open the side panel.
- **Quick suggested questions**: hover over the bottom-right assistant button to see three suggested questions tailored to what is on screen (your list, open email, selection or draft). Click one to ask right away — suggestions are built locally and send nothing until you click.
- **What you can ask**: try "Which emails need a reply today?", "Turn these meeting emails into a to-do list", or "Draft a polite decline to Alex". Emails and drafts the assistant mentions appear as cards you can click to open.
- **Edit and retry**: hover over any chat message to copy it, edit your own message in place, regenerate the last reply, or continue a reply that was stopped.
- **History**: chats stay on your computer. Click the clock icon to revisit or continue past conversations, or turn off saving and clear history in Settings.

:::note[Safety boundaries]
- Under **Settings → Integrations → AI → Assistant**, choose which mailboxes the assistant is allowed to read and add custom instructions (such as how to address you).
- The assistant **cannot delete mail**. When it wants to send an email, it shows a confirmation card with the recipients, subject, text and attachments by default — nothing goes out until you click **Send**.
:::

## Summarize an email

Open an email and click **AI summary** (the sparkle icon) in the reading toolbar:

1. Pick a style (**Brief**, **Detailed** or **Key points**, which highlights any to-dos) and a language.
2. Click **Summarize**. The summary appears above the message, where you can copy it, regenerate it, collapse it to one line, or click **Ask more** to continue in the assistant panel.
3. Summaries are saved locally, so reopening the email shows the summary right away without sending it again.

If you want emails from specific senders — or all longer emails — to summarize automatically when opened, turn that on in the summary panel or under **Settings → Integrations → AI → Assistant** (off by default).

## Write and polish with AI

In the compose window, click **AI writing** (the sparkle icon) in the toolbar or press `Cmd/Ctrl + J` in the body:

- **Draft**: type a few notes and let AI turn them into a full email (replies match the language of the message you're answering by default).
- **Rewrite**: select text (or leave nothing selected to use everything you wrote) and choose **Polish**, **More formal**, **Friendlier**, **More concise**, **Expand**, **Translate into…** or your own request.
- **Proofread**: highlights removed and added words clearly so you can check every change.

Results appear in a preview card right below your text and never alter your draft on their own. Click **Accept** when you're happy (one Undo restores your earlier text), or choose **Insert below**, **Try again** or **Discard**. Quoted original messages and pictures are never sent or modified.

<details>
<summary><strong>Use a proxy just for AI</strong></summary>

AI connects directly by default and does not use your mailbox proxies. Open **Advanced** at the bottom of **Settings → Integrations → AI** to configure an AI-only proxy, then enable it per provider. If the proxy fails, requests stop with an error and never quietly connect directly.

</details>
