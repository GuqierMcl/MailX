# MailX 官网与用户指南

`mailxapp.com` 的源码：Astro 页面加 Starlight 文档，构建为静态文件，部署到 Cloudflare Workers。

| 位置 | 内容 |
| --- | --- |
| `src/pages/<语言>/` | 首页、下载、更新日志；每种语言一个目录，页面实现在 `src/components/` |
| `src/i18n/<语言>.ts` | 官网页面文案。面向所有人，不写协议名和术语 |
| `../docs/guides/<语言>/` | 用户指南，构建时直接读取；新增页面同时写两种语言，并加入 `src/lib/sidebar.mjs` |
| `../docs/assets/screenshots/<语言>/` | 截图，直接引用 |
| `worker/` | 给不带语言的地址（`/`、`/download/`、`/docs/…`）选择语言 |
| `public/_redirects` | 改名页面的重定向；已公开的文档地址不能失效 |

语言目录、URL、cookie 统一使用 `zh-CN` 与 `en`，定义在 `src/lib/locale.mjs`。品牌图形由 `scripts/sync-assets.mjs` 在构建前从 `branding/` 与 `frontend/public/` 复制，副本不入库。下载页在构建时读取最新的 GitHub Release，更新日志读取根目录 `CHANGELOG.md`，都不手写版本号。

## 命令（在仓库根目录运行）

- `bun run site:dev`：本地预览。
- `bun run site:check`：类型检查与测试。
- `bun run site:build`：构建到 `website/dist`。
- `bun run site:preview`：构建后用 Workers 运行时预览，包含语言跳转。

PR 由 `.github/workflows/ci.yml` 的 `Website` 任务检查和构建；合并到 `main` 或发布 Release 后由 `.github/workflows/website.yml` 部署。部署需要仓库 secrets `CLOUDFLARE_API_TOKEN` 与 `CLOUDFLARE_ACCOUNT_ID`。
