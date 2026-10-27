# 桌面应用内更新

状态：**已实现；端到端更新已在 Windows 上实测通过，macOS 与 Linux 未验证**（设计确认于 2026-10-27）。本文记录检查时机、网络路径、发版规则、签名与交互的已确认决定与实现；验证记录见第 10 节，没有实测的部分在那里如实列出。

## 1. 目标与边界

- 让桌面版在用户同意后下载、安装新版本，不必手动去 GitHub 下载安装包。
- 只覆盖桌面宿主（`desktop/`）。核心库不感知更新；`server/` 以后另行设计。
- 不包含代码签名（macOS 公证、Windows Authenticode）。未签名阶段 macOS 更新后会被当作新应用，钥匙串会再请求一次授权（凭据已合并为单条目，所以只有一次），这一点已接受。
- 不做“跳过此版本”，不做测试通道，不做自动下载（以后可以加默认关闭的开关）。

## 2. 发版规则：Pre-release 标记就是推送开关

- 应用只认 GitHub **Latest Release**：检查地址是 `https://github.com/GuqierMcl/MailX/releases/latest/download/latest.json`。GitHub 的 Latest 是最近一个非草稿、非 pre-release 的 Release，按 Release 所基于提交的时间排序。
- 因此规则只有一条：**发布时不勾 Pre-release = 推送给所有用户**，与版本后缀（`alpha`、`beta`、`rc`）无关。想发一个不推送的测试版，就勾上 Pre-release。
- 发版工作流**不再按后缀自动勾 Pre-release**，草稿默认是普通 Release，由维护者在发布时决定。发布说明开头的“预发布版本”提示仍按后缀显示。
- 更新器只在远端版本按 SemVer **高于**当前版本时提示，所以不会降级；`0.2.0-rc.1 < 0.2.0`，装了 rc 的用户会正常收到正式版。
- **撤回**：发出的版本有严重问题时，把它改回 Pre-release 或草稿，Latest 自动回到上一个版本，未更新的用户不会再收到它；已更新的用户不会被降级，接着发更高版本号的修复版。
- 只维护一条版本线，总在最新 `main` 上打标签，Latest 总会指向最新版本。以后若给旧版本线补发修复，发布时用 `make_latest=false`。
- 已发布的 `v0.1.0-alpha.1`、`v0.1.0-alpha.2` 没有内置更新器公钥，**永远不会自动更新**，这些用户需要手动安装第一个带更新器的版本。

## 3. 签名密钥

- 更新器签名用 Tauri 的 minisign 密钥对。公钥（密钥 ID `B0DE08C9ED9EA8E9`）写入 `desktop/tauri.conf.json` 的 `plugins.updater.pubkey`，随应用发布。
- 私钥与密码在 GitHub `release` 环境的 secrets：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥由维护者离线保管，**未设密码**；`…_PASSWORD` secret 已存在，内容未知，第一次接入时需先用手动触发的工作流单独验证签名能成功。
- `release` 环境目前**没有**分支 / 标签保护。建议维护者在网页上把它的部署分支 / 标签规则限制为 **`v*` 标签和 `main` 分支**两者：构建任务都在这个环境里跑，而打开 `createUpdaterArtifacts` 后没有私钥的构建会直接失败，所以 `workflow_dispatch` 的试跑同样需要它——从 `main` 触发是允许的，从其他分支触发会被环境拒绝。这需要在 GitHub 网页上手动设置，代码里不配置。
- 私钥或密码丢失：已安装用户无法再自动更新，只能手动安装带新公钥的版本。更换公钥同理，需要过渡版本。

## 4. 产物与 `latest.json`

- `bundle.createUpdaterArtifacts: true`，构建时为可更新的格式生成更新包和 `.sig` 签名：macOS 为 `.app.tar.gz`，Windows 为 NSIS `-setup.exe`，Linux 为 `.AppImage`。`.deb`、`.rpm` 不能自更新，这部分 Linux 用户仍需手动安装。
- 构建任务的签名 secrets 只在 `environment: release` 下提供；手动触发（`workflow_dispatch`）的试跑也需要签名时同样走该环境。
- 发版工作流生成 `latest.json`，作为 Release 附件上传：

  ```json
  {
    "version": "0.2.0",
    "notes": "<CHANGELOG.md 中该版本的小节，不含安装说明页脚和预发布提示>",
    "pub_date": "2026-11-01T08:00:00Z",
    "platforms": {
      "darwin-aarch64": { "signature": "<.sig 文件内容>", "url": "https://github.com/GuqierMcl/MailX/releases/download/v0.2.0/MailX.app.tar.gz" },
      "darwin-x86_64": { "signature": "…", "url": "…" },
      "windows-x86_64": { "signature": "…", "url": "…" },
      "linux-x86_64": { "signature": "…", "url": "…" }
    }
  }
  ```

  文件名由 `scripts/release-assets.mjs` 统一改为 `MailX_<版本>_<平台><扩展名>`，平台取发布矩阵的名字（`macos-arm64`、`macos-x64`、`windows-x64`、`linux-x64`），`.sig` 跟随同名：`MailX_0.1.0-alpha.4_macos-arm64.dmg`、`MailX_0.1.0-alpha.4_windows-x64-setup.exe`、`MailX_0.1.0-alpha.4_macos-x64.app.tar.gz`。用户按操作系统选下载，架构只在名字里体现。缺任何一个平台都让工作流失败，不发出不完整的 `latest.json`。
