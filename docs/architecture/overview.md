# MailX 架构总览

状态：**桌面邮件核心与本地 MCP 已实现 / 内置 AI 和 Web 为 Planned**。core、Tauri 宿主和 React 工作台提供账号与凭据管理、按账号路由的 IMAP / SMTP、本地缓存和搜索、隔离阅读、邮件整理、富文本草稿与发送队列，以及本地 MCP 工具入口。下文区分实现边界与后续设想；具体平台、服务商和客户端的测试结果见[验证策略](../development/validation.md)。

## 1. 总体结构

采用模块化单体与轻量宿主。每个数据目录只有一个 Rust 核心实例拥有数据库、同步任务和发件队列。当前桌面使用共享核心，后续 Web 宿主复用核心库；独立部署时不共享数据目录。

下图同时包含当前实现与目标结构；HTTP / Web、远程 MCP、内置 Agent、AI Run 和模型端点尚未实现。

```text
前端工作台（React + TypeScript + Vite + shadcn；账号设置、收信阅读、富文本写信与语言设置）
    └── MailxClient 接口
         ├── Tauri IPC adapter ── 桌面宿主
         └── HTTP + SSE adapter ─ Web 宿主（规划）
                                      │
外部 Agent ─ stdio ─ MCP bridge ─ 本地受控 IPC ─┐
远程 Agent ─ HTTPS Streamable HTTP（服务端） ────┤→ MCP 适配层
内置 Agent / 固定工作流（进程内） ────────────────┤
                                                  ↓
                         工具层：可见性 + 按入口策略 + 结果整形
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

MCP bridge 完成 token 握手后只转发 MCP 消息，不打开数据库或启动第二套同步器；本地 MCP 服务与工具层驻留桌面核心所在进程。后续服务端将复用同一份 MCP 实现，内置 AI 将在进程内直接调用工具层，不依赖 MCP 回环，也不实现通用 Agent 插件执行器。当前工具层、本地 MCP 适配与 bridge 已实现，内置 Agent 与远程传输仍为规划，见[工具层与调用入口](agent-tools.md)与 [ADR 0004](../adr/0004-shared-tool-layer.md)。

## 2. 技术选型

| 层 | 设计选择 | 实施约束 |
| --- | --- | --- |
| UI | React + TypeScript + Vite，已初始化 | 三栏工作台与设置弹窗；桌面通过 MailxClient 读取同步文件夹、各目录头部、统一列表与分页，普通浏览器预览使用 mock；HTTP 留待后续 |
| UI 基础 | shadcn `base-mira` preset，已初始化 | 账号业务界面使用 shadcn 组件组合，缺少组件时按现有 preset 安装 |
| 文本语言 | `i18next + react-i18next`；Rust 使用有类型的共享资源子集，已实现 | 根 `locales/{zh-CN,en}/` 随包提供，语言由设置快照统一派生；宿主注入系统语言，core 不读取平台 API。首版只适配文本，日期等区域格式化另行规划 |
| 桌面 | Tauri v2，已初始化 | 使用 `frontend/dist` / Vite `devUrl`；账号 IPC、user-data-dir 与系统凭据库装配已接通；正文经自定义协议与空 sandbox iframe 隔离；`tauri-plugin-single-instance` 使第二次启动只唤起已运行的窗口，`tauri-plugin-window-state` 记住主窗口的大小、位置与最大化 / 全屏（刻意不恢复窗口装饰与可见性，主窗口先隐藏创建、恢复后再显示）；打包版本去掉浏览器默认行为（右键菜单、刷新 / 打印 / 查找 / 检查器快捷键等），主界面靠前端拦截，Windows 另用 WebView2 设置关闭浏览器快捷键与表单记忆；邮件正文所在的沙箱 iframe 有意不处理 |
| 核心 | Rust，`MailxCore` 应用服务门面 | 账号存储为单连接加锁的短事务，宿主经阻塞线程调用；每个账号一个 tokio 后台同步任务，运行时由宿主注入；持锁期间不做网络 I/O |
| 接收 / 发送 | `async-imap` 用于同步与标记写回；`lettre` 构建发送 MIME，SMTP 会话在 core 实现 | IMAP 与 SMTP 连接统一经过账号路由；已有受控服务器测试，完整真实服务商组合及 OAuth 仍待验证，详见[写信与发送](sending.md) |
| MIME / HTML | `mail-parser` + `full_encoding` 与 `ammonia` 已采用 | 多字符集头部与 MIME 正文解码、HTML / CSS 净化、内嵌图片验证及 iframe + CSP 隔离已有实现 |
| 存储 | SQLite + `rusqlite`，已采用 | 单连接、WAL、FULL 同步、外键、迁移校验与目录锁；FTS5 trigram 索引头部、预览、已缓存正文、附件名和本机已发送副本，短词退回 LIKE；覆盖边界见[存储与搜索](storage-search.md) |
| AI | 规划采用 Rust 流式适配器 | OpenAI 兼容与 Ollama 为后续方案，尚未接入模型；不同协议不伪装成同一种接口 |
| MCP | [官方 Rust SDK `rmcp`](https://github.com/modelcontextprotocol/rust-sdk)（仅 `server` 功能，独立 MCP 适配 crate） | 本地 IPC、stdio bridge、桌面开关与邮件工具已有实现；能力与授权边界见 [MCP 契约](../contracts/mcp.md) |
| Web | 规划采用 Axum + 同源静态 SPA + SSE | 后续单用户宿主、服务端 Session、TOTP；当前 `server/` 仅占位 |
| 工具链 | Rust / Cargo、Bun | 工具链版本以根 `rust-toolchain.toml` 和 `package.json` 为准；`bun.lock`、`Cargo.lock` 锁定依赖 |

依赖版本以各包清单和锁文件为准。选型理由见 [ADR 0001](../adr/0001-shared-core-and-hosts.md)、[ADR 0002](../adr/0002-stack-and-storage.md)。协议库的官方接口文档用于 M0 接入验证：[async-imap](https://docs.rs/async-imap/latest/async_imap/)、[lettre](https://docs.rs/lettre/latest/lettre/transport/smtp/struct.AsyncSmtpTransport.html)。

## 3. 目标目录与依赖

关键目录直接放在仓库根目录。`core/`、`desktop/`、`mcp-server/` 与 `mcp-bridge/` 是现有 Cargo 包，`frontend/` 是 Bun 工作区中的前端包；`server/` 仍只占位，`.github/workflows/ci.yml` 已配置前端与三平台 Rust 检查。Cargo 包名表达当前工程依赖，不冻结对外程序、安装包或镜像名称。

```text
Cargo.toml                  # workspace：core + desktop + mcp-server + mcp-bridge
Cargo.lock                  # Rust 依赖解析结果
rust-toolchain.toml          # 固定 Rust 工具链、rustfmt、Clippy
core/                       # mailx-core，共享核心；业务与通用实现均在这里
  src/app.rs                # MailxCore：宿主共用的应用服务门面
  src/accounts/             # 账号服务、写入型认证输入与脱敏 DTO、同步凭证解析
  src/net/                  # 按账号路由建立直连 / SOCKS5 / HTTP CONNECT 与 TLS
  src/imap/                 # IMAP 会话与修改版 UTF-7
  src/mail/compatibility.rs # 主干消费的服务商兼容接口
  src/providers/            # 服务商识别、注册与具体兼容规则
  src/sync/                 # 文件夹 / 邮件头解析、同步计划与每账号后台任务
  src/storage/              # SQLite、迁移、文件夹 / 邮件头缓存与游标、凭证恢复和实例锁
  src/secrets.rs            # 宿主注入的 SecretStore
  src/settings.rs           # 应用偏好、语言兼容与设置快照版本
  src/localization.rs       # 通知与引用模板的纯文本生成
  src/tools/                # 共享工具层（MCP 与未来 Agent 共用）
