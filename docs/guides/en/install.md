---
title: Install and update
description: Install MailX on macOS, Windows and Linux, and keep it up to date.
---

Choose the installer for your computer on the [download page](/en/download/), then open it to install.

## Installing on your system

### macOS

- **Apple silicon (M series)** and **Intel** use different installers. Not sure which you have? Click the Apple menu in the top-left corner → **About This Mac**, and check **Chip** or **Processor**.
- The macOS build is notarized by Apple and opens normally after download.

### Windows

- Built for 64-bit Windows. Run the installer and follow the steps on screen.
- **x64** (Intel / AMD) and **ARM64** (Snapdragon and others) have separate installers. Not sure which one? Open **Settings** → **System** → **About** and look at “System type”. ARM64 installers are offered from newer releases on.
- If Windows shows a blue security prompt the first time you open it, click **More info** → **Run anyway**.

### Linux

Choose **x64** for 64-bit Intel or AMD and **ARM64** for 64-bit ARM. Run `uname -m` in a terminal to check. `x86_64` means x64, and `aarch64` means ARM64.

Check the download page for formats available for your architecture: **AppImage**, **deb** (Debian / Ubuntu), or **rpm** (Fedora / openSUSE). For releases that include ARM64, use Ubuntu 24.04 or later on either architecture.

- **AppImage**: runs directly and supports automatic in-app updates.
- **deb / rpm**: install with your system's package manager. MailX lets you know when a new version is out so you can install it manually.

## Keeping MailX up to date

MailX checks for new versions shortly after launch and while running. Manage updates under **Settings → About**.

- **Automatic updates (default)**: once an update is downloaded, choose **Restart now**, or keep working and let it install the next time you quit MailX completely. MailX never restarts on its own.
- **Manual checks**: click **Check for updates** at any time. If you turn off automatic updates, you can read the release notes first and choose when to download.

:::note
Closing the window while keeping MailX in the background does not install an update — quit the app completely to finish upgrading. On macOS, upgrading from an older unsigned build to a signed one asks for Keychain access once; choose **Always Allow**.
:::

Want to keep a backup before upgrading? See [how to back up your local data](/en/docs/troubleshooting/#where-is-my-data-stored-how-do-i-back-it-up).
