# Getting Started with MailX

English | [简体中文](zh-CN/README.md)

This page covers the basics of the desktop app. Published installers are listed in [Releases](https://github.com/GuqierMcl/MailX/releases); tested combinations are in the [validation record](../development/validation.md), and upcoming features in the [roadmap](../roadmap/README.md).

## Install and add an account

1. Pick the installer for your platform in Releases and open it as described in that release's install notes. macOS has separate Apple silicon and Intel builds; Windows has an x64 installer; Linux has x64 AppImage, deb and rpm. The installers are not code-signed yet, and you can check your download against the `SHA256SUMS.txt` of the same release.
2. Prepare client access on your provider's website, enabling IMAP / SMTP where required, and get a client password, authorization code or app-specific password. MailX has built-in entries for Gmail, iCloud, Outlook, Microsoft 365 / Exchange, QQ Mail / Foxmail, 163, 126, yeah.net and 139; any other account takes manual server and port settings. Outlook and Microsoft 365 sign in in your browser and need no password or authorization code (see below); sign-in in the browser is not available for other providers yet.
3. Add the account in MailX with its address and client credential; if you need a proxy, set SOCKS5 or HTTP CONNECT for that account. You can test incoming and outgoing sign-in before saving and see each result. A successful sign-in test only means authentication works — confirm actual delivery with a test recipient of your own.
4. After saving, wait for the sync. The top of the sidebar collects Inbox, Starred, Drafts and Sent from all accounts, with Archive, Junk and Trash under More; with several accounts, hover a row and use its arrow to list each account. Each account is collapsed below that; open it to see all of its folders, including Inbox and Sent. To give the mail more room, collapse the sidebar from the title bar or with Cmd/Ctrl+B: it becomes a narrow column of icons with the same places, tooltips that show unread counts, and menus for More and each account. Your choice is remembered. If the provider refuses the connection, check that client access is enabled, that the authorization code is valid, and your network routing.

### Gmail

Turn on [2-Step Verification](https://support.google.com/accounts/answer/185839) for your Google Account, then create an [app password](https://support.google.com/accounts/answer/185833) on the App passwords page of your Google Account. Google shows a 16-character password in groups; remove the spaces when you paste it into MailX. IMAP is always on in Gmail, so there is nothing to enable in Gmail settings.

Choose Gmail for a `@gmail.com` or `@googlemail.com` address. MailX fills in `imap.gmail.com:993` and `smtp.gmail.com:465`, both with implicit TLS, sharing the app password. Both usernames are your full email address.

Changing your Google Account password turns off all app passwords; create a new one and update the account connection in MailX. App passwords aren't available if you use only security keys for 2-Step Verification, have Advanced Protection, or use a work or school account. MailX doesn't offer Google sign-in in the browser yet, so these accounts can't be added for now. Gmail uses labels, so the same message can appear in several folders; how this shows up in MailX hasn't been fully checked with real accounts.

### iCloud Mail

First [create an iCloud Mail address](https://support.apple.com/guide/icloud/create-a-primary-icloudcom-email-address-mmdd8d1c5c/icloud) if you do not already have one. Enable two-factor authentication for your Apple Account, then generate an [app-specific password](https://support.apple.com/en-us/102654) at `account.apple.com` under Sign-In and Security → App-Specific Passwords. Enter that password in MailX; use your iCloud Mail address rather than an unrelated email address used to sign in to your Apple Account.

Choose iCloud for an `@icloud.com`, `@me.com` or `@mac.com` address. MailX fills in `imap.mail.me.com:993` with implicit TLS and `smtp.mail.me.com:587` with STARTTLS, sharing the app-specific password. Both usernames default to your full email address. If incoming sign-in fails, Advanced settings lets you change only the IMAP username to the part before `@`, as [Apple's server guide](https://support.apple.com/en-us/102525) suggests; keep the full address for SMTP.

Changing or resetting your Apple Account password revokes all app-specific passwords. Generate a new one and update the account connection in MailX to restore access. Apple Account authorization sign-in is not available in MailX. Use manual IMAP / SMTP setup for an iCloud+ custom domain; aliases have no separate sender selector, and MailX does not automatically merge addresses that share one mailbox. Mail Drop is not available, so Apple's ordinary message-size limit still applies to attachments and inline images.

### Microsoft (Outlook / Microsoft 365)

Two entries sign in through your browser, so MailX never sees your password. Choose **Outlook** for a personal Outlook.com, Hotmail or Live address. Choose **Microsoft 365 / Exchange** for a work or school address. You don't type your address first: change the network settings if you need to, then click **Sign in in browser**. MailX opens the Microsoft sign-in page; finish signing in there and come back. You can cancel while MailX is waiting.

MailX learns your address from the sign-in and checks that the mail server really opens that mailbox. If it can't (for example, you signed in with an alias or a different sign-in name), it asks you for the account's email address and checks that one with the sign-in you already have, without opening the browser again. You get five tries. Then give the mailbox a name and click **Done**. Going back or closing the window drops the sign-in, and it expires on its own after 15 minutes.

If you pick the wrong entry, MailX tells you which one to use. If it says it couldn't open the mailbox, check the address first. For Outlook.com, make sure IMAP is turned on in Outlook settings. For work or school mail, your administrator may need to approve MailX or allow IMAP and SMTP for your mailbox. MailX can't work around an organization that turned either one off, and it doesn't keep asking you to sign in again in that case.

If your sign-in expires or is withdrawn, open the account in MailX and click **Sign in again**; your mail and settings stay as they are. Mail on a company-hosted Exchange server isn't covered by the Microsoft 365 entry. Use manual IMAP / SMTP setup with the server details from your administrator.

Outlook has been tested with a personal Outlook.com account; the Microsoft 365 entry hasn't been tested with a real work or school account yet. Shared mailboxes and China-operated Microsoft 365 aren't supported.

## Reading, search and organizing

The first sync of each folder starts from the last 30 days and fills up to the newest 1,000 message headers in that window per round, keeping previously cached history. This is not a full download of the mailbox. Accounts that support IMAP IDLE listen for inbox changes; other folders and accounts without IDLE keep polling.

Automatic sync runs quietly: account rows keep their unread counts without a loading animation. Use the refresh button immediately to the right of the sidebar toggle in the title bar to refresh all accounts, or the refresh button on an account row to refresh just that account. When you refresh manually, only the refresh button you use shows progress. A newly added account shows a subdued text hint while first fetching browsable content; accounts with cached content do not repeat that hint on normal startup. Sync problems still show a static warning so you can check the account.

Opening a message downloads and caches its body, so cached content can be read offline later. In the Unread view, a message you open stays in the list after it is marked read, until you switch mailbox, filter or search. Search looks at message headers, previews, cached bodies and attachment names on this computer; bodies that were never downloaded are not fully searchable, and attachment contents are not searched. Results do not represent everything on the server.

When a message is part of a back-and-forth, the reader shows the whole conversation: the open message is shown in full, and the other messages in the conversation, including your own replies from Sent, appear as one-line rows above and below it. Click a row to read that message; you stay in the same conversation. Long conversations fold the rest into an "earlier" or "newer" row. Use Single and Conversation at the top of the reader to switch back to one message at a time. MailX links messages by their reply information, so mail from some senders or older mail may still appear on its own; if this computer has not downloaded all of a conversation, the reader says so.

Hover or focus a message with the keyboard to show quick actions in the upper right, in place of the time: star, mark read or unread, archive or move back to inbox, and, set apart at the far end, delete. They work the same whatever the body preview setting. A click made right after the pointer enters a row opens the message instead of hitting a button that is still fading in. After a mouse click, moving the pointer away hides the actions again. A starred message keeps a small yellow star beside its subject. The narrow strip at the left of each row shows a blue dot for unread messages and stays empty for read ones. The reading toolbar has the same read-status ring: an empty ring means read, and a ring with a blue dot means unread. Local sent copies offer a confirmed removal from this device instead. In views that span several accounts, each row names its mailbox after the sender; under Settings → Appearance → Message list, Show mailbox as lets you use a small account icon on the sender's avatar instead.

Right-click a message in the list to open a menu with the same actions in one place: Open, Reply, Reply all, Forward, Mark as read or unread, Add or remove star, Archive or Move back to inbox where archiving is available, and Delete. A local sent copy offers only Open, Reply, Reply all, Forward and Remove from this device. Right-clicking does not open the message; the row stays highlighted while the menu is open. While you are selecting several messages, right-click does nothing; use the buttons at the top instead. With the keyboard, Shift+F10 or the Menu key opens the menu for the focused message, and Esc closes it.

The same menu can show all mail from the message's sender: choose Show mail from this contact, and the message list switches to that contact's mail across your folders, leaving out Junk and Trash. Use the back button at the top of the list, Esc or the mouse's back button to return to where you were. Choose Group mail from this contact to fold all of that contact's mail into a single row in your inboxes; the row shows how many messages there are and how many are unread, and clicking it opens the contact's mail. Searching always lists messages one by one. Grouped rows cannot be picked when selecting several messages. To stop grouping, right-click the row or a message from the contact, or go to Settings → Contacts, which lists every grouped contact. Grouping matches the sender's email address, applies to all accounts, and is kept on this computer only.

The open message, draft or outbox item keeps a blue selection background after the pointer leaves, with separate tones for light and dark themes. Custom accent colors also tint the selection background; nearly gray, very light or very dark colors use the default blue highlight instead so it stays distinct, while your main accent color stays as set. Hovering another row keeps the open item highlighted.

Read and starred states write back to the server. Archive only removes the message from the inbox on this computer; find it under More → Archive or under Archived in the account, and it is unchanged for other clients. A server folder that is itself called Archive stays under its account with the server's name. Delete first offers a short undo window, then moves the message to the server's Trash; in Trash, or when the account has no such folder, a confirmation permanently deletes it. Moving messages to arbitrary server folders is not supported yet.

To act on several messages at once, click Select above the message list, or Cmd/Ctrl-click or Shift-click a message. Selected messages get a blue outline with a check in the corner. Click to add or remove a message, Shift-click to select a run of messages, or use Select all to pick every message in the current list, up to 500 at a time. The buttons at the top then mark the selection read or unread, star it, archive it or delete it. Deleting and archiving can be undone; messages that could not be handled stay selected so you can see which ones. Click Done or press Esc to finish.

When a reply ends with the earlier message it quotes, MailX folds that quoted text behind a Show quoted text button so the new part stands out; click it to expand. MailX recognizes the quoting styles of common mail apps; quotes it cannot recognize, replies written between quoted lines, and forwarded messages are shown in full as before. Folding only changes how the message is displayed: replying or forwarding still includes the full original.

Remote images ask first by default, and attachments can be saved as local files. MailX does not encrypt the mail cache or attachments as a whole; on a shared computer rely on the operating system account and disk protection.

## Translate messages

MailX can translate the message you are reading, keeping its layout: paragraphs, lists, tables, bold text, links and inline images stay where they were. The subject is translated too. Translation works out of the box with Microsoft Translate, which needs no sign-up and no key. It uses an online translation service, not an AI assistant, and it is not built into MailX; if you have connected an AI model (see AI below), you can also choose that. If you want something steadier, you can switch to a service with a key of your own. This feature is new and has only been checked with automated tests, a few test sentences and a preview with sample data, not yet by hand on every system.

### Choose a service

Microsoft Translate and Google Translate need no setup. They use the same channel as the translator built into the Edge and Chrome browsers. That channel is not something Microsoft or Google officially offers to other apps, so it can stop working or slow down at any time, and neither company makes any promise about it. If it fails, the line above the message offers Retry and "Use another service". Google Translate usually needs a proxy in mainland China. For something steadier, choose a service with your own key.

To use an AI model instead, first connect a provider under Settings → Integrations → AI, then choose "AI model" as the Translation service. It uses your default model unless you pick another under "Model" ("Same as default" follows the default model). If no model is ready, the button offers "Open AI settings". Models can be slower than the other services and can make mistakes, and sample tests with real models have not been done yet.

To use your own key:

1. Open Settings → Translation and, under Translation service, choose DeepL, Microsoft Translator or Google Cloud Translation from the "Use your own key" group. You need a key from the service you choose; get it from that service's own website. For Microsoft Translator there is also an optional Region, which is the region you picked when you created the resource; leave it empty for a global resource.
2. Paste the key and click Save. MailX keeps it securely on this device and never shows it again; the row then says Saved, and you can Replace or Remove it. Use Test to translate a short sample sentence and check that the key and connection work (it also works for the no-setup services, which have no key). The test does not send any of your mail.
3. Under Translating, choose what to translate into: "Same as the app language" (the default) or one of 26 languages.

### Translate and switch back

Open a message and click the translate button in the reading toolbar, just after the read and star buttons. The small arrow beside it opens a menu: Translate to (pick another language), Translate again, and Translation settings…. You can also choose Translate this message in the command palette. The original stays readable while the translation runs, and you can cancel. Click the button again to go back to the original, and again for the translation. Both versions stay loaded, so switching is instant and each remembers where you scrolled. If a translation fails, the reason and a Retry button appear in the line above the message.

If you turned translation off ("Don't translate"), or chose a service with your own key and have not saved one yet, the button tells you ("No translation service is set up yet") and offers Open settings. Only messages that have been downloaded and have a formatted (HTML) body can be translated.

The first time you use a service, MailX asks "Send this message to …?" before anything is sent, and you can Cancel. For Microsoft Translate and Google Translate the question also says that this is the channel browsers use, not an official service for other apps, so it may stop working, and that your text goes to Microsoft or Google. Switching to another service asks again. For an AI model it asks before the first message is sent to that provider, and again if you switch to a different provider. While you stay in MailX, it remembers for each message whether you were viewing the translation and whether you closed the hint; this is not kept after you quit.

### The language hint

Once a service is ready (the no-setup services always are), a message that looks like it is in a language other than yours gets a line above it, for example "This message is in French" with a Translate to … button. Its More menu lets you choose "Always translate …" or "Don't suggest translating …" for that language, and the close button hides the hint for this message only. The same two choices are in the toolbar menu. The language is detected on your computer; nothing is sent until you translate. Short messages and mixed-language messages may not be recognized, and Simplified and Traditional Chinese are not told apart. Under Settings → Translation → Foreign-language mail you can turn the hint off ("Offer to translate").

### Translate automatically by language

Choose "Always translate …" for a language, and messages in that language are translated automatically as soon as you open them, which sends their text to the translation service you agreed to. Settings → Translation → Foreign-language mail lists the languages you set to "Always translate" and "Don't offer for", and you can add or remove languages there. A language can only be in one of the two lists.

### What is sent and what is not

MailX sends the message's subject and text to the service you chose (Microsoft, Google, the one whose key you saved, or your AI model's provider), and only after you agree. It does not send attachments, pictures, link addresses or your mailbox passwords. Quoted earlier messages at the end of a reply are not translated unless you turn on "Translate quoted earlier messages" (off by default). Code and plain web or email addresses are left as they are. What each service does with your text is governed by that service's own terms, so check them before you use it for private mail. The no-setup services have no terms and no data-processing statement for this use, so for private or sensitive mail prefer a service with your own key and terms you have read.

### Use a proxy for translation

Translation connects directly by default. Most people never need to change this, so it sits in the "Advanced" group at the bottom of Settings → Translation, which is folded until you open it; while it is folded, it shows the current connection ("Direct connection" or "Through proxy host:port") next to its title. Open it and choose "Through a proxy" under "Connection" (SOCKS5 or HTTP) to use a proxy only for translation. If your proxy asks you to sign in, fill in "Username" and "Password"; they are optional, and most proxies on this computer do not need them. The password is kept on this device, is never shown again, and is only used together with a username. The proxy you set on a mail account is not used for translation. If the proxy cannot connect or does not accept the sign-in, translation fails with an error and never quietly connects directly.

### Saved translations

Finished translations are kept on this computer so reopening a message does not send it again; they are removed when you delete the message or its account. Settings → Translation → Saved translations shows how many messages are saved and how much space they use, and Clear removes them all. There is no button to remove a single message's saved translation; "Translate again" in the toolbar menu replaces it with a new translation of that message. Saved translations are not encrypted, so treat them like your mail cache.

### Limits

- Very long messages are only partly translated: MailX sends at most 50,000 characters of a message and leaves the rest in the original language, and the line above the message says "This message is long, so only the first part was translated". A message whose body is larger than 2 MiB is not translated.
- A message must be downloaded before it can be translated. Plain-text messages can be translated too; a message with an empty body has nothing to translate.
- Sent copies that exist only on this computer cannot be translated, and neither can drafts while you write them.
- If a service drops a link or picture from a paragraph, MailX keeps that paragraph in the original language.
- Translation is not available to MCP clients, and a side-by-side view is not available.

## AI

Settings → Integrations → AI connects MailX to AI model providers. Translation can use a model (choose "AI model" under Translation); nothing else in MailX does yet, and nothing you read or write is sent to a model unless you choose that.

### Add a provider

1. Open Settings → Integrations → AI and choose "Add provider" under "Providers".
2. Pick a provider from the list, or choose Custom and enter the address and API type yourself (OpenAI-compatible or Anthropic-compatible).
3. Enter your API key and choose "Connect". MailX asks the provider for its list of models; if that fails, you see why, and "Save anyway" keeps the provider without checking it. Then pick a model.

The key is kept on this device and is never shown again. MailX shows which host the key is sent to. Local services such as Ollama and LM Studio run on this computer and need no key.

### Default model and Test

Under "Default model", choose one of the models of a provider you added. "Test" sends one short fixed sentence (no mail content) and shows how long the reply took, or why it failed. Translation follows this default unless you choose another under Settings → Translation → "Model".

### Use a proxy for AI

AI connects directly by default. Under "Advanced" you can set a proxy (SOCKS5 or HTTP, with an optional username and password) used only for AI; then choose per provider whether it goes through it. The proxy you set on a mail account is not used. If the proxy fails, the request fails and never connects directly.

## Composing and sending

New mail, reply, reply all and forward use a separate compose window, and drafts save locally as you type. You can attach files, insert pictures or paste screenshots; attachments and inline images together default to a 20 MiB limit, and providers may impose a lower one. Drafts do not sync to other clients.

Open Drafts in the sidebar and select a draft to preview its saved formatting and inline images in the reading pane. The preview follows your message theme setting and updates after the draft saves; file attachments are listed separately. Use Continue editing to reopen the compose window, or Delete to discard the draft.

Check the recipients, sending account, body and attachments, then confirm sending with the second click as prompted. For an undo window, choose 5, 10, 20 or 30 seconds in Settings → Sending; the default waits not at all. Undo only applies to items still waiting in the queue and cannot recall mail the server has accepted.

The outbox shows queued, failed and unknown outcomes. An unknown outcome means the server may have accepted the message, and MailX will not resend it on its own; check the recipient or the provider's Sent folder first, then decide whether to retry manually. The server accepting an SMTP submission does not mean the recipient received the message.

The Sent folder shows a local copy until MailX matches it to the provider's saved copy, then shows only the server copy. QQ Mail may assign a new message identifier; MailX also checks the original identifier preserved by QQ to avoid showing both copies. Existing duplicates are reconciled as mail headers sync, without sending the message again. If no match can be confirmed, the local copy stays. Whether the provider saves sent mail automatically depends on its behaviour; MailX does not upload local copies to the server yet.

## Connecting external AI clients (MCP)

MailX includes a local MCP server. A built-in AI assistant is not available yet. MCP is off by default.

1. Open Settings → MCP, turn it on and read the authorization confirmation. The master switch gives connected local clients reading, mail organizing, draft and local file attachment management, and direct sending for all accounts, **with no per-message send confirmation**.
2. Click Copy client configuration and add it to a compatible MCP client. The configuration uses the bridge installed with the app; copy it again if you move the installation. Follow each client's own documentation for importing it.
3. Keep MailX running; the settings page shows the state and connected clients. It keeps working when the main window is closed and the app stays in the background. Fully quitting the app or turning MCP off makes the client connection unavailable. Turning MCP off does not cancel queued sends; handle those in the outbox.

A client may send the mail it reads to the AI service it uses, so choose clients and models with your privacy in mind. There is no per-client or per-account authorization yet, and MCP cannot delete mail. Compatibility of clients, platforms and tools is recorded in the [validation record](../development/validation.md).

## Background operation, launch at login and updates

Closing the main window can keep MailX in the background or quit; it asks each time by default, and you can change that in General settings. On macOS, the MailX menu bar icon is on by default and can be turned off; the Dock remains a recovery entry when the main window is hidden. Launch at login and start silently are both off by default; start silently only applies when the app starts automatically at login. When the system or desktop environment has no usable background entry, the main window appears instead of running with no way to get back.

MailX always checks GitHub for updates shortly after launch and every six hours while running. Automatic updates are on by default in Settings → About: new versions download in the background, then a notice lets you restart to install. You can keep working and let the update install when you fully quit instead. MailX never restarts automatically, and hiding the window in the background does not install an update.

Turning Automatic updates off keeps version checks and manual downloads available. It stops an automatic download and cancels installation on quit for an automatically downloaded update; an update you chose to download manually still installs on quit. Cancel download pauses that version's automatic download until the next app session, a manual download, or turning Automatic updates off and on again. Temporary download failures are retried a limited number of times; the update entry keeps a Retry action when help is needed. If an installation did not complete, the next launch shows the failure and lets you retry or download the installer from GitHub.

macOS, Windows and the AppImage support in-app updates. For deb / rpm, Automatic updates is unavailable: use the release page or the package manager you installed MailX with. System code signing and update package signing are two different things, and the installers may still trigger a first-run prompt from the system.

## About and feedback

Settings → About shows the MailX logo and your current version above the update controls. The links below open the [GitHub repository](https://github.com/GuqierMcl/MailX), [problem-report template](https://github.com/GuqierMcl/MailX/issues/new?template=bug_report.md), [feature-request template](https://github.com/GuqierMcl/MailX/issues/new?template=feature_request.md) and [release history](https://github.com/GuqierMcl/MailX/releases) in your system browser. If a link cannot be opened, use the error prompt's Copy link action to open it yourself.

To report a problem or suggest a feature, sign in to GitHub if needed, fill in the template and submit it there. MailX does not automatically fill in your version or system details; include the information requested by the template.

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
