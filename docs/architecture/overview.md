# MailX 第一版架构

状态：**Initial draft** · 2026-10-14 · 本文中的模块和路径均为启动阶段的建议方案，尚无源代码；后续设计可以调整或替换。

## 1. 总体结构

采用模块化单体与轻量宿主。每个数据目录只有一个 Rust 核心实例拥有数据库、同步任务和发件队列。桌面与 Web 复用核心库；独立部署时不共享数据目录。

```text
React 工作台（阅读 / 撰写 / 搜索 / AI / 设置）
    └── MailxClient 接口
         ├── Tauri IPC adapter ── 桌面宿主 mailx-app
         └── HTTP + SSE adapter ─ Web 宿主 mailx-server（v0.2）
                                      │
外部 Agent ─ stdio ─ MCP bridge ─ 本地受控 IPC
                                      │
                        应用服务 + RequestContext + 权限策略
                         ├── 账号 / 同步 / 草稿 / 发件队列
                         ├── 本地检索 / 阅读模型 / 附件
                         ├── AI Run / 内容选择 / 草稿建议
                         └── 事件 / 审计 / 任务取消
                                      │
                  Repository / SecretStore / MailTransport /
                  NetworkConnector / ModelProvider 等端口
                                      │
                SQLite + blob 文件 / OS Keyring / IMAP + SMTP /
                账号级路由 / 用户选择的模型端点
```

MCP bridge 只转发受限调用，不打开数据库或启动第二套同步器。内置 AI 通过内部应用服务执行固定工作流；v0.1 不依赖 MCP 回环，也不实现通用 Agent 插件执行器。

## 2. 技术选型

