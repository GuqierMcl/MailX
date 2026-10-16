# desktop

MailX 的 Tauri v2 桌面宿主，当前 Cargo 包名为 `mailx-desktop`，依赖 `mailx-core`。它负责窗口、受限 IPC、user-data-dir 与系统凭据库装配，账号及邮件业务放在 `core/`。

在仓库根目录执行：

```sh
bun install
bun run tauri dev
```

需要 Bun、Rust 和对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。Tauri CLI 作为 `frontend/` 的开发依赖安装，根目录脚本负责把嵌套的配置文件、前端开发服务器和 CLI 连接起来。前端生产资源位于 `frontend/dist`，普通 Rust 构建仍可用于只检查 Rust 代码，但桌面开发以 `bun run tauri dev` 为准。

## 当前范围

- `src/main.rs`：Tauri 程序入口与命令注册，取得 `app_data_dir()` 对应的 user-data-dir，并以应用 identifier 装配系统凭据库。
- `src/commands.rs`：`accounts_list` / `accounts_get` / `accounts_create` / `accounts_update` 适配 AccountService，返回生成的 `ApiResult<T>`；命令自行解析原始负载，仅返回固定脱敏错误，避免解析异常回显秘密。原骨架的 `core_info` 已移除。
- `src/state.rs`：持有宿主注入的目录与 SecretStore，在首次调用的阻塞线程中打开核心，随后串行运行账号服务；数据目录内为 `mailx.sqlite3` 和 `mailx.lock`，不使用仓库目录。
- `capabilities/main.json`：仅为本地 `main` 窗口开放四个账号管理命令，没有文件系统、Shell、网络插件或远程来源授权。`build.rs` 为这些命令生成权限定义；宿主同时校验调用窗口标签及来源 URL。
- `src/secret_store.rs`：使用 keyring 3.6 的原生后端，凭据 service 取应用 identifier；macOS 使用 apple-native，Windows 使用 windows-native，Linux 使用 sync-secret-service + crypto-rust。其他平台明确不支持，不使用 mock 或明文 fallback。适配代码存在不代表该平台已完成运行验证。
- `frontend/`：React + TypeScript + Vite + shadcn 账号配置界面；可以保存与重载配置，尚未登录邮箱或收发邮件。
- `icons/`：从 `branding/logo.svg` 生成的正式应用图标，包含 Windows ICO、macOS ICNS、Linux PNG 和 Windows 商店尺寸；`tray/` 为后续托盘功能预备黑白与 macOS template 资源。

`tauri.conf.json` 设置 CSP，当前不允许 iframe 或远程内容。正式邮件阅读器尚未实现，届时仍需独立验证正文隔离，不能把主窗口权限直接交给邮件正文。[Tauri capability 说明](https://v2.tauri.app/security/capabilities/)。

## 前端与 Tauri 配置

`desktop/tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 分别调用根目录工作区中的前端脚本，`devUrl` 指向 Vite 的 `5173` 端口，`frontendDist` 指向 `../frontend/dist`。`scripts/tauri.mjs` 让根目录命令在配置文件位于 `desktop/` 子目录时仍能正确调用 Tauri CLI。账号界面经统一 MailxClient 调用 IPC，领域类型由 Rust 生成；契约仍可随实现演化。

应用标识 `dev.mailx.desktop` 和 Cargo 包名仅服务当前开发。安装包、命令行别名、正式应用标识和镜像名称留待分发设计；`bundle.active` 当前为 `false`，尚无安装包或发布流程。已有账号配置存储在当前标识对应目录；后续修改标识必须明确迁移目录和系统凭据归属，不能使已有配置静默消失。

更新主 Logo 后，在仓库根目录运行 `bun run icons:generate`，再用 `bun run icons:check` 验证生成文件。不要手工修改 `icons/` 下的派生资源；托盘资源的存在不代表托盘功能已开启。完整目录与使用方式见[品牌资源说明](../branding/README.md)。

账号模型、凭证更新恢复及未实现功能边界见[账号配置](../docs/architecture/account-configuration.md)。实际系统凭据库、平台构建与 IPC 验证分别记录，不能把替身测试当作跨平台支持证据。

真实系统凭据库的合成条目测试默认忽略，需要在已解锁的桌面环境显式运行：

```sh
cargo test -p mailx-desktop --locked native_secret_store_roundtrip -- --ignored
```

该测试使用独立 service，检查读写和幂等删除后清理条目；执行结果单独记录，普通测试通过不包含此项。
