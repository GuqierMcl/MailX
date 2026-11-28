# 发版流程

本文记录版本管理、构建、草稿验证与发布流程。工程版本以根 `package.json` 为准，已发布版本与 Latest 以 [Releases](https://github.com/GuqierMcl/MailX/releases) 为准。操作系统代码签名和更新包签名分别说明；应用内更新机制见[更新器设计](../architecture/updater.md)，实际验证范围见[验证指引](validation.md)。

## 发布前检查

每次发布前按候选提交和[验证指引](validation.md)确认以下项目。实际结果记录在当次 PR 描述和 CI 运行记录，发版时在 Release 说明中概括已覆盖的平台、服务商与场景。

- **对外说明**：中英文 README、产品范围与路线图应与实现一致，区分已有能力、已知限制和后续计划。尤其核对搜索覆盖范围、本地与服务器动作的区别，以及 MCP 的授权范围。
- **发布说明与升级**：检查 `[Unreleased]` 是否涵盖面向用户的改动，并核对每条日志是否已附上对应 PR 的完整链接（有对应 PR 但遗漏的应在发版前或发版 PR 中补齐）。内部数据库升级及迁移版本只记在[数据库迁移清单](../architecture/database-migrations.md)。用户需要执行的升级步骤或注意的兼容性限制写进[升级指南](../guides/zh-CN/README.md#升级与数据保留)。跨版本升级前备份数据目录，确认账号、缓存、草稿、附件与发送记录保留。版本脚本只归并日志，升级路径需单独验证。
- **自动检查**：候选提交通过 `bun run check`、Rust 格式 / Clippy / 测试及三平台 CI，并核对 Linux AppImage 启动检查。检查结果应对应实际准备发布的提交。
- **正文边界专项验证**：修改正文 CSP、sandbox、自定义协议、链接拦截或升级 Tauri / Wry 时，按受影响行为和平台安排[专项原生验证](validation.md#body-native-validation)，记录验证方法、实际结果与未覆盖范围。普通前端和 Rust 检查不等同于真实引擎的隔离验证。
- **安装包冒烟**：对草稿中的各平台安装包验证安装启动、登录、同步、阅读、搜索、写信发信与附件、删除撤销、本机归档、退出 / 后台运行，以及 MCP、自启动和本次变更。按平台、客户端和服务商记录范围，未覆盖项明确保留为限制；既有手测或容器启动证据不能替代候选安装包的验证。
- **更新路径**：用带更新器的旧版验证升级后数据可用，检查默认自动下载、关闭开关后手动下载、立即重启与退出时安装。补齐取消、断网重试、缓存恢复和安装失败反馈，按相邻版本和跨度升级分别记录。检查方法见[更新器验证计划](../architecture/updater.md#10-验证计划)。
- **安装说明**：按实际签名状态核对首次启动提示、安装包平台 / 架构、`SHA256SUMS.txt`、各平台更新包与 `.sig`、`latest.json`。更新包验签与操作系统代码签名是两件事。

从预发布版转为正式版时，用 `bun run version:bump release --dry-run` 预览，再执行 `bun run version:bump release`；它会去掉预发布后缀，并将 `[Unreleased]` 归并为当日的小节。随后运行 `bun run version:check --release` 和 `bun run release:notes`，按下文流程合并发版 PR、打标签、验证草稿，最后发布。

## 1. 版本规则

- 采用 SemVer：`x.y.z`，或带预发布后缀 `x.y.z-alpha.N`、`-beta.N`、`-rc.N`（`N` 从 1 开始）。顺序：`alpha < beta < rc < 正式版`。
- 所有版本号保持一致：根 `package.json`、`frontend/package.json`、`Cargo.toml` 的 `[workspace.package]`，以及 `Cargo.lock` 中各 workspace 成员。版本文件清单由脚本从 `workspaces` 和 Cargo `members` 推导，新增成员（例如 `server`）只要继承 `version.workspace = true` 就自动纳入。
- Windows 只构建 NSIS（`.exe`）。MSI 要求预发布标识为纯数字，`alpha.1` 这类后缀无法打 MSI，所以 `bundle.targets` 里不含 `msi`。

## 2. 版本脚本

脚本只改文件，不提交、不打标签；加 `--dry-run` 只预览。

| 命令 | 示例（当前版本 → 结果） |
| --- | --- |
| `bun run version:bump patch \| minor \| major` | `0.1.0` → `0.1.1` / `0.2.0` / `1.0.0`（仅正式版可用） |
| `bun run version:bump prepatch \| preminor \| premajor <alpha\|beta\|rc>` | `0.1.0` → `0.1.1-alpha.1` / `0.2.0-beta.1` / `1.0.0-rc.1` |
| `bun run version:bump pre` | `0.2.0-alpha.1` → `0.2.0-alpha.2` |
| `bun run version:bump pre beta` | `0.2.0-alpha.2` → `0.2.0-beta.1`（只能往后切，不能退回） |
| `bun run version:bump release` | `0.2.0-rc.2` → `0.2.0` |
| `bun run version:set <版本号>` | 直接设置；不高于当前版本时需 `--force` |

刻意不做的隐式行为：预发布版上执行 `patch` 会报错，而不是悄悄变成正式版。已存在同名标签的版本会被拒绝。

**首次预发布的初始化**：只有从尚未发布的正式工程版本（如 `0.1.0`）切换为更低的预发布版本（如 `0.1.0-alpha.1`）时，才需要 `bun run version:set 0.1.0-alpha.1 --force`。已经处于 alpha / beta / rc 阶段时，用 `bun run version:bump pre` 递增，不重复初始化；可先加 `--dry-run` 预览。实际当前版本以根 `package.json` 和 `version:check` 为准。

## 3. 更新日志

`CHANGELOG.md` 采用 Keep a Changelog 格式，是发布说明的唯一来源。

- 面向用户的改动，在 PR 里把一行写到 `## [Unreleased]` 之下，按以下固定顺序分到对应分类（没有内容的分类不写）：`### Feature`（新功能）、`### Optimization`（行为不变的改进）、`### Fixed`（缺陷修复）、`### Security`（HTML 隔离、凭证、权限、代理等加固）、`### Changed`（不兼容的行为、数据格式、配置变化与移除的功能）。每条以 `- ` 开头，脚本只认这个前缀，不校验分类名与顺序。
- 条目写给用户看，一行一句；测试、CI、重构、文档等内部改动不写。结尾带 PR 的完整链接，例如 `([#25](https://github.com/GuqierMcl/MailX/pull/25))`：`#25` 这种简写在 Release 页面里会被 GitHub 自动链接，但在仓库的 `.md` 文件里不会，完整链接两处都可点击。
- **预发布版**不新增小节，发布说明取自 `[Unreleased]`，并自动加上“预发布版本”提示。
- **正式版**：`version:bump` / `version:set` 把 `[Unreleased]` 归并为 `## [x.y.z] - 日期`、留下新的空 `[Unreleased]` 并更新底部对比链接。`[Unreleased]` 为空会被拒绝；确需跳过用 `--no-changelog`。
- 发布说明末尾追加 `.github/release-install-notes.md`（安装说明：macOS 已签名并公证，Windows 未签名与钥匙串过渡提示）。内容随签名状态调整，见第 9 节。
- `bun run version:check --release` 检查日志是否可发布；`bun run release:notes [版本]` 可本地预览将要发布的说明。

## 4. 发布即推送：Pre-release 标记是开关

发版工作流**不再按版本后缀自动勾选 Pre-release**，草稿默认是普通 Release，由维护者在发布时决定。应用内更新只读 GitHub 的 **Latest Release**（`https://github.com/GuqierMcl/MailX/releases/latest/download/latest.json`），而 Latest 是最近一个非草稿、非 Pre-release 的 Release。

- **不勾 Pre-release 直接发布 = 推送给所有已经装了带更新器版本的用户**，与版本号里有没有 `alpha` / `beta` / `rc` 无关。开启自动更新的客户端会后台下载，随后提醒重启或在正常退出时安装；发布前必须按此实际影响验证草稿。
- **想发一个不推送的测试版**：勾上 Pre-release 再发布。之后要让它推送，编辑 Release 取消 Pre-release 即可（Latest 随之指向它）。
- 发布说明开头的“预发布版本”提示仍按后缀显示，和推不推送是两件事。
- 更新器只在远端版本 SemVer **高于**当前版本时提示，所以不会降级（`0.2.0-rc.1 < 0.2.0`）；装了 rc 的用户会正常收到正式版。
- **撤回一个坏版本**：在 Release 页面把它改回 Pre-release（或改回草稿）。Latest 回到上一个版本；客户端在后续成功检查时清理或替换被撤回的目标，跨进程复用缓存也要先核对新清单。已下载的运行中实例仍可能在下一次检查前退出并安装，因此不能把撤回当作即时阻止全部安装的保证；已经更新的用户不会被降级，需继续发布更高版本号的修复版。不要删除已发布的标签。
- 已发布的 `v0.1.0-alpha.1`、`v0.1.0-alpha.2` 发布时还没有内置更新器公钥，**永远不会自动更新**；用过它们的用户要手动安装第一个带更新器的版本（`v0.1.0-alpha.2` 之后的那个）。

## 5. 每次发版的步骤

1. **确认 main 的 CI 是绿的**（合并发版 PR 时 main 最新提交的 `CI result` 已通过，合并后才能只跑轻量检查），并确认 `CHANGELOG.md` 的 `[Unreleased]` 已写全、所有有对应 PR 的条目均已附上完整 PR 链接（如有遗漏，可直接在本次发版 PR 的 `CHANGELOG.md` 中补齐，不影响发版 PR 的轻量 CI 判定）。
2. **新建发版分支**：`git switch main && git pull --ff-only && git switch -c release/vX.Y.Z`。
3. **改版本**：先 `bun run version:bump … --dry-run` 看结果，再去掉 `--dry-run` 执行。
4. **检查**：`git diff`（应只有版本文件和 `CHANGELOG.md`）；`bun run version:check --release`、`bun run version:test`。发版 PR 只要保持这种改动，CI 就只跑这两项轻量检查（见下文“发版 PR 的 CI”）。
5. **提交并开 PR**：`git commit -am "chore(release): vX.Y.Z"`，推送、等 CI 通过、合并。
6. **等 `main` 的 CI 通过后打标签**（必须是合并后的 main，不是 PR 分支）：合并前 `main` 的 CI 已通过时，合并提交在 `main` 上同样只跑轻量检查，约一分钟即可打标签；否则会跑全量 CI，需要等更久。都以 `CI result` 变绿为准。提前推送的标签会被发版工作流拒绝，CI 通过后在 Actions 页重跑该次运行即可。
   ```sh
   git switch main && git pull --ff-only
   bun run version:check --release     # 仍然通过
   git tag -a vX.Y.Z -m "MailX X.Y.Z"
   git push origin vX.Y.Z
   ```
7. **等待构建**：推标签触发 `Release` 工作流（Actions 页）。依次是校验版本、四平台构建、全部成功后创建**草稿** Release。
8. **验证草稿**：在仓库 `Releases` 页找到带 `Draft` 标记的条目，下载各平台安装包，走安装、登录、同步、阅读、发信与本次变更的冒烟；核对文件名中的版本号、`SHA256SUMS.txt`、更新包 / 签名和 `latest.json`。未验证的平台或场景须在发布说明中明确；检查范围见上面的发布前检查。
9. **（可选）修改说明**：草稿的 `Edit`（铅笔）页可改正文，但只改 GitHub 上那份，不会回写 `CHANGELOG.md`。有价值的改动请同步进日志；想让正文重新生成，见第 7 节。
10. **发布**：草稿的 `Edit` 页底部点 **Publish release**（命令行：`gh release edit vX.Y.Z --draft=false`）。**发布前决定要不要勾 Pre-release**：不勾就是推送给所有用户，见第 4 节。工作流不会替你勾。
11. **观察更新**：已装带更新器版本的用户在启动约 5 秒后检查，之后每 1 小时检查，间隔从上次自动检查完成后计算。当前默认开启自动更新，发现版本后先后台下载，完成才提醒重启；关闭自动更新的用户只收到版本提示。用旧版分别验证自动与手动下载、说明与进度、立即重启、正常退出安装和数据保留，记录对应客户端版本与平台。

发版后**不需要**再把版本改成下一个开发版本；`main` 保持在最近发布的版本号，下次发版再改。

### 发版 PR 的 CI

CI 的变更检测把 PR 分成四类：只改文档、只改官网（可同时改文档）、发版、其他改动。前两类由 `scripts/check-change-scope.mjs` 判定，同样使用基准提交上的版本；只改官网时只运行 `Website` 检查，前端和三平台 Rust 检查跳过。发版 PR 的判定由 `scripts/check-release-pr.mjs` 完成（使用基准分支上的版本，PR 不能改变自己的判定方式）：改动的文件只能是版本文件和 `CHANGELOG.md`，版本必须升高，且每个版本文件都必须与在基准提交上运行 `version:bump` / `version:set` 写出的内容一字不差；`Cargo.lock` 中只允许 MailX 自身成员的版本变化。满足时只运行 `Release PR checks`（`version:check --release` 与 `version:test`），前端和三平台 Rust 检查跳过；任何其他改动或判定出错都按普通改动跑全量检查。

推送到 `main` 时，只有同时满足以下条件才按发版处理、只跑上述轻量检查：推送前的 `main` 提交在 `main` 上的 `CI result` 最近一次全部为成功；从该提交到本次推送的全部改动按同一脚本（取自推送前的提交）判定为发版。这时代码与一个已通过全量检查的提交完全相同，只有版本号和日志不同，重复跑全量检查没有意义；安装包的编译仍由发版工作流在四个平台上完成。推送前的提交 CI 未完成、失败、被取消、查询出错，或推送中含其他改动时，一律跑全量检查。推送前的提交 CI 已通过、且全部改动都属于官网（`website/`、`docs/guides/`、`docs/assets/screenshots/`）时，`main` 也只跑 `Website` 检查。`main` 上其他只改文档的推送仍跑全量检查。发版工作流在构建前核对标签提交的 `CI result` 已通过。

## 6. 工作流做了什么

`.github/workflows/release.yml`：

Linux x64 与 ARM64 安装包均在原生 Ubuntu 24.04 环境构建，以 glibc 2.39 为基线。提高系统基线时需验证新基线上的实际启动，并在发布说明里告知旧系统用户先升级操作系统。既有 x64 AppImage 更新器按架构从 Latest 选择产物，无法按用户的 Linux 版本筛选。验证过渡版本时可使用工作流产物或 GitHub Pre-release，避免旧系统提前收到不兼容的自动更新。

Linux 构建先运行 `bun scripts/prepare-linux-appimage.mjs --out "$RUNNER_TEMP/tauri-linux-appimage.json"`，再把该临时配置传给 `bun run tauri build --config "$RUNNER_TEMP/tauri-linux-appimage.json"`。脚本从构建机的 `pkg-config` 库目录解析 AppIndicator、Ayatana Indicator/Ido 与 dbusmenu 的五个 SONAME，检查 ELF、SONAME 和依赖闭包，并把真实文件复制到 AppImage 的 `usr/lib`。这组库由托盘后端动态加载，普通链接依赖扫描会遗漏；其余依赖和 RPATH 仍由 linuxdeploy 处理。macOS 与 Windows 继续使用原构建命令。

- **触发**：推送 `v*` 标签；或在 Actions 页手动 `Run workflow`（只构建、产物留在该次运行里 7 天，不创建 Release，用来调试工作流）。
- **verify**：标签必须等于仓库版本（`v` + `package.json` 的 `version`）；`CHANGELOG.md` 必须可发布；标签指向的提交必须是 `main` 的祖先，且该提交在 `main` 上的 `CI result` 必须已经通过（发版提交在 `main` 上的判定规则见上文“发版 PR 的 CI”）。版本后缀只决定发布说明中的预发布提示，不设置 GitHub 的 Pre-release 标记。
- **build**：macOS arm64（`macos-26`）、macOS x64（`macos-26-intel`）、Windows x64（`windows-2025`）、Windows ARM64（`windows-11-arm`，Windows 11 客户端，原生构建 `aarch64-pc-windows-msvc`）、Linux x64（`ubuntu-24.04`）、Linux ARM64（`ubuntu-24.04-arm`），都在 GitHub 的 `release` 环境里运行。`bun run tauri build` 生成 `.dmg`、`-setup.exe`、`.deb`、`.rpm`、`.AppImage`，并在 `bundle.createUpdaterArtifacts` 打开时额外生成可自更新的包（macOS `.app.tar.gz`、Windows NSIS `-setup.exe`、Linux `.AppImage`）与各自的 `.sig`。macOS 包必须用 SDK 26 或更新版本构建：macOS 26 起，用旧 SDK 构建的应用会进入兼容模式，红绿灯的大小和位置与标题栏布局不符。构建后由 `scripts/check-macos-sdk.sh` 用 `otool` 检查 `.app` 内每个可执行文件的 SDK 版本，同时识别 Intel 低部署目标使用的 `LC_VERSION_MIN_MACOSX` 与较新目标的 `LC_BUILD_VERSION`；未知或低于 26 均失败；本地开发构建使用本机的 Xcode 或命令行工具，版本可能与 CI 不同。Windows 构建后由 `scripts/check-windows-binaries.mjs` 读取主程序与 MCP sidecar 的 PE 头，确认 CPU 类型与平台一致（x64 为 `0x8664`，ARM64 为 `0xAA64`；NSIS 安装程序外壳本身始终是 x86，不在检查范围）。收集时统一改为 `MailX_<版本>_<平台><扩展名>`，例如 `MailX_<版本>_macos-arm64.app.tar.gz` 与 `MailX_<版本>_macos-x64.app.tar.gz`。任一平台失败即整体失败。
- **release**：仅标签触发，全部平台成功后，用 `scripts/updater-manifest.mjs` 从收集到的包与 `CHANGELOG.md` 生成 `latest.json`（缺 `darwin-aarch64`、`darwin-x86_64`、`windows-x86_64`、`windows-aarch64`、`linux-x86_64`、`linux-aarch64` 任何一个就直接失败，不发出不完整的清单），再生成 `SHA256SUMS.txt` 并创建草稿 Release。同一标签重跑会替换自己上一次的草稿，已发布的 Release 绝不覆盖。

改动 Linux 打包或 runner 后，先通过手动 Linux AppImage 工作流验证分支上的无签名包。在 main 上手动触发 Release 工作流，核对全部构建及签名产物后再打标签。`release` 环境只允许 main 和 `v*` 标签。

## 7. 出问题时

- **构建失败**：修复走新的 PR。签名失败时会报 `TAURI_SIGNING_PRIVATE_KEY` 之类的错误：检查 `release` 环境的 secrets 是否还在，以及 `…_PASSWORD` 是否与私钥的密码一致。私钥丢失后，已安装用户无法再自动更新，只能发一个内置新公钥的过渡版本。删除标签后在新的 main 提交上重打同名标签：`git push origin :refs/tags/vX.Y.Z && git tag -d vX.Y.Z`，再按第 6 步重新打标签。草稿 Release 会被替换。
- **想改发布说明并重新生成**：改 `CHANGELOG.md` 走 PR 合并，然后同上删标签重打（版本号未发布前可以这样做）。
- **已发布的版本有严重问题**：发一个新的补丁版，**不要复用版本号**，也不要移动已发布的标签。必要时把有问题的 Release 改回草稿或删除，让 `latest` 回退到上一个版本。

## 8. 更新器签名

- 密钥是 Tauri 的 minisign 密钥对。公钥（ID `B0DE08C9ED9EA8E9`）写在 `desktop/tauri.conf.json` 的 `plugins.updater.pubkey`，随应用发布。
- 私钥与密码只放在 GitHub **`release` 环境**的 secrets：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥由维护者离线保管。
- 手动触发（`Run workflow`）的构建同样会进入 `release` 环境并拿到私钥：打开 `createUpdaterArtifacts` 后，没有私钥的构建会失败，所以试跑也离不开这个环境。
- 仓库的 `release` 环境已将部署分支 / 标签限制为 **`v*` 标签和 `main` 分支**。从 `main` 手动试跑可以照常进行，其他分支触发会被环境拒绝。这是 GitHub 环境设置，工作流文件里不配置。
- 第一次接入时先用手动触发（`Run workflow`）跑一遍，确认各平台的 `.sig` 都生成；手动触发只构建并上传本次运行的产物，不建 Release（`latest.json` 在 release 作业里生成，只有标签触发才会运行）。
- 私钥或密码丢失：已安装用户无法再自动更新，只能手动安装一个内置新公钥的版本。更换公钥同理，需要过渡版本。

## 9. macOS 代码签名与公证

macOS 安装包用 Apple Developer ID 签名并公证：Tauri 在构建时对 `.app`、随包的 `mailx-mcp` 和 `.dmg` 签名，并公证 `.app`、把票据装订（staple）到上面；Tauri 不公证 `.dmg`，所以工作流的 `Notarize the disk image` 步骤再用同一把 API 密钥公证 `.dmg` 并装订票据。用户下载的是 `.dmg`，它也带票据时，首次打开不需要联网向 Apple 查询。签名与更新器签名是两件事，见上一节；本文只说明操作系统这一层。

- **凭据**只放在 GitHub **`release` 环境**的 secrets，只有 macOS 构建任务读取：

  | Secret | 内容 |
  | --- | --- |
  | `APPLE_CERTIFICATE` | Developer ID Application 证书与私钥导出的 `.p12`，base64 编码 |
  | `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 时设置的密码 |
  | `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <姓名> (<TEAMID>)` |
  | `APPLE_API_ISSUER` | App Store Connect API 密钥的 Issuer ID |
  | `APPLE_API_KEY` | API 密钥的 Key ID（10 位） |
  | `APPLE_API_PRIVATE_KEY` | `AuthKey_<KeyID>.p8` 文件内容 |

  工作流把 `.p8` 写到 runner 的临时目录并通过 `APPLE_API_KEY_PATH` 指定，文件权限为仅当前用户可读，公证结束后随 runner 销毁。

- **证书有效期**：Developer ID Application 证书约 13 个月到期，到期前需要重新签发并替换前三个 secrets。过期不影响已经签名并公证的版本（签名带安全时间戳），Team ID 不变，用户侧不会感知；但必须在上一个证书过期前完成替换，否则构建会失败。
- **构建后校验**：`release.yml` 的 `Verify the macOS signature and notarization` 步骤要求 `.app` 通过 `codesign --verify --strict`、带有 Developer ID 签名机构、hardened runtime（`codesign` 把它写在 `CodeDirectory` 行内的 `flags=…(runtime)`）与时间戳、通过 `spctl --assess`，内嵌的 `mailx-mcp` 带有同样的签名，`.dmg` 的签名可验证且 Gatekeeper 接受，并且 `.app` 与 `.dmg` 的票据都能通过 `stapler validate`。任何一项不满足即失败，并在日志里写明是哪一项，不发出未签名或公证失败的包。
- **公证等待时间**：公证要排在 Apple 的队列里，耗时不受本仓库控制。首次签名构建在构建约 15 分钟后，又在公证上等了 45 分钟以上，因此构建作业的 `timeout-minutes` 是 90 而不是原先的 60。作业在上限处被取消时，日志的最后一条通常是 `Notarizing …`，这时应确认 Apple 公证服务状态后重跑，而不是当成签名失败处理。
未签名阶段的安装版每次构建都被钥匙串当作新的应用。第一个签名版本仍会让老用户遇到一次钥匙串授权（旧条目由未签名身份创建），选择“始终允许”后，之后的签名版本沿用同一身份，不再询问。发版说明与用户指南要保留这条提示，直到确认老用户都已升级。
- **验证**：签名与公证只能在真实 macOS 上确认，验证方法与通过标准见[验证指引](validation.md)。

## 10. 尚未覆盖（后续阶段）

- **代码签名**：Windows Authenticode 尚未接入（开源项目可申请 SignPath Foundation 的免费签名，或购买 OV 证书）。macOS 已签名，但已签名安装包上的钥匙串首次授权仍需在真机按上一节确认。

- **server**：并入 workspace 后与桌面端共用版本号和标签；在 `release.yml` 增加服务端二进制与容器镜像任务，镜像的 `latest` 与 `X.Y` 标签只给正式版。
