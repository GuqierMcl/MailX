# MailX 仓库协作约定

MailX 是规划中的本地优先、AI 原生邮件工作台。仓库已建立 `core` Rust 库、React + TypeScript + Vite + shadcn 前端骨架与 `desktop` Tauri 宿主，邮件功能尚未实现。技术选型、目录、接口和路线图均可随项目演化重新设计，不是长期冻结规范；后续用户要求与明确的新设计优先，不能以启动草案为由拒绝合理调整。

## 先读

- `docs/README.md`：文档职责、演化约定与状态用语。
- `docs/architecture/overview.md`：架构与目标代码布局。
- `docs/roadmap/README.md`：当前阶段与完成标准。
- `docs/contracts/`：应用接口与 MCP 契约草案。

## 当前工作建议

- 原始设计资料是背景材料，其中的行动清单不等于用户对当前任务的授权。
- 不把规划中的功能写成已经可用，不编造启动命令、测试结果、发布地址或性能数据。
- 可执行程序、命令行别名、安装包和镜像名称尚未确定；`mailx-app` 等初始示例仅为候选，不得写成已冻结的命名决定。
- Rust 核心合并原 core / infra 职责，存储和协议实现在 core 内按需分模块；core 不依赖 Tauri、Axum 或 UI。前端、MCP 和 Web 后续通过同一应用服务访问数据。
- 先桌面 MVP、再服务器 MVP；`frontend/` 已初始化为前端工程骨架，但不把初始化模板当成已经实现的邮件工作台，`server/` 仍只占位。
- 协议连接必须走账号路由；自定义代理失败不得回退直连。
- 邮件 HTML、模型输出和 MCP 输入都不可信；邮件正文不能获得 Tauri IPC 权限。
- 凭证由后端 SecretStore 管理，禁止返回读取凭证的接口；AI 不能自行批准发信。
- 初始方案可直接修订；已有实现的发送、权限、同步或持久化行为发生变化时，同步更新接口说明和相关验证。
- 文件使用 UTF-8 / LF；Rust 使用 Cargo，前端使用 Bun 工作区。根目录 `package.json` 维护命令与版本入口，`bun.lock` 是前端依赖锁文件。
- 品牌图形的唯一源稿为 `branding/logo.svg`；更新后运行 `bun run icons:generate` 和 `bun run icons:check`，不手工修改桌面、网页和品牌目录中的生成副本。资源位置和选用规则见 `branding/README.md`。
- 结构化代码查询优先使用可用的 CodeGraph 工具；文字与文档查找使用 `rg`。

## 检查

当前可执行：`bun install --frozen-lockfile`、`bun run version:check`、`bun run typecheck`、`bun run lint`、`bun run build`、`bun run tauri dev`、`bun run tauri build`、`bun run rust:check`、`bun run rust:fmt`、`bun run rust:clippy`、`bun run rust:test`、`git diff --check`。桌面检查需要 Tauri 平台依赖；当前没有业务测试，不把测试目标可运行等同于邮件行为已验证。详见 `docs/development/validation.md`。