- 更新说明取自 `notes`：一次请求同时拿到版本与说明，不访问 GitHub API。在网页上修改 Release 正文不会同步到 `notes`，正本仍是 `CHANGELOG.md`。

## 5. 网络路径

- 更新请求由桌面宿主发出，走应用自己的 HTTP 客户端（`tauri-plugin-updater` 内置），**不经过核心的账号路由，也不读取任何账号的代理设置**。账号代理只服务该账号的邮件协议连接。
- 应遵循系统代理；TUN 在网络层生效，无需处理。**对锁定版本源码的核实结论**（`tauri-plugin-updater` 2.14.0、`reqwest` 0.13.5、`hyper-util` 0.1.21，源码在 `~/.cargo/registry/src/*/` 下）：**macOS 与 Windows 默认读取系统代理，无需额外配置**。
  - `tauri-plugin-updater-2.14.0/Cargo.toml` 的默认特性含 `system-proxy`，它打开 `reqwest/system-proxy`；`desktop/Cargo.toml` 使用默认特性，未关闭。
  - `tauri-plugin-updater-2.14.0/src/updater.rs:584`、`:842`：用 `ClientBuilder::new()` 建客户端，只有显式配置 `no_proxy` / `proxy` 才改变行为（`:35-41`），本应用不配置。
  - `reqwest-0.13.5/src/async_impl/client.rs:310`：`auto_sys_proxy` 默认 `true`；`:417-418`：为真时追加 `ProxyMatcher::system()`。
  - `hyper-util-0.1.21/src/client/proxy/matcher.rs:238-247`（`from_system`）：先读环境变量（`HTTPS_PROXY` / `NO_PROXY` 等），再按平台读系统设置——macOS 走 `mac::with_system`（SystemConfiguration 的代理设置），Windows 走 `win::with_system`（注册表 Internet Settings）。
  - **Linux 只读环境变量**，不读桌面环境（GNOME / KDE）的代理设置；Linux 用户需要代理时应通过 `HTTPS_PROXY` 启动应用。这是结论，不是已验证的行为（本机未在 Linux 上运行）。
  - 宿主不使用 `core/src/net`，也不读任何账号的代理；自定义账号代理失败“不得回退直连”的规则只约束邮件协议连接，与更新请求无关。
- 只请求 `github.com` 的 Release 下载地址（含其重定向目标）；不新增其他域名。

## 6. 检查时机

| 触发 | 规则 |
| --- | --- |
| 启动检查 | 启动约 30 秒后检查一次 |
| 定时检查 | 运行期间每 6 小时一次 |
| 手动检查 | 设置 → 关于 →“检查更新”，随时可用 |

- 启动和定时检查失败都**静默**，等下一轮；手动检查失败显示原因和重试。
- 发现新版本后停止定时检查，不重复检查同一个结果。
- 设置“自动检查更新”（`general.autoCheckUpdates`，默认开启）：关闭后只停止启动与定时检查，手动检查仍可用。

## 7. 流程：发现、下载、安装三步，下载需用户同意

1. **发现**：只通知，不下载。
2. **下载**：用户在更新弹窗里点“下载更新”才开始；下载中可以关闭弹窗继续使用。若插件能干净地中断下载，提供“取消下载”；不能则不提供，并在本文记录。
3. **安装**：下载完成后，用户选“立即重启”，或“稍后”——稍后表示**下次退出应用时自动安装**（用户已在第 2 步同意更新）。

用户拒绝的方式就是不点“下载更新”：关闭 toast 或弹窗即可，标题栏图标一直保留，随时可再打开。

**安装与退出安全**：

- “立即重启”与“退出时安装”都必须走现有的优雅退出路径（`compose::main_close_requested`，先让写信窗口保存草稿），不能直接结束进程。
- 有邮件处于正在发送状态时，“立即重启”不可用并说明原因，发送结束后恢复。
- Windows 上安装会运行 NSIS 安装程序并结束应用，安装程序进度窗口短暂出现，这是正常行为。
- 安装失败不影响当前版本继续运行；在更新弹窗显示原因，可重试。

