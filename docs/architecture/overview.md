# MailX 第一版架构

状态：**部分 Implemented / 其余 Initial draft** · 2026-10-18。已建立 core、Tauri 宿主、shadcn 邮件工作台与设置弹窗，接通配置持久化、系统凭据库、账号路由及 IMAP 文件夹和收件箱邮件头同步。桌面端可分页查询缓存头部、搜索主题和发件人；正文、标记写回与发信尚未实现，真实服务商已有部分冒烟证据，完整兼容性仍待验证。下文区分当前能力与后续设想，所有设计仍可演化。

## 1. 总体结构

采用模块化单体与轻量宿主。每个数据目录只有一个 Rust 核心实例拥有数据库、同步任务和发件队列。桌面与 Web 复用核心库；独立部署时不共享数据目录。

```text
前端工作台（React + TypeScript + Vite + shadcn；当前为账号设置与只读邮件头）
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
| UI | React + TypeScript + Vite，已初始化 | 三栏工作台与设置弹窗；桌面通过 MailxClient 读取同步文件夹、收件箱头部、统一列表与分页，普通浏览器预览使用 mock；HTTP 留待后续 |
| UI 基础 | shadcn `base-mira` preset，已初始化 | 账号业务界面使用 shadcn 组件组合，缺少组件时按现有 preset 安装 |
| 桌面 | Tauri v2，已初始化 | 使用 `frontend/dist` / Vite `devUrl`；账号 IPC、user-data-dir 与系统凭据库装配已接通；正文隔离待实现 |
| 核心 | Rust，`MailxCore` 应用服务门面 | 账号存储为单连接加锁的短事务，宿主经阻塞线程调用；每个账号一个 tokio 后台同步任务，运行时由宿主注入；持锁期间不做网络 I/O |
| 接收 / 发送 | `async-imap` 0.12 用于文件夹与收件箱头部同步；`lettre` 候选 | 经账号路由完成只读 `EXAMINE`、`UID SEARCH` 和 `UID FETCH`；SMTP、OAuth 与真实服务商组合仍需验证 |
| MIME / HTML | `mail-parser` 0.11 + `full_encoding` 已用于头部；`ammonia` 候选 | 多字符集头部解码、BODYSTRUCTURE 附件标志；正文解析、HTML 净化与 iframe + CSP 待实现 |
| 存储 | SQLite + `rusqlite`，已采用 | 账号、文件夹、邮件头与游标使用单连接、WAL、FULL 同步、外键、迁移校验与目录锁；头部使用参数化 LIKE 查询，FTS 待实现 |
| AI | Rust `reqwest` 流式适配器 | v0.1 OpenAI 兼容与 Ollama；不同协议不伪装成同一种接口 |
| MCP | [官方 Rust SDK `rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 候选 | M0 验证 SDK / 客户端协议协商，首版 stdio |
| Web | Axum + 同源静态 SPA + SSE | v0.2；单用户、服务端 Session、TOTP |
| 工具链 | Rust / Cargo 1.94.0、Bun 1.3.12，均已记录 | 根 `package.json` 提供命令与版本入口；`bun.lock`、`Cargo.lock` 分别锁定依赖 |

