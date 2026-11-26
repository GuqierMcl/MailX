---
title: Add an account
description: Connect your personal or work mailboxes to MailX and manage existing accounts.
---

You do not need a MailX account. Just add the email accounts you already use.

## Add your first mailbox

1. Open MailX and click **Add account** (you can add more anytime from **Accounts** in the sidebar).
2. Choose your email provider.
3. Follow the sign-in prompt, or enter your email address and client password.
4. Click **Test connection**, then save and wait for your inbox to sync.

:::tip[Why can't I use my regular webmail password?]
To protect your account, providers like Gmail, iCloud, QQ and 163 require a separate **app password** or **authorization code** for desktop mail apps. It is only used for sending and receiving mail, separate from your main password, and can be replaced anytime.
:::

## Provider setup guides

Open your provider below to see how to get your sign-in details:

<details>
<summary><strong>Outlook / Microsoft 365 (browser sign-in)</strong></summary>

Both sign in through your browser, so MailX never sees your password:

- Choose **Outlook** for a personal `@outlook.com`, `@hotmail.com` or `@live.com` address.
- Choose **Microsoft 365** for a work or school address.

Click **Sign in in browser**, finish signing in on Microsoft's page, then return to MailX, give the mailbox a name and click **Done**. If MailX cannot open the mailbox, check that personal Outlook has client access turned on, or ask your organization's administrator whether third-party mail apps are allowed.

</details>

<details>
<summary><strong>Gmail</strong></summary>

1. Turn on [2-Step Verification](https://support.google.com/accounts/answer/185839) for your Google Account.
2. Create a 16-character password on Google's [App passwords](https://support.google.com/accounts/answer/185833) page.
3. Choose **Gmail** in MailX, enter your full email address and paste that password (without spaces).

Changing your Google Account password turns off existing app passwords; generate a new one and update it in MailX. Note: accounts that use only security keys, Advanced Protection or certain work/school policies may not offer app passwords and cannot be added yet.

</details>

<details>
<summary><strong>iCloud Mail</strong></summary>

1. Make sure you have an `@icloud.com`, `@me.com` or `@mac.com` address and two-factor authentication turned on for your Apple Account.
2. Visit `account.apple.com` and create an app-specific password under **Sign-In and Security → App-Specific Passwords**.
3. Choose **iCloud** in MailX and enter your iCloud Mail address and that password.

Changing or resetting your Apple Account password revokes app-specific passwords; create a new one and update it in MailX.

</details>

<details>
<summary><strong>QQ Mail, 163 / 126 / Yeah / 139 Mail</strong></summary>

1. Sign in to your provider's webmail in a browser and open its settings.
2. Turn on email client access and generate an **authorization code**.
3. Choose the provider in MailX, enter your full email address and paste the authorization code as the password.

</details>

<details>
<summary><strong>Sina Mail</strong></summary>

Choose **Sina Free Mail** for a free `@sina.com` or `@sina.cn` address. In Sina webmail, [turn on client access and generate an authorization code](https://help.sina.com.cn/comquestiondetail/view/1566/) (both steps are needed), then enter your address and code in MailX. For Sina VIP mailboxes, use **Manual setup**.

</details>

<details>
<summary><strong>Feishu Mail / Lark Mail</strong></summary>

If Feishu or Lark hosts your company mailbox, choose **Feishu Mail (China)** or **Lark Mail (International)** even if your address uses your company's domain:

1. Ask your administrator to allow third-party mail clients.
2. In the Feishu or Lark desktop app, open **Settings → Mail → Third-party email client login** and generate a device-specific password.
3. Enter your full member email address and that password in MailX.

</details>

<details>
<summary><strong>Exchange (self-hosted)</strong></summary>

For a company-hosted Exchange mailbox, choose **Exchange (self-hosted)**. Enter your email address and click **Find server**, or enter the server address from your administrator, then enter your login name (such as `DOMAIN\user`) and password. If you need a proxy, set it before clicking **Find server**.

</details>

<details>
<summary><strong>Other mailboxes (Manual setup)</strong></summary>

If your provider is not listed, choose **Manual setup** and enter the incoming (IMAP) and outgoing (SMTP) server addresses, ports and encryption settings provided by your mail service.

</details>

## Managing existing accounts

Open **Accounts** in the sidebar to adjust any mailbox:

- **Name and look**: change the display name, sender name, icon and color tag (handy for telling mailboxes apart in the unified inbox). Changes save automatically.
- **Check or update sign-in**: use **Verify connection** and **Edit connection** (or **Sign in again** for browser sign-in accounts) at the bottom of the page. Updating an expired password keeps all your mail and settings intact.
- **Per-account proxy**: each mailbox can have its own proxy. If the proxy cannot be reached, MailX stops rather than quietly connecting directly.
- **Status indicators**: an amber exclamation mark means a temporary network issue that MailX will retry on its own; a red mark means sync has stopped — open the account to see why and how to fix it.
- **Remove account**: click **Remove account** in red at the bottom left to remove the account and its local cache from this computer. Mail on your provider's server is not deleted.
