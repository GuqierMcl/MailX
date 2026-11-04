# ADR 0002：工程基础与前端、存储选型

状态：**基础选型已采用，后续能力继续演进**。本记录始于 2026-10-16；下文保留初始选择的背景与重评条件，并说明已经落地的选型。

## 已采用的选型

Rust / Cargo、Tauri、React + Vite + shadcn、rusqlite 与 ts-rs 已采用。桌面邮件工作台、IMAP 同步 / IDLE、手写 SMTP 会话与发送队列已接通；全文搜索采用 SQLite FTS5 trigram，短词退回 LIKE；正文、草稿附件和待发送 / 已发送快照使用内容寻址 blob。前端已有虚拟列表与隔离正文阅读。Cargo workspace 现包含 `core`、`desktop`、`mcp-server`、`mcp-bridge`，工具层与本地 MCP 已落地，OAuth、内置 AI 与服务器宿主仍规划。

这些是实现状态，不代表完整服务商、性能或三平台验收通过；具体边界见[存储与搜索](../architecture/storage-search.md)、[同步与发信](../architecture/mail-sync.md)和[验证记录](../development/validation.md)。

## 背景

蓝图保留了 Svelte / React 两种可能；参考项目 NexusPilot 已使用 React、TypeScript、Bun 和 Tauri。MailX 同时需要可恢复邮件缓存、草稿、队列与离线查询，早期不宜引入外部数据库服务。

## 初始建议与后续调整

已采用 Rust 2024 edition 与 Tauri v2，以 `rust-toolchain.toml` 固定构建工具链、`Cargo.lock` 固定依赖解析；具体版本以这些配置为准，不承诺永久 MSRV。最初两个、现已扩展为四个 Cargo 包统一继承版本、许可证和仓库地址，暂不发布到 crates.io。

前端采用 React + TypeScript + Vite，组件基础采用 shadcn `base-mira` preset，路由依赖采用 React Router；这些是可随实现验证调整的工程起点，不是产品功能完成的承诺。根目录使用 Bun 工作区，包管理器版本以 `package.json` 为准；Tauri CLI 作为前端开发依赖安装，根脚本统一提供前端、桌面和检查命令。初始界面只含账号导航、列表和配置表单，之后已扩展为邮件工作台；外壳使用自有组件，其余界面按现有 shadcn 配置组合。前端通过 MailxClient 访问 Tauri，跨边界类型由 Rust DTO 使用 ts-rs 生成并检查漂移。

账号配置采用 SQLite + `rusqlite`，以单连接同步服务提供简单的事务边界；宿主在阻塞工作线程调用同步存储，避免阻塞 UI。数据直接存用户数据目录，独占锁限制一个写入核心，启用 WAL、外键、FULL 同步和迁移校验。凭证由宿主 SecretStore 提供，更新 journal 协调两种存储的恢复。选择该组合便于用临时数据库和故障替身验证一致性；初始阶段暂不引入读连接池。FTS5、中文查询和后台协议执行模型随后已有实现与针对性测试。草稿与队列是需保护的本地源数据，可重建缓存与索引区别处理；附件与大对象的 blob 方案也已落地，当前执行边界以存储与同步设计为准。

初版拟覆盖 IMAP / SMTP；POP3、JMAP 与服务商 API 保留扩展空间。候选库 `async-imap` / `lettre` 需通过统一路由验证，尤其 SMTP 自定义连接、TLS 和 OAuth。`async-imap` 已用于同步；`lettre` 已通过 SMTP 自定义连接与 TLS 的路由探针（2026-10-23，见[写信与发送设计](../architecture/sending.md#4-smtp-会话)），2026-10-23 起 `lettre` 是 `mailx-core` 的普通依赖，但 `default-features = false` 且只启用 `builder`（MIME 构造与地址解析）；SMTP 会话改为手写，经 `net::connect` / `net::upgrade_tls`，不启用 lettre 的传输与 TLS 特性，依赖树中没有 `native-tls` / `openssl` / `aws-lc`，原因见[写信与发送设计](../architecture/sending.md#4-smtp-会话)。OAuth 仍待验证。验证失败时可以换库，不用绕过代理来保住选型名称。

## 代价与重评条件

数据库执行模型需要明确任务边界；WAL 仍不能消除单写入限制；全文中文短词需要专门体验评估。前端仍需持续验证已有虚拟列表、正文隔离与启动成本，不能把性能交给框架名称或初始化 preset。

收信链路已落地，后续根据并发、维护成本、搜索质量和开发体验重评选型。已有用户数据，更换存储须设计迁移；schema 与凭证引用的后续变更须通过迁移和恢复验证，不能删除重建用户配置。详见[存储设计](../architecture/storage-search.md)。
