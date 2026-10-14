# MailX

一款极速启动、兼容市面所有邮箱、本地数据优先、AI 原生（内置全功能智能引擎 + 双向 MCP 互通）的跨平台开源邮件工作台。

MailX 围绕快速启动、广泛的邮箱兼容性和流畅的日常收发体验设计，让个人邮箱、工作邮箱与企业邮箱在同一个轻量工作台中统一管理。桌面端基于 Rust + Tauri，面向 Windows、macOS 和 Linux，后续提供单用户 Docker Web 版本。

> **项目状态：工程骨架阶段。** 已建立 Rust 核心库、React + TypeScript + Vite 前端和 Tauri 桌面宿主，前端开发服务器与桌面开发命令已经打通；邮件功能尚未实现，当前页面仍是初始化模板。上述定位与下述亮点描述产品愿景，目前没有可用的邮件客户端、安装包或 Docker 镜像，设计可随项目演化调整。

项目仓库：[GuqierMcl/MailX](https://github.com/GuqierMcl/MailX)。当前为私有仓库，按开源项目的文档、贡献和版本管理方式维护；计划在首个 MVP 版本发布后公开并开源。

## 核心亮点

- **极速轻快**：以快速启动、低资源占用和流畅交互为目标，打开即可进入邮件工作。
- **广泛兼容**：面向国内外主流邮箱与企业邮箱，通过通用邮件协议、服务商适配和配置预设持续扩展兼容范围。
- **统一收发**：多账号收件箱、离线阅读、快速检索、草稿与可靠发信，覆盖日常邮件工作。
- **分账号路由**：每个邮箱独立选择直连或代理，适应不同账号的网络环境。
- **本地优先**：邮件在本地存储与检索，不依赖 MailX 云端中继，远程图片默认关闭。
- **AI 原生与互通**：内置全功能智能引擎与双向 MCP 互通，将智能辅助融入阅读、整理和撰写流程。

首版以桌面端为交付重点，具体功能、邮箱兼容范围与 Web 版顺序见[产品范围](docs/product/README.md)。AI 可使用自带 Key 的云端模型或本地模型；使用云端模型时，所选内容会发送至对应提供商，详见 [AI 设计](docs/ai-runtime/README.md)。

项目与传统 Unix `mailx` 命令无关，可执行程序、命令行别名与安装包名称尚未确定。

## 参与开发

从[文档入口](docs/README.md)开始，或直接阅读：

- [架构总览](docs/architecture/overview.md)：模块划分、技术选型与目标代码结构。
- [实施路线图](docs/roadmap/README.md)：里程碑、依赖关系和完成标准。
- [开发指南](docs/development/README.md)：环境、构建检查与下一条收信链路。
- [贡献说明](CONTRIBUTING.md)：提交和验证约定。

根目录 `core/` 管理共享核心与通用实现，`frontend/` 是 React + TypeScript + Vite + shadcn 前端工程，`desktop/` 是 Tauri 宿主；`server/` 留待第二阶段服务器 MVP。数据库与协议库尚未引入。

准备 Bun、Rust 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)后，在仓库根目录执行：

```sh
bun install
bun run dev                 # 仅启动前端 Vite 开发服务器
bun tauri dev               # 启动前端 + Tauri 桌面开发环境（等价于 bun run tauri dev）
bun run build               # 构建前端静态资源
bun tauri build             # 构建桌面 release 二进制；当前不生成安装包
bun run check               # 版本、TypeScript、ESLint 与 Rust check
```

根目录 `package.json` 是命令入口和工作区版本入口，`bun run version:check` 会校验根目录、`frontend/package.json` 与 Cargo workspace 的版本一致。工具链固定为 Rust 1.94.0，Bun 版本记录在 `package.json`，依赖解析记录在 `bun.lock` 与 `Cargo.lock`。完整验证命令见[开发指南](docs/development/README.md)。

## 许可证

本项目采用 [Apache License 2.0](LICENSE)（SPDX：`Apache-2.0`）。

Copyright 2026 GuqierMcl and MailX contributors.