| 层 | 设计选择 | 实施约束 |
| --- | --- | --- |
| UI | React + TypeScript + Vite | 与 NexusPilot 保持开发经验一致，UI 框架不决定邮件性能 |
| UI 基础 | Tailwind CSS + shadcn/ui，TanStack Query，少量 Zustand | Query 管服务状态；Zustand 只管布局与交互 |
| 桌面 | Tauri v2 | 邮件阅读 iframe 无 IPC 能力；按窗口缩小 capabilities |
| 核心 | Rust + Tokio | 宿主无关；有界任务队列、超时、取消、可恢复状态 |
| 接收 / 发送 | `async-imap` / `lettre` 候选 | M0 必须验证自定义流、TLS、OAuth 和代理；不能因库限制绕过账号路由 |
| MIME / HTML | `mail-parser` / `ammonia` 候选 | 限制大小、递归深度；净化之外仍需 iframe + CSP |
| 存储 | SQLite + `rusqlite`，WAL + FTS5 | 专用阻塞数据库执行器，短事务，SQL 迁移文件 |
| AI | Rust `reqwest` 流式适配器 | v0.1 OpenAI 兼容与 Ollama；不同协议不伪装成同一种接口 |
| MCP | [官方 Rust SDK `rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 候选 | M0 验证 SDK / 客户端协议协商，首版 stdio |
| Web | Axum + 同源静态 SPA + SSE | v0.2；单用户、服务端 Session、TOTP |
| 工具链 | Cargo + Bun | M0 固定具体版本与锁文件；本设计不虚构已锁定依赖 |

选型理由见 [ADR 0001](../adr/0001-shared-core-and-hosts.md)、[ADR 0002](../adr/0002-stack-and-storage.md)。协议库的官方接口文档用于 M0 接入验证：[async-imap](https://docs.rs/async-imap/latest/async_imap/)、[lettre](https://docs.rs/lettre/latest/lettre/transport/smtp/struct.AsyncSmtpTransport.html)。

## 3. 目标目录与依赖

以下目录在对应里程碑创建，当前不放置无内容占位工程。

```text
apps/
  desktop/src-tauri/         # Tauri 宿主；桌面二进制 mailx-app
  web/                      # Web SPA 构建入口（v0.2）
packages/
  ui/                       # 可复用 React 工作台
  contracts/                # 生成的 TS DTO + JSON Schema
crates/
  mailx-core/               # domain、ports、services、policy、jobs
  mailx-infra/              # SQLite、协议、路由、密钥、模型适配
  mailx-mcp/                # 工具定义、stdio bridge、本地受控调用
  mailx-server/             # Axum 宿主（v0.2）
migrations/                 # 有序、不可改写的已发布 SQL 迁移
resources/providers/        # 审核后的邮箱预设，不含凭证
tests/fixtures/             # 合成 MIME / 协议 / HTML 样本
docs/                       # 文档单一来源
```

依赖方向：`mailx-infra → mailx-core`；桌面 / Web 宿主装配 core 与 infra；`mailx-mcp → mailx-core` 的 DTO / 应用端口，不直接依赖数据库实现。core 不导入 Tauri、Axum、SQLite 或具体模型 SDK。初期在 crate 内按模块分层，不为每张表建立独立 crate。

Rust DTO 作为跨边界类型来源，M0 选择生成工具，生成 TypeScript / Schema 后检查差异。前端只经 `MailxClient` 调用，组件不直接导入 Tauri API 或拼 HTTP 请求。

## 4. 核心职责

| 模块 | 拥有的状态 / 责任 | 不承担 |
| --- | --- | --- |
| AccountService | 账号配置、端点、能力、认证状态、密钥引用 | 返回密钥明文 |
| SyncCoordinator | 每账号任务、文件夹游标、增量与对账 | UI 排序和模型判断 |
| MailService / SearchService | 缓存读取、状态变更、受限搜索 | 让前端执行 SQL |
| DraftService / OutboxService | 草稿版本、发送快照、幂等与结果状态 | 模型自主确认发送 |
| AiService | 选定上下文、运行、流式结果、取消 | 直接持有邮件协议连接 |
| PolicyService | 账号范围、动作权限、内容披露与确认 | 依赖前端按钮隐藏来授权 |
| Repository / BlobStore | 持久化、迁移、缓存与备份 | 网络调用跨越事务 |

每次请求携带由宿主构造的 `RequestContext`：调用主体、来源、授权账号集合、权限集合、`requestId`。客户端不能自报自己是 UI 或管理员。单用户实例仍需隔离不同 MCP 客户端的授权。

## 5. 关键数据流

1. **同步**：账号配置 → 路由与认证 → 分页读取 UID / 头部 → 短事务写入与游标推进 → 发送失效事件 → UI 重新查询。正文按需读取，不阻塞壳启动。
2. **阅读**：缓存查询 → 缺失则创建获取任务 → MIME 解码 → 纯文本与净化 HTML → 隔离渲染。远程图片由受控资源加载器按单封授权处理。
3. **发送**：本地草稿 → 用户检查完整收件人与附件 → 固化版本与 MIME → 单个队列项 → SMTP → 记录 accepted / failed / unknown。网络不确定时不自动重发。
4. **AI**：用户选定内容与模型 → 披露策略检查 → 后端请求模型 → 流式展示 → 保存建议或新草稿。建议不直接变成已发送邮件。
5. **外部 Agent**：显式配对 → 限定账号 / 工具 → MCP → 应用权限 → 与 UI 相同服务 → 返回限量纯文本，不返回密钥和宿主路径。

详细状态与恢复语义分别见[同步](mail-sync.md)、[应用契约](../contracts/app-api.md)、[AI Runtime](../ai-runtime/README.md)。

## 6. 需要先验证的风险

M0 先证明 IMAP / SMTP 都能使用同一代理策略、OAuth 刷新不改变路由、数据库 FTS5 可用、邮件 iframe 无 IPC、MCP bridge 不产生第二个核心实例。随后再扩大 UI 和功能；风险关闭标准见[路线图](../roadmap/README.md)。
