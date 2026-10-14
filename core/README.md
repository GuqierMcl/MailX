# core

MailX 的共享 Rust 核心库，当前 Cargo 包名为 `mailx-core`。桌面端先使用它，第二阶段的服务器端复用同一核心。

本包合并原先设想的 core / infra 职责。账号、同步、发送、存储、搜索、网络路由与模型适配在功能落地时按模块加入；暂不预建空模块、空 trait 或独立适配层。SQLite、协议与 HTTP 库可以直接成为本包依赖，Tauri、Axum 和窗口 API 不可以。

当前只提供 `VERSION` 编译信息，没有邮件服务、数据库、后台任务或网络连接，也没有第三方依赖。可单独检查，不需要桌面 WebView 环境：

```sh
cargo check -p mailx-core --locked
```

路径、平台凭据实现与宿主配置在需要时由宿主提供；共享业务逻辑不能移入桌面 IPC 命令。详见[架构总览](../docs/architecture/overview.md)。