## 8. 界面

全部文字中英两套，经过 `i18n:check`。写信窗口不显示任何更新界面，也不能触发检查、下载或安装。

### 8.1 Toast

- **自动检查发现新版本**：弹一个**不自动关闭**的 toast，标题“MailX 0.2.0 可用”，唯一按钮“查看更新”，点开更新弹窗。同一版本**每次运行只弹一次**；关闭后靠标题栏图标提醒。
- **手动检查发现新版本**：不弹 toast，直接打开更新弹窗（用户正在等待结果）。没有新版本时在“关于”页显示“已是最新版本”。
- **下载完成且弹窗已关闭**：弹 toast“0.2.0 已下载”，按钮“立即重启”。

### 8.2 更新弹窗

- 标题含新版本号，副标题为当前版本 → 新版本，以及发布日期（若有）。
- 正文是渲染后的更新说明（见 8.4），可滚动。
- 底部按钮随阶段变化：发现时“下载更新”与“稍后”；下载中显示进度条（可选“取消下载”）；就绪时“立即重启”与“稍后（退出时安装）”；失败时显示原因与“重试”。
- 提供“在 GitHub 查看”链接，用系统浏览器打开对应 Release 页面。

### 8.3 标题栏图标

- 位置：标题栏右侧 `trailing` 插槽，明暗模式按钮左边；与它**同尺寸的纯图标按钮**，悬停显示提示文字，不展开、不改变宽度，避免标题栏抖动。
- 有可用更新时才出现：

  | 阶段 | 外观 | 提示文字 |
  | --- | --- | --- |
  | 发现 | 普通前景色的上箭头圆圈图标，右上角主色小圆点 | “MailX 0.2.0 可用” |
  | 下载中 | 同尺寸的圆形进度环 | “正在下载 0.2.0（45%）” |
  | 就绪 | 同一图标整体改为主色 | “重启以更新到 0.2.0” |

- 无闪烁、无脉冲动画；`aria-label` 与提示文字一致，包含版本号。
- 点击任一阶段都打开同一个更新弹窗。
- 不在设置齿轮上加小圆点，同一件事只有一个常驻提示。

### 8.4 更新说明的轻量 Markdown 渲染

- 自行实现，不引入 Markdown 库。支持：`#`–`###` 标题、段落、`- ` / `* ` 列表、`> ` 引用、`**粗体**`、`` `代码` ``、`[文字](链接)`。其余语法按原文显示。
- 渲染为 React 元素，**不使用 `dangerouslySetInnerHTML`，原始 HTML 一律按文本显示**。
- 链接只接受 `https:`，点击后由宿主用系统浏览器打开；其他协议（含 `javascript:`、`data:`、`file:`）按纯文本显示。
- `notes` 超过 20 000 字符时截断并提示可在 GitHub 查看完整说明。
- `latest.json` 本身没有签名（签名只覆盖安装包），所以说明按不可信文本处理。

### 8.5 设置 → 关于

- 现有“版本”行下方加状态行：已是最新 / 正在检查 / 有可用更新（点击打开更新弹窗）/ 失败原因与重试；加“检查更新”按钮。**不显示上次检查时间**。
- 加“自动检查更新”开关。

## 9. 权限与安全

- 更新相关命令只允许主窗口调用，在 `desktop/src/window_access.rs` 的白名单里与现有命令同样处理；邮件正文 WebView 与写信窗口都不能调用。
- 前端不直接使用插件的 JS API，统一经宿主命令和事件，保持与其他功能一致的 IPC 校验方式；不在 capabilities 中授予 `updater:default` 给任何窗口，除非实现时证明必须且仅限主窗口。
- 下载包由插件用内置公钥验签，验签失败拒绝安装并报错。
- 日志与错误信息不包含签名、下载地址以外的敏感内容；不记录 `notes` 全文。

## 10. 验证计划

- **单元测试**：Markdown 渲染（含 HTML 注入、`javascript:` 链接、超长截断）；检查调度（启动延迟、6 小时间隔、开关关闭、发现后停止）；前端状态机（发现 → 下载 → 就绪 → 重启 / 稍后，失败与重试）；`latest.json` 生成脚本（缺平台失败、`notes` 不含页脚）；窗口白名单表。
- **工作流**：先用手动触发验证签名能成功、`.sig` 与 `latest.json` 生成正确，再打标签。
- **手动端到端**：装一个带更新器的旧版本，发布一个更高版本的普通 Release，验证检查、下载、立即重启、退出时安装在 macOS 与 Windows 上各走通一次；Linux AppImage 能做则做，未做如实记录。
- 自动测试通过不等于真实更新已验证；未实测的平台在文档中写明。

