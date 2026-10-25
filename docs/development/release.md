# 发版流程

状态：**脚本与工作流已就绪，尚未做过真实发布** · 本页记录当前约定；工作流在第一次真实发版时才算验证过，遇到问题直接修订本页。安装包目前**未做代码签名**，也**没有应用内自动更新**。

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

**第一次发布**：仓库当前版本是 `0.1.0`（从未发布），而 `0.1.0-alpha.1` 在 SemVer 里低于它，所以要写 `bun run version:set 0.1.0-alpha.1 --force`。

## 3. 更新日志

`CHANGELOG.md` 采用 Keep a Changelog 格式，是发布说明的唯一来源。

- 面向用户的改动，在 PR 里把一行写到 `## [Unreleased]` 之下，分到 `### Feature`（新功能）、`### Optimization`（行为不变的改进）、`### Fixed`（缺陷修复）、`### Security`（HTML 隔离、凭证、权限、代理等加固）、`### Changed`（不兼容的行为、数据格式、配置变化与移除的功能，含数据迁移）之一；没有内容的分类不写。每条以 `- ` 开头，脚本只认这个前缀，不校验分类名。
- 条目写给用户看，一行一句；测试、CI、重构、文档等内部改动不写。结尾带 PR 的完整链接，例如 `([#25](https://github.com/GuqierMcl/MailX/pull/25))`：`#25` 这种简写在 Release 页面里会被 GitHub 自动链接，但在仓库的 `.md` 文件里不会，完整链接两处都可点击。
- **预发布版**不新增小节，发布说明取自 `[Unreleased]`，并自动加上“预发布版本”提示。
- **正式版**：`version:bump` / `version:set` 把 `[Unreleased]` 归并为 `## [x.y.z] - 日期`、留下新的空 `[Unreleased]` 并更新底部对比链接。`[Unreleased]` 为空会被拒绝；确需跳过用 `--no-changelog`。
- 发布说明末尾追加 `.github/release-install-notes.md`（未签名安装提示）。签名落地后在那里更新或删除。
- `bun run version:check --release` 检查日志是否可发布；`bun run release:notes [版本]` 可本地预览将要发布的说明。

## 4. 每次发版的步骤

1. **确认 main 的 CI 是绿的**，`CHANGELOG.md` 的 `[Unreleased]` 已写全。
2. **新建发版分支**：`git switch main && git pull --ff-only && git switch -c release/vX.Y.Z`。
3. **改版本**：先 `bun run version:bump … --dry-run` 看结果，再去掉 `--dry-run` 执行。
4. **检查**：`git diff`（应只有版本文件和 `CHANGELOG.md`）；`bun run check`、`bun run rust:test`（也可交给 PR 的 CI）。
5. **提交并开 PR**：`git commit -am "chore(release): vX.Y.Z"`，推送、等 CI 通过、合并。
6. **打标签**（必须是合并后的 main，不是 PR 分支）：
   ```sh
   git switch main && git pull --ff-only
   bun run version:check --release     # 仍然通过
   git tag -a vX.Y.Z -m "MailX X.Y.Z"
   git push origin vX.Y.Z
   ```
7. **等待构建**：推标签触发 `Release` 工作流（Actions 页）。依次是校验版本、四平台构建、全部成功后创建**草稿** Release。
8. **验证草稿**：在仓库 `Releases` 页找到带 `Draft` 标记的条目，下载各平台安装包，至少在 macOS 和 Windows 各装一次，走登录、同步、阅读的冒烟；核对文件名中的版本号与 `SHA256SUMS.txt`。
9. **（可选）修改说明**：草稿的 `Edit`（铅笔）页可改正文，但只改 GitHub 上那份，不会回写 `CHANGELOG.md`。有价值的改动请同步进日志；想让正文重新生成，见第 6 节。
10. **发布**：草稿的 `Edit` 页底部点 **Publish release**（命令行：`gh release edit vX.Y.Z --draft=false`）。预发布版会自动带 `Pre-release` 标记，发布前确认勾选。

发版后**不需要**再把版本改成下一个开发版本；`main` 保持在最近发布的版本号，下次发版再改。

## 5. 工作流做了什么

`.github/workflows/release.yml`：

- **触发**：推送 `v*` 标签；或在 Actions 页手动 `Run workflow`（只构建、产物留在该次运行里 7 天，不创建 Release，用来调试工作流）。
- **verify**：标签必须等于仓库版本（`v` + `package.json` 的 `version`）；`CHANGELOG.md` 必须可发布；标签指向的提交必须是 `main` 的祖先。含 `-` 的版本标为预发布。
- **build**：macOS arm64（`macos-15`）、macOS x64（`macos-15-intel`）、Windows x64、Linux x64。`bun run tauri build` 生成 `.dmg`、`-setup.exe`、`.deb`、`.rpm`、`.AppImage`。任一平台失败即整体失败。
- **release**：仅标签触发，全部平台成功后，生成 `SHA256SUMS.txt` 并创建草稿 Release。同一标签重跑会替换自己上一次的草稿，已发布的 Release 绝不覆盖。

首次真实运行前，建议先在 Actions 页对分支手动触发一次，确认四个平台都能出包；`macos-15-intel` 之类的 runner 标签若已下线需要改。

## 6. 出问题时

- **构建失败**：修复走新的 PR。删除标签后在新的 main 提交上重打同名标签：`git push origin :refs/tags/vX.Y.Z && git tag -d vX.Y.Z`，再按第 6 步重新打标签。草稿 Release 会被替换。
- **想改发布说明并重新生成**：改 `CHANGELOG.md` 走 PR 合并，然后同上删标签重打（版本号未发布前可以这样做）。
- **已发布的版本有严重问题**：发一个新的补丁版，**不要复用版本号**，也不要移动已发布的标签。必要时把有问题的 Release 改回草稿或删除，让 `latest` 回退到上一个版本。

## 7. 尚未覆盖（后续阶段）

- **应用内自动更新**：需要 Tauri 更新器签名密钥（`bun run tauri signer generate`）、`tauri-plugin-updater`、`latest.json`。私钥只放在 GitHub Secrets 和离线备份，**丢失后已安装的用户无法再自动更新**；公钥必须随应用发布，没内置公钥的版本以后也不会自动更新。
- **代码签名**：macOS Developer ID 加公证、Windows 签名。macOS 钥匙串对不同签名身份的访问提示可能影响已保存凭证的体验，正式面向用户前需要在真机验证。
- **server**：并入 workspace 后与桌面端共用版本号和标签；在 `release.yml` 增加服务端二进制与容器镜像任务，镜像的 `latest` 与 `X.Y` 标签只给正式版。
