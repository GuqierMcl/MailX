# desktop

MailX 的 Tauri v2 桌面宿主，当前 Cargo 包名为 `mailx-desktop`，依赖 `mailx-core` 和 `mailx-mcp-server`。它负责窗口、受限 IPC、user-data-dir、系统凭据库、通知、后台运行、应用更新与本地 MCP 生命周期；账号及邮件业务放在 `core/`。

在仓库根目录执行：

```sh
bun install --frozen-lockfile
bun run tauri dev
```

需要 Bun、Rust 和对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。Tauri CLI 作为 `frontend/` 的开发依赖安装，根目录脚本负责把嵌套的配置文件、前端开发服务器和 CLI 连接起来。前端生产资源位于 `frontend/dist`，普通 Rust 构建仍可用于只检查 Rust 代码，但桌面开发以 `bun run tauri dev` 为准。

## 当前范围

- `src/main.rs`：Tauri 程序入口与命令注册，经 `src/storage_location.rs` 取得 user-data-dir，并装配系统凭据库。
- `src/storage_location.rs`：数据目录与凭据 service 以产品名 `MailX` 命名，不再由应用 identifier 推导；环境变量 `MAILX_DATA_DIR`（绝对路径）可改用独立目录，凭据 service 随之变为 `MailX.<目录名>`；目录位于 Tauri `app_data_dir()` 的同级（macOS 的 `~/Library/Application Support/MailX`，Windows 的 `%APPDATA%\MailX`，Linux 的 `~/.local/share/MailX`）。
- `src/commands.rs`：启动状态与显式重试、账号与连接验证、同步、邮件列表与正文、标记 / 删除 / 本机归档、草稿与发送队列、设置等 IPC 适配。邮件业务在核心 Ready 前拒绝访问；命令核对窗口权限，返回结构化结果与脱敏错误，避免解析异常回显秘密。
- `src/updater/`：应用内更新的宿主侧。`policy.rs` 是不依赖 Tauri 的状态机、版本比较、`latest.json` 解析与检查日程；`mod.rs` 负责经 `tauri-plugin-updater` 检查、下载（可取消）、验签安装，以及退出时安装与重启。它不走 `core` 的账号路由，也不读账号代理。设计见[应用内更新](../docs/architecture/updater.md)。
- `src/state.rs`：宿主启动时在阻塞线程主动打开核心、执行迁移并恢复凭证 journal，使用 `Initializing` / `Ready` / `Failed` 状态和单飞重试，Ready 后启动后台任务。数据目录内为 `mailx.sqlite3`、`mailx.lock` 和邮件 blob 等缓存，不使用仓库目录。
- `capabilities/main.json` / `composer.json`：分别为本地主窗口与写信窗口开放所需命令；没有通用文件系统、Shell、网络插件、`updater:*` 插件权限或远程来源授权。`build.rs` 生成命令权限，宿主同时校验调用窗口标签及来源 URL；写信窗口不能继承主窗口的账号管理或 MCP 开关权限。
- `src/secret_store.rs`：使用 keyring 3.6 的原生后端，凭据 service 固定为 `MailX`（与 identifier 无关）；macOS 上经 `src/secret_vault.rs` 把所有密码合并为一个钥匙串条目，使启动时最多一次授权，旧的逐账号条目在首次读取时迁入、下次启动清理（设计见[凭据保险库](../docs/architecture/secret-vault.md)）；macOS 使用 apple-native，Windows 使用 windows-native，Linux 使用 sync-secret-service + crypto-rust。其他平台明确不支持，不使用 mock 或明文 fallback。适配代码存在不代表该平台已完成运行验证。
- `src/body_protocol.rs`：按需正文与图片的隔离协议；`src/attachment_save.rs` 与 `compose_files.rs` 负责系统文件对话框及写信附件桥接。
- `src/compose.rs`：独立写信窗口、草稿绑定与窗口权限；`lifecycle.rs`、`tray.rs` / `tray_support.rs` 负责后台运行、托盘恢复和等待草稿 / 发送的安全退出；`autostart.rs` 负责用户主动开启的登录启动，静默偏好仅影响登录自动启动。
- `src/notifications/`：新邮件原生通知、提示音和点击打开；`src/mcp.rs` / `mcp_sidecar.rs`：本地 MCP 启停、状态事件、客户端配置和随应用打包的 `mailx-mcp` bridge。
- 根目录 `frontend/`：React + TypeScript + Vite + shadcn 邮件工作台、设置与富文本写信界面，经 `MailxClient` 调用宿主。
- `icons/`：从 `branding/logo.svg` 生成的正式应用图标，包含 Windows ICO、macOS ICNS、Linux PNG 和 Windows 商店尺寸；`tray/` 提供当前托盘使用的黑白与 macOS template 资源。