### 验证记录（2026-10-27，macOS arm64 开发机）

已运行并通过：`bun run check`（含 `version:check`、`i18n:check`、`types:check`、`typecheck`、`lint`、前端测试、`build`）、`bun run version:test`（含 `scripts/updater-manifest.test.mjs`）、`cargo fmt`、`clippy -D warnings`、`cargo test --workspace --locked`（含宿主 `updater` 的状态机 / 日程 / 版本 / 清单解析 / 链接白名单测试与窗口白名单表）、`actionlint`、`git diff --check`。

`bun run tauri build` 的实际结果：

- `bundle.targets` 在 macOS 上原本只有 `dmg`，此时**不会**生成 `.app.tar.gz`。已加入 `app` 目标（实现中的唯一偏离：规格要求保持 targets 不变，但没有它就没有 macOS 更新包）。
- 加入后构建产出 `MailX.app`、`MailX_0.1.0-alpha.2_aarch64.dmg` 和 `MailX.app.tar.gz (updater)`，随后因缺少私钥报 `A public key has been found, but no private key. Make sure to set TAURI_SIGNING_PRIVATE_KEY`，**命令以退出码 1 结束，没有生成 `.sig`**。没有用任何密钥绕过。
- 因此没有私钥的本地构建需要关闭更新包：`bun run tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'`（已确认退出码 0，只产出 `.app` 与 `.dmg`）。CI 在 `release` 环境里提供密钥。

**后续修正**（验收反馈）：下载不再沿用检查的 30 秒超时——`Update::timeout` 在开始下载前改为 20 分钟（`DOWNLOAD_TIMEOUT`，检查仍是 `CHECK_TIMEOUT` = 30 秒），因为插件把这个超时用作整个下载请求的上限（`tauri-plugin-updater-2.14.0/src/updater.rs:328` 的说明，`:591` 处应用）。

**真实端到端更新（2026-10-27，Windows，维护者手动验证）**：已安装 `0.1.0-alpha.3`，从应用内检查到 `0.1.0-alpha.4`、下载、安装，两条路径都走通——

- “立即重启”：重启后版本为新版本。
- “稍后”：退出应用时出现安装窗口一瞬间，随后正常结束；下次启动已是新版本。

验证的发布：`v0.1.0-alpha.3` 与 `v0.1.0-alpha.4` 均以普通 Release 发布（不勾 Pre-release），`latest.json` 由 CI 生成并上传，四平台签名齐全，文件名按 `MailX_<版本>_<平台><扩展名>` 命名。

**仍未验证**：macOS 与 Linux 上的端到端更新；Linux AppImage 的替换行为；macOS、Windows、Linux 三个平台上的系统代理实际表现（仅核对了源码，没有在受限网络里测过）；下载取消与中途断网的界面反馈；多版本跨度的升级（目前只验证了相邻的 alpha.3 → alpha.4）。

## 11. 服务端兼容

状态：**规划，未实现**。未来服务器 / Web 版只做“提示有新版本并显示更新说明”，不下载、不安装；界面上用 `canInstall: false` 表达这一点。

- **能力开关而不是写死安装**：宿主发给前端的更新状态带 `canInstall`（桌面端为 `true`）。前端所有更新界面（标题栏图标、弹窗、通知、关于页）都经 `MailxClient` 取状态，并按 `canInstall` 分支：为 `false` 时弹窗只有版本、渲染后的更新说明和“在 GitHub 查看”链接，没有下载 / 重启按钮与进度，标题栏图标只会出现“有新版本”一种状态。前端测试覆盖了 `canInstall: false` 不出现任何下载 / 安装控件。
- **检查由服务端进程发起，浏览器不检查**：避免把访问者的 IP 暴露给 GitHub（隐私），也不需要放宽页面的 CSP。检查沿用标准代理环境变量（`HTTPS_PROXY` / `NO_PROXY` 等），不使用任何邮箱账号的代理。
- **可由服务端配置关闭自动检查**：离线或自托管部署必须能关掉，不留任何出站请求。
- **读同一份 `latest.json`**：服务端只读 `version`、`notes`（可选 `pub_date`），不要求 `platforms`；版本比较与清单解析是桌面宿主里不依赖 Tauri 的纯函数（`desktop/src/updater/policy.rs`），日后可原样移到 `core` 供服务端使用，当前**没有**移动。
- **统一版本号的副作用**：只改桌面端的发布也会让服务端用户收到“有新版本”。CHANGELOG 条目可以用 `[桌面端]` / `[服务端]` 前缀，让更新说明里一眼看出与自己有无关系。
