# MailX

一个以本地数据为基础、面向 AI 工作流的跨平台邮件工作台。

MailX 计划将多账号收发、离线检索、分账号代理、AI 辅助阅读与拟信，以及 MCP 工具访问整合到同一应用中。桌面端基于 Rust + Tauri，后续提供单用户 Docker Web 版本。项目与传统 Unix `mailx` 命令无关，计划使用 `mailx-app` 作为桌面程序名称。

> **项目状态：初始设计阶段。** 当前仓库记录项目启动时的愿景、第一版架构草案和实施建议，尚无可运行客户端、安装包或 Docker 镜像。下述能力均为开发目标，设计与范围可随项目演化调整，不构成长期承诺。

项目仓库：[GuqierMcl/MailX](https://github.com/GuqierMcl/MailX)。当前为私有仓库，按开源项目的文档、贡献和版本管理方式维护；计划在首个 MVP 版本发布后公开并开源。

## 首版目标

- **多账号邮件工作台**：IMAP / SMTP、统一收件箱、离线阅读、草稿与可靠发信。
- **分账号网络策略**：直连、应用默认路由或自定义 SOCKS5 / HTTP CONNECT 代理。
- **本地优先**：邮件存入本机数据库；不依赖 MailX 云端中继；远程图片默认关闭。
- **可选 AI**：自带模型 API Key 或连接本地 Ollama，支持摘要、待办提取和辅助拟信。
- **MCP 接入**：外部 Agent 可在授权范围内检索、读取邮件和创建草稿，发送由用户确认。

首版以桌面端为交付重点；邮箱兼容范围与 Web 版顺序见[产品范围](docs/product/README.md)。调用云端模型会将所选内容发往该提供商，具体边界见 [AI 设计](docs/ai-runtime/README.md)。

## 参与开发

从[文档入口](docs/README.md)开始，或直接阅读：

- [架构总览](docs/architecture/overview.md)：模块划分、技术选型与目标代码结构。
- [实施路线图](docs/roadmap/README.md)：里程碑、依赖关系和完成标准。
- [开发指南](docs/development/README.md)：从第一条收信链路开始落地。
- [贡献说明](CONTRIBUTING.md)：提交和验证约定。

计划采用 **Rust / Cargo、Tauri v2、React / TypeScript / Vite、Bun、SQLite**。当前仅需 Git 和文本编辑器即可参与文档工作；应用启动命令将在工程脚手架落地后补充。

## 许可证

本项目采用 [Apache License 2.0](LICENSE)（SPDX：`Apache-2.0`）。

Copyright 2026 GuqierMcl and MailX contributors.