邮件正文视为不可信输入，经净化、自定义协议、正文 CSP 与空 sandbox iframe 隔离，不获得 Tauri IPC 权限。主窗口 CSP 只允许所需的本地正文来源，远程图片由后端按账号路由和安全策略获取；正文外链经受限入口交给系统打开。隔离与平台证据见[网络与安全](../docs/architecture/network-security.md)和[验证记录](../docs/development/validation.md)。[Tauri capability 说明](https://v2.tauri.app/security/capabilities/)。

## 前端与 Tauri 配置

`desktop/tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 分别调用根目录工作区中的前端脚本，`devUrl` 指向 Vite 的 `5173` 端口，`frontendDist` 指向 `../frontend/dist`。`scripts/tauri.mjs` 让根目录命令在配置文件位于 `desktop/` 子目录时仍能正确调用 Tauri CLI。账号界面经统一 MailxClient 调用 IPC，领域类型由 Rust 生成；契约仍可随实现演化。

应用标识为 `com.mailxapp.desktop`（对应域名 `mailxapp.com`）；Cargo 包名、命令行别名和镜像名称尚未作为正式命名冻结。`bundle.active` 为 `true`，`bundle.createUpdaterArtifacts` 让构建额外产出可自更新的 `.app.tar.gz` / NSIS / `.AppImage` 与对应 `.sig`，发版由 `.github/workflows/release.yml` 生成未签名的安装包和 `latest.json`，流程见[发版流程](../docs/development/release.md)。`plugins.updater` 的内置公钥只能验证对应私钥签出的包：没有私钥时，`bun run tauri build` 会在生成 `.app.tar.gz` 之后因缺少 `TAURI_SIGNING_PRIVATE_KEY` 以退出码 1 结束；本地构建可加 `--config '{"bundle":{"createUpdaterArtifacts":false}}'`。macOS 的 `bundle.targets` 因此包含 `app`（否则不会生成更新包）。用户数据目录和系统凭据 service 都不依赖 identifier，所以以后调整标识不会让已有账号和邮件缓存消失；但 Tauri 自己按 identifier 命名的 WebView 存储、缓存和窗口位置文件会随标识变化而重建。

更新主 Logo 后，在仓库根目录运行 `bun run icons:generate`，再用 `bun run icons:check` 验证生成文件。不要手工修改 `icons/` 下的派生资源。完整目录与使用方式见[品牌资源说明](../branding/README.md)。

账号模型、凭证更新恢复及未实现功能边界见[账号配置](../docs/architecture/account-configuration.md)。实际系统凭据库、平台构建与 IPC 验证分别记录，不能把替身测试当作跨平台支持证据。

真实系统凭据库的合成条目测试默认忽略，需要在已解锁的桌面环境显式运行：

```sh
cargo test -p mailx-desktop --locked native_secret_store_roundtrip -- --ignored
```

该测试使用独立 service，检查读写和幂等删除后清理条目；执行结果单独记录，普通测试通过不包含此项。