mcp-server/                 # mailx-mcp-server（候选名），本地 MCP 服务端库：rmcp 适配、socket / 命名管道、握手、发现文件
mcp-bridge/                 # mailx-mcp-bridge，cargo 二进制 mailx-mcp-bridge，随包 sidecar 名 mailx-mcp（候选名）：stdio 到本地 IPC 的转发，不链接 core
desktop/                    # mailx-desktop，Tauri 宿主
  src/                      # 程序入口与 IPC 适配
  src/locale.rs             # 系统语言读取并注入共享核心
  src/mcp.rs                # 本地 MCP 服务的启停、状态事件与命令
  capabilities/             # 主窗口权限
  icons/                    # 正式桌面应用图标与托盘图标资源
server/                     # 第二阶段服务器宿主，当前占位
frontend/                   # React + TypeScript + Vite + shadcn 前端
  src/features/accounts/    # 账号管理、修改连接与添加账号弹窗，资料与连接表单
  src/features/mail/        # 三栏邮件工作台
  src/features/settings/    # 设置弹窗（通用语言、外观、通知、发信、安全性、关于）
  src/i18n/                 # 资源初始化、结构化错误及跨窗口语言同步
  src/lib/mailx-client.ts    # 传输适配
  src/generated/            # Rust DTO 生成的 TypeScript
  package.json              # 前端脚本与依赖
  vite.config.ts            # Vite / Tailwind 配置
