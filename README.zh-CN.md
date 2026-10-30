<div align="center">

<img src="branding/logo.svg" alt="MailX Logo" width="112" />

# MailX

快速、轻量、本地优先的跨平台邮件客户端，兼容各类邮箱，AI 原生。

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/GuqierMcl/MailX?include_prereleases&sort=semver)](https://github.com/GuqierMcl/MailX/releases)
[![Downloads](https://img.shields.io/github/downloads/GuqierMcl/MailX/total)](https://github.com/GuqierMcl/MailX/releases)
![Platform: Windows | macOS | Linux](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

[English](README.md) | 简体中文

</div>

> [!NOTE]
> MailX 正处于早期开发阶段，变化可能较快，欢迎提交 Issue 和 Pull Request。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/mailx-dark.webp">
  <img src="docs/assets/screenshots/mailx-light.webp" alt="MailX 主窗口" width="900">
</picture>

## 特性

- **轻快** —— Rust 核心配合系统 WebView，不捆绑浏览器内核；打开即是本地缓存的邮件。支持 Windows、macOS 和 Linux。
- **所有账号，一个收件箱** —— 多个邮箱汇总到统一收件箱。搜索和翻页都在本地完成，已读和星标会同步回服务器。
- **安全阅读** —— 正文在打开时才下载，并在与应用隔离的沙箱中渲染；远程图片要等你允许才加载。
- **独立的写信窗口** —— 富文本撰写新邮件、回复和转发，可以添加附件、直接粘贴截图；草稿边写边保存在本地。
- **可靠的发送** —— 每封邮件都经过发送队列：发出后可撤销，失败自动重试，可能已被服务器接受的邮件不会自动重发。
- **按账号路由网络** —— 每个账号可以直连，也可以走自己的 SOCKS5 或 HTTP 代理；代理失败时直接停止，不会悄悄改为直连。
- **桌面原生体验** —— 新邮件通知，关闭窗口后驻留托盘，应用内更新，浅色与深色主题，中英文界面。

## 支持的邮箱

以下服务商内置预设：选择服务商，填写邮箱地址和服务商为客户端提供的密码或授权码，服务器设置会自动填好。

| <img src="frontend/src/assets/providers/qqmail.svg" alt="QQ 邮箱" height="32"> | <img src="frontend/src/assets/providers/netease-163.svg" alt="网易 163 邮箱" height="32"> | <img src="frontend/src/assets/providers/netease-126.svg" alt="网易 126 邮箱" height="32"> | <img src="frontend/src/assets/providers/netease-yeah.svg" alt="网易 Yeah 邮箱" height="32"> | <img src="frontend/src/assets/providers/mail-139.svg" alt="139 邮箱" height="32"> |
| :---: | :---: | :---: | :---: | :---: |
| QQ 邮箱 | 163 邮箱 | 126 邮箱 | Yeah 邮箱 | 139 邮箱 |

其他支持 IMAP / SMTP 密码登录的邮箱，可以手动填写服务器添加。需要 OAuth 登录的服务（如 Gmail、Outlook）暂不支持。与真实服务商的兼容性目前只做了部分验证，遇到问题欢迎[提交 issue](https://github.com/GuqierMcl/MailX/issues)。

<sub>服务商名称与 Logo 为其各自所有者的商标，仅用于标识服务商。</sub>

## 隐私与数据

- **邮件留在你的电脑上。** 邮件头、已下载的邮件和草稿都存在本地数据库里。没有 MailX 账号、云端中转，也不收集遥测数据。
- **密码存放在系统钥匙串**（macOS 钥匙串、Windows 凭据管理器、Linux Secret Service），应用不会再把它显示出来。
- **只做必要的连接。** MailX 只连接你的邮件服务器（设置了代理就经由代理），以及向 GitHub 检查更新（可以关闭）。发件人头像查询默认关闭，邮件里的远程图片会先询问。
- **发出的邮件自带图片。** 正文里的图片随邮件一起发出，收件人打开时不会从外部加载任何内容。

## 路线图

| 模块 | 状态 |
| --- | --- |
| 桌面端：收信、阅读、本地搜索、多账号 | ✅ 可用 |
| 桌面端：撰写与发送 | ✅ 可用 |
| 桌面端：在已发送文件夹中显示发出的邮件 | 🚧 进行中 |
| 全文搜索 | 🗓️ 规划中 |
| OAuth 登录（Gmail、Outlook） | 🗓️ 规划中 |
| AI 助手 | 🗓️ 规划中 |
| MCP 服务（经用户批准，让外部 AI 工具安全访问邮箱） | 🗓️ 规划中 |
| 服务器：自托管 Web 版本 | 🗓️ 规划中 |

各版本的变化见 [CHANGELOG](CHANGELOG.md)，更详细的路线图见 [docs/roadmap/README.md](docs/roadmap/README.md)。

## 快速开始

### 下载

早期构建发布在 [Releases](https://github.com/GuqierMcl/MailX/releases) 页面：

| 平台 | 文件 |
| --- | --- |
| macOS（Apple 芯片） | `MailX_<版本>_macos-arm64.dmg` |
| macOS（Intel） | `MailX_<版本>_macos-x64.dmg` |
| Windows | `MailX_<版本>_windows-x64-setup.exe` |
| Linux | `.AppImage`、`.deb` 或 `.rpm`（`linux-x64`） |

安装包尚未做代码签名，系统可能拦截首次启动，各版本发布说明里的安装说明介绍了打开方法。macOS、Windows 和 AppImage 版本可以在应用内自动更新。这些构建尚未在每个平台上逐一验证。

### 从源码构建

环境要求：

- [Bun](https://bun.sh/) 1.3.12（`package.json` 中固定的版本）
- Rust 1.94.0（由 `rust-toolchain.toml` 固定，首次使用时 rustup 会自动安装）
- [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)

```sh
bun install
bun tauri dev       # 以开发模式运行桌面应用
bun tauri build     # 构建当前平台的 release 安装包（未签名）
bun run check       # 运行版本、类型、Lint、测试、构建与 Rust 检查
```

完整命令列表见 [docs/development/README.md](docs/development/README.md)。

## 文档

- [文档入口](docs/README.md)
- [架构总览](docs/architecture/overview.md)
- [产品范围](docs/product/README.md)
- [AI 设计](docs/ai-runtime/README.md)
- [开发指南](docs/development/README.md)

## 参与贡献

欢迎参与贡献，请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 许可证

MailX 采用 [Apache License 2.0](LICENSE) 许可。

Copyright 2026 GuqierMcl and MailX contributors.

---

MailX 与 Unix 的 `mailx` 命令无关。
