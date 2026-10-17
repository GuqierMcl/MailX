# 开发指南

状态：**账号配置基础已接通，邮件功能待实现**。当前可启动 shadcn 邮件工作台外壳与 Tauri 宿主，在设置弹窗中创建、查看、修改并重载账号配置；普通浏览器运行 `bun run dev` 时工作台使用内存 mock 数据，桌面应用中尚无邮件命令；非敏感数据存 user-data-dir 的 SQLite，凭证由系统凭据库保存。协议接入、OAuth 与服务器仍未实现。

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

# 一次执行版本、生成类型漂移、前端类型 / lint / 测试 / build 和 Rust check
bun run check

# 前端账号组件与客户端测试
bun run frontend:test

# 核心账号及存储测试
cargo test -p mailx-core --locked

# 生成 Rust DTO 对应的 TypeScript，并检查提交产物漂移
bun run types:generate
bun run types:check

# 变更检查
git diff --check
```

`bun run dev`（也可写作 `bun dev`）只启动 Vite；`bun tauri dev`（等价于 `bun run tauri dev`）由根脚本调用 Tauri CLI，并通过 `desktop/tauri.conf.json` 启动 Vite 和桌面宿主。`bun run build` 只生成 `frontend/dist`；`bun tauri build` 会先构建该目录，再编译桌面 release 二进制。由于 Tauri 在编译时检查 `frontendDist`，单独执行 `cargo check --workspace` 前也需要先执行 `bun run build`；推荐使用会自动准备前端资源的 `bun run check`。当前 `bundle.active=false`，所以不宣称已经生成安装包。

当前前端先通过 `MailxClient → app_get_startup_status` 等启动 IPC 等待宿主 Ready，再进入账号导航、列表与设置详情，经 `accounts_* IPC → AccountService` 完成读写。页面显示“尚未验证连接”，保存不访问邮箱服务器。普通浏览器的 `bun run dev` 没有系统凭据及核心能力，会显示需要桌面应用；不能把它作为已保存配置的替代后端。

桌面数据目录直接来自 Tauri `app_data_dir()`，包含 `mailx.sqlite3` 与 `mailx.lock`，不写入仓库或当前工作目录。应用标识变化会影响目录归属，已有数据需要显式迁移。测试通过宿主/核心显式注入隔离临时目录；不要用真实邮箱或秘密做公开测试样本。

正常 Tauri 开发和构建使用 `frontend/dist`，不再在 `desktop/` 内维护另一份前端资源。Tauri 生成的 schema 与命令权限文件忽略提交，由 `build.rs` 重新生成。`git diff --check` 不覆盖未跟踪文件，新文件需额外检查。

## 3. 包边界

Logo 的唯一设计源文件为 `branding/logo.svg`。更新后执行 `bun run icons:generate`，会同步品牌素材、桌面图标和网页图标；`bun run icons:check` 验证派生资源与源稿一致。该工具只在设计资源变更时运行，无需加入每次应用构建。详见[品牌资源说明](../../branding/README.md)。

- **core**：已实现账号、认证与路由配置、SQLite 存储及凭证更新恢复；之后按功能加入同步、发件队列、连接器和搜索。可直接依赖协议和数据库库，不依赖 Tauri、Axum 或 UI。
- **desktop**：Tauri 生命周期、窗口、受限 IPC、user-data-dir 和系统凭据库装配，在阻塞线程串行调用同步核心。IPC 负责适配核心调用，不承载邮件业务。
- **frontend**：React + TypeScript + Vite + shadcn（Mira preset）；账号业务界面由 shadcn 组件组成，缺少组件时按现有配置安装。MailxClient 隔离传输，Rust DTO 生成 TypeScript，表单状态与应用契约分别维护。
- **server**：当前仅顶级目录占位，第二阶段服务器 MVP 复用 core。

不预建空 trait、Repository 层或每种技术的独立 crate。需要多个实现、平台适配或故障注入时再引入明确接口。

## 4. 接下来的桌面 MVP 工作

1. 在已有账号配置基础上编写 IMAP / SMTP 探针，确定协议异步执行方案，验证 Direct / SOCKS5 / HTTP CONNECT 下的 TLS、认证与收发，并证明代理失败不会直连。
2. 实现连接测试及状态，把密码/授权码配置接入真实连接器；按可用资格推进 OAuth 注册、主体核验与授权流程。
3. 在现有 SQLite 迁移与恢复基础上加入邮件来源身份、批次事务和 FTS5 探针，验证搜索与同步负载下的执行模型。
4. 完成“测试邮箱 → 最近 5 封头部 → 去重入库 → 重启读取 → 展示缓存正文”链路，再增加第二个不同路由账号。
5. 随功能补齐故障测试与 CI；先桌面 MVP，再初始化 server 和容器部署。现有账号配置的自动化测试不代表邮件或全部平台已验证。

## 5. 增加邮箱预设

候选格式包含 providerId、匹配域名、IMAP / SMTP 主机端口、TLS 模式、认证方法、用户名模板、开通说明、官方来源、核验日期和已验证能力。没有来源或测试证据时标记 experimental。

读取可信内置预设后允许用户查看、修改并测试，不能仅凭邮箱域名自动连接任意远程发现结果。测试覆盖收信与发信两个方向、失败提示和网络路由。连接配置、OAuth 注册信息和账号授权在不同层管理，真实凭证不得放进资源文件。

详细验收方法见[验证策略](validation.md)，任务依赖见[路线图](../roadmap/README.md)。设计、命名和选型均可根据后续验证继续调整。