locales/                    # zh-CN / en 公共文本资源；Rust 与前端共用
scripts/check-i18n.mjs      # 资源、复数、参数与 Rust 子集门禁
docs/                       # 文档
branding/                   # 主 Logo 源稿、品牌变体、分享素材与资源清单
.github/workflows/ci.yml     # 前端检查与 Linux / macOS / Windows Rust 检查
```

当前依赖为 **`desktop → core`**、**`desktop → mcp-server → core`**；`mcp-bridge` 不链接 core。第二阶段增加 **`server → core`** 并复用 MCP 适配层。不再单设 `infra` 包：存储、邮件协议、网络路由和后续模型适配随功能放入 `core` 的内部模块。宿主无关意味着 core 不导入 Tauri、Axum 或 UI API，可以依赖 SQLite 驱动、协议库和 HTTP 客户端；不要求为每个实现预建 trait。平台特定能力按需要由宿主注入。

邮件同步、收信、发信、重试与持久化均由 Rust core 承担，前端负责展示、输入和交互。桌面 IPC 与本地 MCP 工具共用核心应用服务；后续 HTTP 也应复用同一套服务。出现独立发布、可选重依赖或明确复用需求时，再评估拆包。

服务商差异通过 core 内的中立兼容接口接入，具体识别与偏差处理放在 `core/src/providers/`，不在主干同步、发送、存储流程中散布品牌分支。模块边界、已发送副本对账和扩展约束以[邮箱服务商兼容层](provider-compatibility.md)为准。

账号以 Rust DTO 为跨边界类型来源，通过 `ts-rs` 生成 TypeScript；输入秘密与读取结果分开。前端仅经 `MailxClient` 调用宿主，生成产物可检查漂移。接口仍处于开发期，未冻结对外兼容承诺。

首版中英文文本的语言来源、持久化兼容、快照修订号及窗口同步规则见[中英文文本适配](internationalization.md)，设置与事件契约见[应用 API](../contracts/app-api.md#应用偏好设置)。系统语言由 desktop 读取，core 只接收受支持语言并生成通知 / 引用文本；没有新增邮件数据库迁移或正文 iframe 权限。日期 / 时间、数字、文件大小、列表连接与排序保留现有行为；三平台打包体验与原生通知仍待分别验收。

## 4. 核心职责

当前 `MailxCore` / `AccountService` 已提供账号、邮件查询与整理、草稿和发送服务，`SyncManager` 管理每账号同步、IDLE 与写回任务；表中服务名描述职责，尚未单独建立的服务按实际功能拆分，不要求先创建空目录或接口。

| 模块 | 拥有的状态 / 责任 | 不承担 |
| --- | --- | --- |
| AccountService | 账号、端点、认证配置、路由、凭证引用与恢复；连接诊断由核心连接器完成 | 返回密钥明文 |
| SyncManager | 每账号任务、目录、邮件头游标、增量对账、IDLE 与标记 / 删除写回 | UI 排序和模型判断 |
| 邮件读取与搜索（当前在 MailxCore / AccountService / Storage） | 缓存读取、FTS5 / 短词 LIKE、稳定分页、按需正文缓存与本地归档 | 让前端执行 SQL |
| DraftService / OutboxService | 草稿版本、发送快照、幂等与结果状态 | 绕过宿主授权；自动重发 unknown 结果 |
| AiService（规划） | 选定上下文、运行、流式结果、取消 | 直接持有邮件协议连接 |
| 工具层与 RequestContext | 宿主授予的账号范围、动作权限、按入口可见性、结果整形与审计 | 依赖前端按钮隐藏来授权 |
| Storage / BlobStore | 持久化、迁移与缓存；备份恢复工具仍为规划 | 网络调用跨越事务 |

桌面 IPC 按主窗口与写信窗口的能力分别开放，邮件正文 iframe 没有 IPC 权限；参数不接受客户端自报主体或秘密引用。本地 MCP 由宿主构造 `RequestContext`，包含调用主体、授权账号集合、权限集合和请求标识。客户端不能自报自己是 UI 或管理员。当前 MCP 总开关授予统一能力，逐客户端配对与独立范围尚未实现。

## 5. 关键数据流

当前账号配置链路为：shadcn 表单 → MailxClient → 受限 Tauri IPC → AccountService → SecretStore 与 SQLite 更新恢复。保存配置的短事务不连接邮箱，成功后宿主启动或重启该账号的后台同步任务；完整模型与操作边界见[账号配置](account-configuration.md)。下列同步、阅读、发送和外部 Agent 链路已有实现，内置 AI 链路仍为规划。

1. **同步（当前）**：账号配置 → 路由与认证 → `STATUS` / `EXAMINE` / `UID SEARCH` → 每批最多 200 封头部 → 短事务写入与游标推进 → 逐目录数据版本事件 → UI 重新查询。账号之间并发且暂不设全局上限，账号内顺序对账所有可选文件夹；前台轮询每 20 秒一轮，IDLE 在线时改为 3 分钟，后台为 3–10 分钟，目录每 10 分钟到期刷新。独立 IDLE 连接监视收件箱并唤醒同步；头部同步不下载正文。
2. **阅读（当前）**：缓存查询 → 缺失时按账号路由经独立连接 BODY.PEEK 下载 → 有界 MIME 解码与 HTML 净化 → 本地缓存 → 空 sandbox 隔离渲染。远程图片按安全性设置询问 / 自动加载 / 永不加载，经 Rust 识别实际格式并验证，在本次阅读内存与按账号、来源及连接版本隔离的磁盘缓存中复用；失败有固定原因和手动重试。自定义代理可显式启用 HTTPS 图片域名解析，具体限制见[网络与安全](network-security.md)。点击正文外链由 Rust 交给系统打开；附件可经系统另存为对话框保存。
3. **发送**：写信窗口检查完整收件人与附件并两次点击发送，或授权的 MCP / Agent 调用发送工具 → 核心校验账号范围、权限、草稿版本和幂等键 → 固化版本与 MIME → 单个队列项 → SMTP → 记录 accepted / failed / unknown。网络不确定时不自动重发。细节见[写信与发送设计](sending.md)。
4. **AI（规划）**：用户选定内容与模型 → 披露策略检查 → 工具层（内置 Agent 可见的工具） → 后端请求模型 → 流式展示 → 保存建议或新草稿。内置 AI 的交互与发送策略在接入时单独验证。
5. **外部 Agent**：用户开启 MCP → token 握手 → 统一授权（邮件整理、草稿与本机文件附件管理、直接发送、发件箱查询与取消） → MCP 适配层 → 工具层（显式对外可见的工具） → 应用权限 → 与 UI 相同服务 → 返回限量文本和元数据，不返回密钥和宿主路径；不能删除邮件。

详细状态与恢复语义分别见[同步](mail-sync.md)、[应用契约](../contracts/app-api.md)、[AI Runtime](../ai-runtime/README.md)。

## 6. 验证边界

自动测试、受控协议探针和实际安装包验证承担不同职责。公开支持的平台与服务商组合，应有收读发、网络路由、正文隔离、MCP、升级与恢复等对应证据；功能实现不能替代这些验证。OAuth、内置 AI 与 Web 在接入时分别补齐其授权、路由和生命周期验证。检查项见[路线图](../roadmap/README.md)，执行结果统一记录在[验证策略](../development/validation.md)。
