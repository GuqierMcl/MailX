<div align="center">

<img src="branding/logo.svg" alt="MailX logo" width="112" />

# MailX

A fast, lightweight, local-first email client — built for every inbox, with AI at its core.

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/GuqierMcl/MailX?include_prereleases&sort=semver)](https://github.com/GuqierMcl/MailX/releases)
[![Downloads](https://img.shields.io/github/downloads/GuqierMcl/MailX/total)](https://github.com/GuqierMcl/MailX/releases)
![Platform: Windows | macOS | Linux](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

English | [简体中文](README.zh-CN.md)

</div>

> [!NOTE]
> MailX is in early development. Things may change quickly — issues and pull requests are welcome.

<p align="center">
  <img src="docs/assets/screenshots/mailx-light.webp" alt="MailX main window in light mode" width="900">
</p>
<p align="center">
  <img src="docs/assets/screenshots/mailx-dark.webp" alt="MailX main window in dark mode" width="900">
</p>

## Features

- **Fast** — A Rust core reads from the local cache first, so MailX opens straight into your mail.
- **Lightweight** — Built on Tauri with the system WebView instead of bundling a browser engine.
- **Cross-platform** — Windows, macOS and Linux from a single codebase.
- **All in One** — Multiple accounts share one unified inbox. MailX is designed for standard IMAP/SMTP mailboxes, and each account can connect directly or through a SOCKS5 or HTTP CONNECT proxy, with no silent fallback to a direct connection.
- **Rich-text composing** — A lightweight Tiptap editor in its own compose window: headings, bold/italic/underline/strikethrough, links, lists, quotes, alignment, a fixed color palette and Markdown shortcuts. Mail goes out as plain text plus HTML, and the core sanitizes everything you write. Attach files from a button or by dragging them in (up to 20 MiB in total), and forwards carry the original attachments. Drafts are saved locally, sends can be undone, and a message that may already have been accepted is never resent on its own.
- **Local-first & private** — Mail is stored and searched on your machine, with no MailX cloud relay. Remote images are off by default, and message bodies render in an isolated sandbox.
- **AI-native** *(planned)* — A built-in assistant for reading, organizing and writing, using your own cloud model key or a local model, plus two-way MCP integration. AI never sends mail on its own.
- **Self-hostable** *(planned)* — A single-user web version you can deploy with Docker.

## Roadmap

| Module | Status |
| --- | --- |
| Desktop app: receiving, reading, search, multiple accounts | 🚧 In progress |
| Desktop app: composing and sending (rich-text editor, replies and forwards, attachments, reliable send queue) | 🚧 In progress |
| Inline images when composing | 🗓️ Planned |
| Full-text search | 🗓️ Planned |
| OAuth sign-in (Gmail, Outlook) | 🗓️ Planned |
| AI assistant | 🗓️ Planned |
| MCP server (lets external AI tools access your mailbox safely, with your approval) | 🗓️ Planned |
| Server: self-hosted web version | 🗓️ Planned |

The detailed roadmap is in [docs/roadmap/README.md](docs/roadmap/README.md) (written in Chinese).

## Getting Started

### Download

Early builds for Windows, macOS and Linux are published on the [Releases](https://github.com/GuqierMcl/MailX/releases) page. They are not code-signed, so your system may block the first launch; the install notes in each release explain how to open them. These builds have not been verified on every platform yet.

### Build from source

Prerequisites:

- [Bun](https://bun.sh/) 1.3.12 (the version pinned in `package.json`)
- Rust 1.94.0 (pinned in `rust-toolchain.toml`; rustup installs it on first use)
- [Tauri platform dependencies](https://v2.tauri.app/start/prerequisites/)

```sh
bun install
bun tauri dev       # run the desktop app in development mode
bun tauri build     # build release installers for the current platform (unsigned)
bun run check       # run the version, type, lint, test, build and Rust checks
```

See [docs/development/README.md](docs/development/README.md) for the full list of commands.

## Documentation

Documentation is currently written in Chinese.

- [Documentation index](docs/README.md)
- [Architecture overview](docs/architecture/overview.md)
- [Product scope](docs/product/README.md)
- [AI design](docs/ai-runtime/README.md)
- [Development guide](docs/development/README.md)

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. To report a security issue, follow [SECURITY.md](SECURITY.md) and report it privately.

## License

MailX is licensed under the [Apache License 2.0](LICENSE).

Copyright 2026 GuqierMcl and MailX contributors.

---

MailX is not related to the Unix `mailx` command.
