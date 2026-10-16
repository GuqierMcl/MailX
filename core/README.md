# core

MailX 的共享 Rust 核心库，当前 Cargo 包名为 `mailx-core`。桌面端先使用它，第二阶段的服务器端复用同一核心。

本包合并原先设想的 core / infra 职责。账号、同步、发送、存储、搜索、网络路由与模型适配在功能落地时按模块加入；暂不预建空模块、空 trait 或独立适配层。SQLite、协议与 HTTP 库可以直接成为本包依赖，Tauri、Axum 和窗口 API 不可以。

当前提供 `AccountService` 的账号创建、列表、详情与修改，以及 `VERSION` 编译信息。账号配置以 `rusqlite` 持久化在宿主提供的用户数据目录，启用外键、WAL、FULL 同步、迁移校验与实例锁。凭证通过宿主注入的 `SecretStore` 保存，不能从应用读取接口取得凭证值或内部引用。`SecretValue` 的 Debug 脱敏并在释放时清理内容。

服务采用同步接口，宿主应在阻塞工作线程执行。`AccountService::open(path, secrets)` 在启动时恢复未完成的凭证更新；保留的非敏感 journal 协调 SQLite 与系统凭据库，不能将它们视作同一事务。账号 revision 与凭证 generation 独立；端点变更保留原信任范围，显示 `needsConfirmation`；收信来源变化创建新的 `mailboxSourceId`。OAuth 只有预留类型，配置请求返回 `unsupported`。没有协议连接、同步或发送功能。

核心可独立验证，不需要桌面 WebView 环境：

```sh
cargo check -p mailx-core --locked
cargo test -p mailx-core --locked
cargo clippy -p mailx-core --all-targets --locked -- -D warnings
```

根目录 `bun run types:generate` 调用 `export_types` 从 Rust DTO 生成 TypeScript，`bun run types:check` 检查产物漂移；请求类型与只读结果类型分开，秘密输入类型不实现 Rust 序列化。创建请求以 `requestId` 去重，账号仍处于首个 revision 且参数和系统凭据比对一致时返回原账号；编辑后的历史创建请求返回冲突。读取会向凭据库确认凭证是否存在，暂不可用返回可识别错误。

测试使用隔离临时目录与显式故障替身，覆盖跨账号关系、持久化、版本冲突、路由、端点信任、实例锁及凭证/数据库提交恢复。系统凭据库和桌面 IPC 另由宿主验证。路径、平台凭据实现与宿主配置由宿主提供；共享业务逻辑不能移入桌面 IPC 命令。详见[账号配置与凭证生命周期](../docs/architecture/account-configuration.md)及[架构总览](../docs/architecture/overview.md)。
