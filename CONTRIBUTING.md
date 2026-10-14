# 参与 MailX

当前已建立 `core` Rust 库、React + TypeScript + Vite + shadcn 前端骨架与 `desktop` Tauri 宿主，邮件功能尚未实现。请先阅读[文档入口](docs/README.md)、[架构总览](docs/architecture/overview.md)和[里程碑](docs/roadmap/README.md)。

## 工作方式

1. 选择一个里程碑内可独立验证的任务，说明预期行为和验收证据。
2. 功能实现与对应架构、契约、测试说明一起提交。接口变动先明确调用方与兼容影响。
3. 文档中区分初始设想、后续设计与已实现行为。启动草案可以被新方案替代，不要求后续实现机械遵从；只有代码和验证证据齐备后才能标记为已实现。
4. 提交前检查 `git diff --check`，复核相对链接和敏感信息；Rust 改动执行格式、Clippy、测试和相关构建，详见[验证策略](docs/development/validation.md)。

## 工程约定

- Rust 使用 Cargo 与仓库固定工具链，提交 `Cargo.lock`；前端使用 Bun 工作区，提交根目录 `bun.lock`。前端的初始化模板不代表邮件功能已经实现。
- Rust 核心合并业务与通用实现，不单设 infra 包；不依赖 Tauri 或 Axum，后续 UI、HTTP、MCP 共用应用服务。
- Rust 内部使用 `snake_case`，跨边界 JSON 使用 `camelCase`；结构化错误和权限在核心服务统一处理。
- 凭证、邮箱正文、真实附件、模型提示词原文和本地连接配置不得进入提交、日志或测试快照。测试邮件使用合成数据。
- 涉及同步、发送、HTML、权限和迁移的改动，应包含故障路径验证；普通文档排版修改无需新增测试。
- 提交消息使用 Conventional Commits，例如 `docs(architecture): define mailbox sync lifecycle`。

PR 描述说明行为变化、验证结果和未解决限制。初始方案调整时更新对应文档即可；重大选型可通过 [ADR](docs/adr/README.md)保留取舍理由，不需要为了改变启动草案建立额外审批流程。

## 开源发布前

项目仓库为 [GuqierMcl/MailX](https://github.com/GuqierMcl/MailX)。当前保持私有，按开源项目方式维护文档、贡献流程与版本记录；计划在首个 MVP 版本发布后公开并开源。

项目采用 [Apache License 2.0](LICENSE)。维护者联系渠道和安全问题报告渠道在公开前确定，并检查提交历史中是否包含敏感信息。下载地址、徽章和发布命令以实际产物为准。