选型理由见 [ADR 0001](../adr/0001-shared-core-and-hosts.md)、[ADR 0002](../adr/0002-stack-and-storage.md)。协议库的官方接口文档用于 M0 接入验证：[async-imap](https://docs.rs/async-imap/latest/async_imap/)、[lettre](https://docs.rs/lettre/latest/lettre/transport/smtp/struct.AsyncSmtpTransport.html)。

## 3. 目标目录与依赖

关键目录直接放在仓库根目录。`core/` 与 `desktop/` 是现有 Cargo 包，`frontend/` 是 Bun 工作区中的前端包；`server/` 仍只占位，`.github/workflows/ci.yml` 已配置前端与三平台 Rust 检查。Cargo 包名表达当前工程依赖，不冻结对外程序、安装包或镜像名称。

```text
Cargo.toml                  # workspace：core + desktop
Cargo.lock                  # Rust 依赖解析结果
rust-toolchain.toml          # Rust 1.94.0、rustfmt、Clippy
core/                       # mailx-core，共享核心；业务与通用实现均在这里
  src/app.rs                # MailxCore：宿主共用的应用服务门面
  src/accounts/             # 账号服务、写入型认证输入与脱敏 DTO、同步凭证解析
  src/net/                  # 按账号路由建立直连 / SOCKS5 / HTTP CONNECT 与 TLS
  src/imap/                 # IMAP 会话与修改版 UTF-7
  src/sync/                 # 文件夹 / 邮件头解析、同步计划与每账号后台任务
  src/storage/              # SQLite、迁移、文件夹 / 邮件头缓存与游标、凭证恢复和实例锁
  src/secrets.rs            # 宿主注入的 SecretStore
desktop/                    # mailx-desktop，Tauri 宿主
  src/                      # 程序入口与 IPC 适配
  capabilities/             # 主窗口权限
  icons/                    # 正式桌面应用图标与托盘图标资源
server/                     # 第二阶段服务器宿主，当前占位
frontend/                   # React + TypeScript + Vite + shadcn 前端
  src/features/accounts/    # 账号配置表单
  src/features/mail/        # 三栏邮件工作台
  src/features/settings/    # 设置弹窗
  src/lib/mailx-client.ts    # 传输适配
  src/generated/            # Rust DTO 生成的 TypeScript
  package.json              # 前端脚本与依赖
  vite.config.ts            # Vite / Tailwind 配置
docs/                       # 文档
branding/                   # 主 Logo 源稿、品牌变体、分享素材与资源清单
.github/workflows/ci.yml     # 前端检查与 Linux / macOS / Windows Rust 检查
```

当前依赖为 **`desktop → core`**，第二阶段增加 **`server → core`**。不再单设 `infra` 包：存储、邮件协议、网络路由和模型适配随功能一起放入 `core` 的内部模块。宿主无关意味着 core 不导入 Tauri、Axum 或 UI API，可以依赖 SQLite 驱动、协议库和 HTTP 客户端；不要求为每个实现预建 trait。平台特定能力按需要由宿主注入。

邮件同步、收信、发信、重试与持久化均由 Rust core 承担，前端负责展示、输入和交互。当前账号 IPC 调用 `AccountService` 的创建、列表、详情与修改；后续 HTTP / MCP 也应调用同一套核心服务。出现独立发布、可选重依赖或明确复用需求时，再评估拆包。

账号以 Rust DTO 为跨边界类型来源，通过 `ts-rs` 生成 TypeScript；输入秘密与读取结果分开。前端仅经 `MailxClient` 调用宿主，生成产物可检查漂移。接口仍处于开发期，未冻结对外兼容承诺。

## 4. 核心职责

当前 `MailxCore` / `AccountService` 已提供账号、文件夹和邮件头查询，`SyncManager` 管理每账号的只读后台同步；表中服务名描述职责，尚未单独建立的服务按实际功能拆分，不要求先创建空目录或接口。

| 模块 | 拥有的状态 / 责任 | 不承担 |
| --- | --- | --- |
| AccountService | 账号、端点、认证配置、路由、凭证引用与恢复；当前未探测连接能力 | 返回密钥明文 |
| SyncManager | 每账号任务、文件夹目录、邮件头游标、增量与只读对账；后续再扩展写操作 | UI 排序和模型判断 |
| 邮件读取与搜索（当前在 AccountService / Storage） | 缓存头部读取、参数化 LIKE、稳定分页；正文和状态变更待实现 | 让前端执行 SQL |
| DraftService / OutboxService | 草稿版本、发送快照、幂等与结果状态 | 模型自主确认发送 |
| AiService | 选定上下文、运行、流式结果、取消 | 直接持有邮件协议连接 |
| PolicyService | 账号范围、动作权限、内容披露与确认 | 依赖前端按钮隐藏来授权 |
| Repository / BlobStore | 持久化、迁移、缓存与备份 | 网络调用跨越事务 |

当前仅本地受信主窗口可调用账号管理和邮件读取 IPC，参数不接受客户端自报主体或秘密引用。未来多入口请求携带由宿主构造的 `RequestContext`：调用主体、来源、授权账号集合、权限集合、`requestId`。客户端不能自报自己是 UI 或管理员。单用户实例仍需隔离不同 MCP 客户端的授权。

## 5. 关键数据流

当前账号配置链路为：shadcn 表单 → MailxClient → 受限 Tauri IPC → AccountService → SecretStore 与 SQLite 更新恢复。保存配置的短事务不连接邮箱，成功后宿主启动或重启该账号的后台同步任务；完整模型与操作边界见[账号配置](account-configuration.md)。下列头部同步已实现，正文、发送与 AI 链路仍为后续设计。

1. **同步（当前）**：账号配置 → 路由与认证 → `STATUS` / `EXAMINE` / `UID SEARCH` → 每批最多 200 封头部 → 短事务写入与游标推进 → `idle` 事件 → UI 重新查询。每 60 秒重新登录并对账收件箱，文件夹目录每 10 分钟刷新；不下载正文。
2. **阅读（后续）**：缓存查询 → 缺失则创建获取任务 → MIME 解码 → 纯文本与净化 HTML → 隔离渲染。远程图片由受控资源加载器按单封授权处理。
3. **发送**：本地草稿 → 用户检查完整收件人与附件 → 固化版本与 MIME → 单个队列项 → SMTP → 记录 accepted / failed / unknown。网络不确定时不自动重发。
4. **AI**：用户选定内容与模型 → 披露策略检查 → 后端请求模型 → 流式展示 → 保存建议或新草稿。建议不直接变成已发送邮件。
5. **外部 Agent**：显式配对 → 限定账号 / 工具 → MCP → 应用权限 → 与 UI 相同服务 → 返回限量纯文本，不返回密钥和宿主路径。

详细状态与恢复语义分别见[同步](mail-sync.md)、[应用契约](../contracts/app-api.md)、[AI Runtime](../ai-runtime/README.md)。

## 6. 需要先验证的风险

M0 先证明 IMAP / SMTP 都能使用同一代理策略、OAuth 刷新不改变路由、数据库 FTS5 可用、邮件 iframe 无 IPC、MCP bridge 不产生第二个核心实例。随后再扩大 UI 和功能；风险关闭标准见[路线图](../roadmap/README.md)。
