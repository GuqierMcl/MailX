# ADR 0002：工程基础与前端、存储选型

状态：**部分 Accepted / 部分 Proposed** · 2026-10-16。Rust / Cargo、Tauri、shadcn、rusqlite 和 ts-rs 已采用；协议、全文搜索与邮件任务执行模型继续验证。

## 背景

蓝图保留了 Svelte / React 两种可能；参考项目 NexusPilot 已使用 React、TypeScript、Bun 和 Tauri。MailX 同时需要可恢复邮件缓存、草稿、队列与离线查询，早期不宜引入外部数据库服务。

## 当前建议

已采用 Rust 2024 edition、Rust / Cargo 1.94.0 与 Tauri v2，添加 `rust-toolchain.toml` 和 `Cargo.lock`；工具链版本是当前构建基线，不承诺永久 MSRV。两个 Cargo 包统一继承版本、许可证和仓库地址，暂不发布到 crates.io。

前端当前采用 React + TypeScript + Vite，组件基础采用 shadcn `base-mira` preset，路由依赖采用 React Router；这些是可随实现验证调整的工程起点，不是产品功能完成的承诺。根目录使用 Bun 1.3.12 工作区，Tauri CLI 作为前端开发依赖安装，根脚本统一提供前端、桌面和检查命令。当前账号导航、列表和配置表单使用 shadcn 组件组合，缺少组件时按现有 preset 安装；完整邮件工作台尚未实现。前端通过 MailxClient 访问 Tauri，跨边界类型由 Rust DTO 使用 ts-rs 生成并检查漂移。

账号配置采用 SQLite + `rusqlite`，以单连接同步服务提供简单的事务边界；宿主在阻塞工作线程串行调用，避免阻塞 UI。数据直接存 user-data-dir，独占锁限制一个写入核心，启用 WAL、外键、FULL 同步和迁移校验。凭证由宿主 SecretStore 提供，更新 journal 协调两种存储的恢复。选择该组合便于用临时数据库和故障替身验证一致性；当前不为尚不存在的同步负载引入读连接池。FTS5、中文查询和后台协议执行模型仍待验证。草稿与队列是需保护的本地源数据，可重建缓存与索引区别处理；附件与大对象使用 blob 文件仍为候选设计。

初版拟覆盖 IMAP / SMTP；POP3、JMAP 与服务商 API 保留扩展空间。候选库 `async-imap` / `lettre` 需通过统一路由验证，尤其 SMTP 自定义连接、TLS 和 OAuth。当前没有协议依赖；验证失败时可以换库，不用绕过代理来保住选型名称。

## 代价与重评条件

数据库执行模型需要明确任务边界；WAL 仍不能消除单写入限制；全文中文短词需要专门体验评估。前端仍需验证虚拟列表、正文隔离与启动成本，不能把性能交给框架名称或初始化 preset。

实现第一条收信链路时，再根据并发、维护成本、搜索质量和开发体验补齐选型。已积累用户数据后更换存储须设计迁移；当前已有账号 schema 与凭证引用，后续变更须通过迁移和恢复验证，不能删除重建用户配置。详见[存储设计](../architecture/storage-search.md)。
