# desktop

MailX 的 Tauri v2 桌面宿主，当前 Cargo 包名为 `mailx-desktop`，依赖 `mailx-core`。它负责窗口、受限 IPC、user-data-dir 与系统凭据库装配，账号及邮件业务放在 `core/`。

在仓库根目录执行：

```sh
bun install
bun run tauri dev
```

需要 Bun、Rust 和对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。Tauri CLI 作为 `frontend/` 的开发依赖安装，根目录脚本负责把嵌套的配置文件、前端开发服务器和 CLI 连接起来。前端生产资源位于 `frontend/dist`，普通 Rust 构建仍可用于只检查 Rust 代码，但桌面开发以 `bun run tauri dev` 为准。

## 当前范围

- `src/main.rs`：Tauri 程序入口与命令注册，经 `src/storage_location.rs` 取得 user-data-dir，并装配系统凭据库。
- `src/storage_location.rs`：数据目录与凭据 service 以产品名 `MailX` 命名，不再由应用 identifier 推导；环境变量 `MAILX_DATA_DIR`（绝对路径）可改用独立目录，凭据 service 随之变为 `MailX.<目录名>`；目录位于 Tauri `app_data_dir()` 的同级（macOS 的 `~/Library/Application Support/MailX`，Windows 的 `%APPDATA%\MailX`，Linux 的 `~/.local/share/MailX`）。
- `src/commands.rs`：`app_get_startup_status` / `app_retry_startup` 提供启动状态与显式重试；四个 `accounts_*` 命令在 Ready 前立即拒绝，之后适配 AccountService，均返回生成的 `ApiResult<T>`。命令自行解析原始负载，仅返回固定脱敏错误，避免解析异常回显秘密。原骨架的 `core_info` 已移除。
- `src/updater/`：应用内更新的宿主侧。`policy.rs` 是不依赖 Tauri 的状态机、版本比较、`latest.json` 解析与检查日程；`mod.rs` 负责经 `tauri-plugin-updater` 检查、下载（可取消）、验签安装，以及退出时安装与重启。它不走 `core` 的账号路由，也不读账号代理。设计见[应用内更新](../docs/architecture/updater.md)。
- `src/state.rs`：宿主启动时在阻塞线程主动打开核心并恢复凭证 journal，使用 `Initializing` / `Ready` / `Failed` 状态和单飞重试；业务服务只在 Ready 后串行运行。数据目录内为 `mailx.sqlite3` 和 `mailx.lock`，不使用仓库目录。
- `capabilities/main.json`：仅为本地 `main` 窗口开放启动状态/重试、账号管理命令和更新命令，没有文件系统、Shell、网络插件、`updater:*` 插件权限或远程来源授权。`build.rs` 为这些命令生成权限定义；宿主同时校验调用窗口标签及来源 URL。
- `src/secret_store.rs`：使用 keyring 3.6 的原生后端，凭据 service 固定为 `MailX`（与 identifier 无关）；macOS 上经 `src/secret_vault.rs` 把所有密码合并为一个钥匙串条目，使启动时最多一次授权，旧的逐账号条目在首次读取时迁入、下次启动清理（设计见[凭据保险库](../docs/architecture/secret-vault.md)）；macOS 使用 apple-native，Windows 使用 windows-native，Linux 使用 sync-secret-service + crypto-rust。其他平台明确不支持，不使用 mock 或明文 fallback。适配代码存在不代表该平台已完成运行验证。
- `frontend/`：React + TypeScript + Vite + shadcn 账号配置界面；可以保存与重载配置，尚未登录邮箱或收发邮件。
- `icons/`：从 `branding/logo.svg` 生成的正式应用图标，包含 Windows ICO、macOS ICNS、Linux PNG 和 Windows 商店尺寸；`tray/` 为后续托盘功能预备黑白与 macOS template 资源。

`tauri.conf.json` 设置 CSP，当前不允许 iframe 或远程内容。正式邮件阅读器尚未实现，届时仍需独立验证正文隔离，不能把主窗口权限直接交给邮件正文。[Tauri capability 说明](https://v2.tauri.app/security/capabilities/)。

## 前端与 Tauri 配置

`desktop/tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 分别调用根目录工作区中的前端脚本，`devUrl` 指向 Vite 的 `5173` 端口，`frontendDist` 指向 `../frontend/dist`。`scripts/tauri.mjs` 让根目录命令在配置文件位于 `desktop/` 子目录时仍能正确调用 Tauri CLI。账号界面经统一 MailxClient 调用 IPC，领域类型由 Rust 生成；契约仍可随实现演化。

应用标识为 `com.mailxapp.desktop`（对应域名 `mailxapp.com`）；Cargo 包名、命令行别名和镜像名称尚未作为正式命名冻结。`bundle.active` 为 `true`，`bundle.createUpdaterArtifacts` 让构建额外产出可自更新的 `.app.tar.gz` / NSIS / `.AppImage` 与对应 `.sig`，发版由 `.github/workflows/release.yml` 生成未签名的安装包和 `latest.json`，流程见[发版流程](../docs/development/release.md)。`plugins.updater` 的内置公钥只能验证对应私钥签出的包：没有私钥时，`bun run tauri build` 会在生成 `.app.tar.gz` 之后因缺少 `TAURI_SIGNING_PRIVATE_KEY` 以退出码 1 结束；本地构建可加 `--config '{"bundle":{"createUpdaterArtifacts":false}}'`。macOS 的 `bundle.targets` 因此包含 `app`（否则不会生成更新包）。用户数据目录和系统凭据 service 都不依赖 identifier，所以以后调整标识不会让已有账号和邮件缓存消失；但 Tauri 自己按 identifier 命名的 WebView 存储、缓存和窗口位置文件会随标识变化而重建。

更新主 Logo 后，在仓库根目录运行 `bun run icons:generate`，再用 `bun run icons:check` 验证生成文件。不要手工修改 `icons/` 下的派生资源；托盘资源的存在不代表托盘功能已开启。完整目录与使用方式见[品牌资源说明](../branding/README.md)。

账号模型、凭证更新恢复及未实现功能边界见[账号配置](../docs/architecture/account-configuration.md)。实际系统凭据库、平台构建与 IPC 验证分别记录，不能把替身测试当作跨平台支持证据。

真实系统凭据库的合成条目测试默认忽略，需要在已解锁的桌面环境显式运行：

```sh
cargo test -p mailx-desktop --locked native_secret_store_roundtrip -- --ignored
```

该测试使用独立 service，检查读写和幂等删除后清理条目；执行结果单独记录，普通测试通过不包含此项。
