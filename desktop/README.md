# desktop

MailX 的 Tauri v2 桌面宿主，当前 Cargo 包名为 `mailx-desktop`，依赖 `mailx-core`。它负责窗口、IPC 与后续的平台集成，邮件逻辑放在 `core/`。

在仓库根目录执行：

```sh
bun install
bun run tauri dev
```

需要 Bun、Rust 和对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。Tauri CLI 作为 `frontend/` 的开发依赖安装，根目录脚本负责把嵌套的配置文件、前端开发服务器和 CLI 连接起来。前端生产资源位于 `frontend/dist`，普通 Rust 构建仍可用于只检查 Rust 代码，但桌面开发以 `bun run tauri dev` 为准。

## 当前范围

- `src/main.rs`：Tauri 程序入口与命令注册。
- `src/commands.rs`：唯一的 `core_info` 开发检查命令，从核心返回 `{ "coreVersion": "0.1.0" }`；不代表邮件服务已就绪。
- `capabilities/main.json`：仅为本地 `main` 窗口开放 `allow-core-info`，没有文件系统、Shell、网络插件或远程来源授权。`build.rs` 为该命令生成权限定义。
- `frontend/`：React + TypeScript + Vite + shadcn 前端工程骨架；当前初始化模板尚未实现邮件功能。
- `icons/`：为跨平台构建提供的开发占位图标，后续可替换。

`tauri.conf.json` 设置 CSP，当前不允许 iframe 或远程内容。正式邮件阅读器尚未实现，届时仍需独立验证正文隔离，不能把主窗口权限直接交给邮件正文。[Tauri capability 说明](https://v2.tauri.app/security/capabilities/)。

## 前端与 Tauri 配置

`desktop/tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 分别调用根目录工作区中的前端脚本，`devUrl` 指向 Vite 的 `5173` 端口，`frontendDist` 指向 `../frontend/dist`。`scripts/tauri.mjs` 让根目录命令在配置文件位于 `desktop/` 子目录时仍能正确调用 Tauri CLI。正式前端接入 IPC 时可以改为模块化 API 和统一客户端适配器，不能把当前初始化模板视为已冻结的应用契约。

应用标识 `dev.mailx.desktop`、Cargo 包名和占位图标仅服务当前开发。安装包、命令行别名、正式应用标识和镜像名称留待分发设计；`bundle.active` 当前为 `false`，尚无安装包或发布流程。首次发布前确定标识，避免后续改变平台数据目录归属。
