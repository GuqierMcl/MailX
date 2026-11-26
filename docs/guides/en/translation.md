---
title: Translate emails
description: Translate foreign-language messages in place, keep their layout, and switch back anytime.
---

When you open an email in another language, click the **Translate** button in the reading toolbar to translate both the subject and body — paragraphs, lists, tables, links and images stay right where they were.

## Translate and switch back

1. Open a downloaded email and click **Translate** in the reading toolbar.
2. The original stays readable while the translation runs, and you can cancel anytime.
3. Click **Translate** again to switch back to the original immediately; both versions remember where you scrolled.
4. Click the small arrow beside the button to pick another target language, translate again or open translation settings.

## Language hints and automatic translation

When an email looks like it is in another language, MailX detects it on your computer and shows a bar above the message (for example, "This message is in French") with a one-click translate button.

- From the bar's menu, choose **Always translate** for that language (future emails in that language translate as soon as you open them) or **Don't suggest translating** for it.
- Manage these language lists or turn off the hint anytime under **Settings → Translation**.
- Language detection happens locally on your computer — nothing is sent until you translate (or enable automatic translation).

## Choose a translation service

Pick your preferred service and default target language under **Settings → Translation**:

- **Works out of the box (default)**: **Microsoft Translate** and **Google Translate** need no sign-up or key. They use the same channel as browser translation, which has no official uptime guarantee and can occasionally slow down or stop working.
- **Use your own key**: for steadier service and clear privacy terms, choose **DeepL**, **Microsoft Translator** or **Google Cloud Translation** and paste your own key. Once saved, the key is never shown again; use **Test** to verify it works.
- **Use an AI model**: if you have connected an AI provider, you can choose **AI model** for translations that pay closer attention to tone and context.

## Privacy and notes

- Before the first use of any translation service, MailX asks for your permission.
- Only the subject and message text are sent — **never attachments, images, link addresses or mailbox passwords**. Folded quoted replies at the end of a message are skipped by default.
- Translations are cached on your computer so reopening a message doesn't send it again. View or clear cached translations under **Settings → Translation → Saved translations**.
- Very long emails translate up to the first 50,000 characters and keep the rest in the original language.

<details>
<summary><strong>Use a proxy just for translation</strong></summary>

Translation connects directly by default and does not use your mailbox proxies. To route translation through a proxy, open **Advanced** at the bottom of **Settings → Translation** and enter your proxy details. If the proxy fails, translation stops with an error and never quietly connects directly.

</details>
