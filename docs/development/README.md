# 开发指南

状态：**桌面工程骨架已建立，邮件功能待实现**。当前可以启动 Vite 前端、启动前端 + Tauri 桌面开发环境并构建两者；数据库、协议接入与服务器端仍待后续设计。

## 1. 环境与目录

- 安装 Git 与通过 rustup 管理的 Rust。仓库 `rust-toolchain.toml` 指定 1.94.0、rustfmt 和 Clippy，首次运行 Cargo 时 rustup 会按需安装。
- 安装 [Bun](https://bun.sh/)。根目录 `package.json` 记录当前包管理器版本，前端依赖统一从仓库根目录安装。
- 构建桌面端需对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)：Windows 的 MSVC 构建工具与 WebView2、macOS 的 Xcode 工具链、Linux 的 WebKitGTK 等。
- 仅开发 `core/` 不需要 Tauri、WebView 或系统 GUI 依赖；桌面开发需要 Tauri 前置依赖。Tauri CLI 作为 `frontend/` 工作区的开发依赖安装，由根目录脚本统一调用。

根 Cargo workspace 包含 `core/` 和 `desktop/`；前端工作区位于 `frontend/`，公共 Rust 包元数据与依赖版本在根 `Cargo.toml` 管理，`Cargo.lock` 跟踪 Rust 解析结果，`bun.lock` 跟踪前端解析结果。Cargo 包暂不发布到 crates.io；`0.1.0` 是工程初始版本，不表示已发布 MVP。

## 2. 已有开发命令

以下命令均在仓库根目录执行：

```sh
# 安装前端依赖；只使用根目录 bun.lock
bun install

# 前端开发服务器
bun run dev

# 前端 + Tauri 桌面开发环境（Bun shorthand）
bun tauri dev

# 前端静态资源构建
bun run build

# 桌面 release 构建；当前 bundle.active=false，不生成安装包
bun tauri build

# 版本入口校验：根 package.json、frontend/package.json、Cargo workspace
bun run version:check

# 独立检查核心，不构建桌面 GUI
cargo check -p mailx-core --locked

# Rust 检查
bun run rust:check
bun run rust:fmt
bun run rust:clippy
bun run rust:test

# 前端检查
bun run typecheck
bun run lint

# 一次执行版本、前端类型 / lint 和 Rust check
bun run check

# 变更检查
git diff --check
```

`bun run dev`（也可写作 `bun dev`）只启动 Vite；`bun tauri dev`（等价于 `bun run tauri dev`）由根脚本调用 Tauri CLI，并通过 `desktop/tauri.conf.json` 启动 Vite 和桌面宿主。`bun run build` 只生成 `frontend/dist`；`bun tauri build` 会先构建该目录，再编译桌面 release 二进制。由于 Tauri 在编译时检查 `frontendDist`，单独执行 `cargo check --workspace` 前也需要先执行 `bun run build`；推荐使用会自动准备前端资源的 `bun run check`。当前 `bundle.active=false`，所以不宣称已经生成安装包。

当前前端仍是初始化模板，尚未实现邮件列表、账号、同步或发信。桌面启动后，Tauri 宿主仍可通过 `core_info` 验证页面 → IPC → core 的连接；该调用不检查数据库、账号或同步状态。代码尚无业务测试；`bun run rust:test` 当前验证测试目标可编译运行，不代表邮件行为已有覆盖。

正常 Tauri 开发和构建使用 `frontend/dist`，不再在 `desktop/` 内维护另一份前端资源。Tauri 生成的 schema 与命令权限文件忽略提交，由 `build.rs` 重新生成。`git diff --check` 不覆盖未跟踪文件，新文件需额外检查。

## 3. 包边界

- **core**：共享业务与通用实现一起管理，之后按功能逐步加入账号、同步、发件队列、存储、路由、搜索等模块。可直接依赖协议和数据库库，不依赖 Tauri、Axum 或 UI。
- **desktop**：Tauri 生命周期、窗口、IPC 和平台集成。IPC 负责适配核心调用，不承载邮件业务。
- **frontend**：React + TypeScript + Vite + shadcn（Mira preset）工程骨架，依赖 React Router；当前只包含初始化模板，后续通过统一客户端适配器调用 core 能力。
- **server**：当前仅顶级目录占位，第二阶段服务器 MVP 复用 core。

不预建空 trait、Repository 层或每种技术的独立 crate。需要多个实现、平台适配或故障注入时再引入明确接口。

## 4. 接下来的桌面 MVP 工作

1. 在现有前端骨架上确定 UI 客户端适配和 DTO 生成方案，接入真实工作台。
2. 选择异步执行方案，编写最小 IMAP / SMTP 探针，验证 Direct / SOCKS5 / HTTP CONNECT 下的 TLS、认证与收发，并证明代理失败不会直连。
3. 在 SQLite 方向下比较 SQLx / rusqlite，验证 WAL、FTS5、迁移和执行模型；建立平台数据目录、实例锁与凭据存储。
4. 完成“配置测试邮箱 → 获取最近 5 封头部 → 去重入库 → 重启读取 → 展示缓存正文”的纵向链路，再增加第二个不同路由账号。
5. 随功能补齐故障测试和真实 CI；第一阶段先完成桌面 MVP，第二阶段再初始化 server 与容器部署。

## 5. 增加邮箱预设

候选格式包含 providerId、匹配域名、IMAP / SMTP 主机端口、TLS 模式、认证方法、用户名模板、开通说明、官方来源、核验日期和已验证能力。没有来源或测试证据时标记 experimental。

读取可信内置预设后允许用户查看、修改并测试，不能仅凭邮箱域名自动连接任意远程发现结果。测试覆盖收信与发信两个方向、失败提示和网络路由。连接配置、OAuth 注册信息和账号授权在不同层管理，真实凭证不得放进资源文件。

详细验收方法见[验证策略](validation.md)，任务依赖见[路线图](../roadmap/README.md)。设计、命名和选型均可根据后续验证继续调整。
