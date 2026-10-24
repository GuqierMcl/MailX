<div align="center">

<img src="branding/logo.svg" alt="MailX Logo" width="112" />

# MailX

快速、轻量、本地优先的跨平台邮件客户端，兼容各类邮箱，AI 原生。

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![CI](https://github.com/GuqierMcl/MailX/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/GuqierMcl/MailX/actions/workflows/ci.yml)
![Platform: Windows | macOS | Linux](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

[English](README.md) | 简体中文

</div>

> [!NOTE]
> MailX 正处于早期开发阶段，变化可能较快，欢迎提交 Issue 和 Pull Request。

<p align="center">
  <img src="docs/assets/screenshots/mailx-light.webp" alt="MailX 主界面（浅色模式）" width="900">
</p>
<p align="center">
  <img src="docs/assets/screenshots/mailx-dark.webp" alt="MailX 主界面（深色模式）" width="900">
</p>

## 特性

- **快速**：Rust 核心优先读取本地缓存，打开即可进入邮件。
- **轻量**：基于 Tauri，使用系统自带的 WebView，不内置浏览器引擎。
- **跨平台**：一套代码同时面向 Windows、macOS 和 Linux。
- **多邮箱合一**：多个账号汇总到统一收件箱，面向标准 IMAP/SMTP 邮箱；每个账号可独立选择直连或通过 SOCKS5、HTTP CONNECT 代理，代理失败时不会悄悄回退为直连。
- **本地优先，注重隐私**：邮件在本机存储与检索，不经过 MailX 云端中继；远程图片默认关闭，邮件正文在隔离沙箱中渲染。
- **AI 原生**（规划中）：内置助手协助阅读、整理和撰写，可使用自带 Key 的云端模型或本地模型，并支持双向 MCP 互通；AI 不会自行发送邮件。
- **可自托管**（规划中）：提供可用 Docker 部署的单用户 Web 版本。

## 路线图

| 模块 | 状态 |
| --- | --- |
| 桌面端：收信、阅读、搜索、多账号 | 🚧 进行中 |
| 桌面端：撰写与发送 | 🚧 进行中 |
| 富文本撰写与附件 | 🗓️ 规划中 |
| 全文搜索 | 🗓️ 规划中 |
| OAuth 登录（Gmail、Outlook） | 🗓️ 规划中 |
| AI 助手 | 🗓️ 规划中 |
| MCP 服务（经用户批准，让外部 AI 工具安全访问邮箱） | 🗓️ 规划中 |
| 服务器：自托管 Web 版本 | 🗓️ 规划中 |

更详细的路线图见 [docs/roadmap/README.md](docs/roadmap/README.md)。

## 快速开始

### 从源码构建

目前尚无预编译安装包。

环境要求：

- [Bun](https://bun.sh/) 1.3.12（`package.json` 中固定的版本）
- Rust 1.94.0（由 `rust-toolchain.toml` 固定，首次使用时 rustup 会自动安装）
- [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)

```sh
bun install
bun tauri dev       # 以开发模式运行桌面应用
bun tauri build     # 构建 release 二进制（目前不生成安装包）
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
