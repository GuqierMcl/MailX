# MailX 第一版架构

状态：**部分 Implemented / 其余 Initial draft** · 2026-10-14。根目录 Cargo workspace、`core/`、React + TypeScript + Vite + shadcn 前端骨架与 `desktop/` 已建立；邮件功能尚未实现。下文区分当前骨架与后续设想，所有设计仍可演化。

## 1. 总体结构

采用模块化单体与轻量宿主。每个数据目录只有一个 Rust 核心实例拥有数据库、同步任务和发件队列。桌面与 Web 复用核心库；独立部署时不共享数据目录。

```text
前端工作台（React + TypeScript + Vite + shadcn；当前为初始化模板）
    └── MailxClient 接口
         ├── Tauri IPC adapter ── 桌面宿主
         └── HTTP + SSE adapter ─ Web 宿主（v0.2）
                                      │
外部 Agent ─ stdio ─ MCP bridge ─ 本地受控 IPC
                                      │
                        应用服务 + RequestContext + 权限策略
                         ├── 账号 / 同步 / 草稿 / 发件队列
                         ├── 本地检索 / 阅读模型 / 附件
                         ├── AI Run / 内容选择 / 草稿建议
                         └── 事件 / 审计 / 任务取消
                                      │
                  core 内部的存储 / 协议 / 路由 / 模型模块
                  按实际需要定义 SecretStore 等可替换接口
                                      │
                SQLite + blob 文件 / OS Keyring / IMAP + SMTP /
                账号级路由 / 用户选择的模型端点
```

MCP bridge 只转发受限调用，不打开数据库或启动第二套同步器。内置 AI 通过内部应用服务执行固定工作流；v0.1 不依赖 MCP 回环，也不实现通用 Agent 插件执行器。

## 2. 技术选型

| 层 | 设计选择 | 实施约束 |
| --- | --- | --- |
| UI | React + TypeScript + Vite，已初始化 | 当前只有模板页；通过统一客户端适配器访问桌面 IPC 或未来 HTTP |
| UI 基础 | shadcn `base-mira` preset，已初始化 | 组件基础已建立，服务状态、交互状态和邮件领域模型仍待实现 |
| 桌面 | Tauri v2，已初始化 | 使用 `frontend/dist` / Vite `devUrl`；仅有核心版本 IPC；正文隔离待实现 |
| 核心 | Rust，已初始化；Tokio 为后续候选 | 单个 crate 承担业务与通用实现，当前无第三方依赖 |
| 接收 / 发送 | `async-imap` / `lettre` 候选 | M0 必须验证自定义流、TLS、OAuth 和代理；不能因库限制绕过账号路由 |
| MIME / HTML | `mail-parser` / `ammonia` 候选 | 限制大小、递归深度；净化之外仍需 iframe + CSP |
| 存储 | SQLite 方向；SQLx / `rusqlite` 待定 | 驱动、执行模型与迁移方式经验证后确定，当前未引入 |
| AI | Rust `reqwest` 流式适配器 | v0.1 OpenAI 兼容与 Ollama；不同协议不伪装成同一种接口 |
| MCP | [官方 Rust SDK `rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 候选 | M0 验证 SDK / 客户端协议协商，首版 stdio |
| Web | Axum + 同源静态 SPA + SSE | v0.2；单用户、服务端 Session、TOTP |
| 工具链 | Rust / Cargo 1.94.0、Bun 1.3.12，均已记录 | 根 `package.json` 提供命令与版本入口；`bun.lock`、`Cargo.lock` 分别锁定依赖 |

选型理由见 [ADR 0001](../adr/0001-shared-core-and-hosts.md)、[ADR 0002](../adr/0002-stack-and-storage.md)。协议库的官方接口文档用于 M0 接入验证：[async-imap](https://docs.rs/async-imap/latest/async_imap/)、[lettre](https://docs.rs/lettre/latest/lettre/transport/smtp/struct.AsyncSmtpTransport.html)。

## 3. 目标目录与依赖

关键目录直接放在仓库根目录。`core/` 与 `desktop/` 是现有 Cargo 包，`frontend/` 是 Bun 工作区中的前端包；`server/` 和 `.github/` 仍只占位。Cargo 包名表达当前工程依赖，不冻结对外程序、安装包或镜像名称。

```text
Cargo.toml                  # workspace：core + desktop
Cargo.lock                  # Rust 依赖解析结果
rust-toolchain.toml          # Rust 1.94.0、rustfmt、Clippy
core/                       # mailx-core，共享核心；业务与通用实现均在这里
  src/lib.rs                # 当前只提供版本信息
desktop/                    # mailx-desktop，Tauri 宿主
  src/                      # 程序入口与 IPC 适配
  capabilities/             # 主窗口权限
  icons/                    # 正式桌面应用图标与托盘图标资源
server/                     # 第二阶段服务器宿主，当前占位
frontend/                   # React + TypeScript + Vite + shadcn 前端骨架
  src/                      # 页面与 UI 组件
  package.json              # 前端脚本与依赖
  vite.config.ts            # Vite / Tailwind 配置
docs/                       # 文档
branding/                   # 主 Logo 源稿、品牌变体、分享素材与资源清单
.github/                    # 协作与 CI，当前占位
```

当前依赖为 **`desktop → core`**，第二阶段增加 **`server → core`**。不再单设 `infra` 包：存储、邮件协议、网络路由和模型适配随功能一起放入 `core` 的内部模块。宿主无关意味着 core 不导入 Tauri、Axum 或 UI API，可以依赖 SQLite 驱动、协议库和 HTTP 客户端；不要求为每个实现预建 trait。平台特定能力按需要由宿主注入。

邮件同步、收信、发信、重试与持久化均由 Rust core 承担，前端负责展示、输入和交互。当前 `desktop/src/commands.rs` 仅把核心版本交给前端模板，没有业务流程；后续 IPC / HTTP / MCP 仍应调用同一套核心服务。出现独立发布、可选重依赖或明确复用需求时，再评估拆包。

后续以 Rust DTO 作为跨边界类型来源，再决定 TypeScript / Schema 生成工具。正式前端通过统一客户端适配器调用；当前模板保留的 Tauri 调用仅用于验证宿主，不是已冻结的应用契约。

## 4. 后续核心职责

以下服务与模块尚未实现，按实际功能逐步建立，不要求先创建所有目录或接口。

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
