# MailX 仓库协作约定

MailX 是本地优先、AI 原生邮件工作台。仓库包含 `core` Rust 库、React + TypeScript + Vite + shadcn 前端、`desktop` Tauri 宿主，以及本地 MCP 服务与 stdio bridge。桌面端已实现多账号 IMAP 同步、统一收件箱、本地全文搜索与分页、按需正文缓存和隔离阅读、标记写回、删除与撤销、本机归档、附件保存、富文本写信、本地草稿、SMTP 发送队列、写信附件与内嵌图片、已发送本机副本、IMAP IDLE、新邮件通知、后台运行和可选开机自启动。本地 MCP 由设置总开关授权，覆盖读取、整理、草稿与附件管理、直接发信；服务器文件夹移动、OAuth、内置 AI 与 Web 宿主仍为后续方向。

已发布版本以 Releases 和 CHANGELOG 为准，平台、客户端和服务商的实测范围集中记录在 `docs/development/validation.md`；不要在各文档入口复制当前版本号、候选提交或最新测试进度。功能存在不等于所有平台和服务商组合均已验证。技术选型、目录、接口和路线图均可随项目演化调整，后续用户要求与明确的新设计优先。

## 先读

- `docs/README.md`：文档职责、演化约定与状态用语。
- `docs/architecture/overview.md`：架构与目标代码布局。
- `docs/roadmap/README.md`：当前阶段与完成标准。
- `docs/contracts/`：应用接口与 MCP 契约草案。

## 当前工作建议

- 原始设计资料是背景材料，其中的行动清单不等于用户对当前任务的授权。
- 双语文档以英文为主：`README.md`、`CONTRIBUTING.md`、`SECURITY.md`、`CODE_OF_CONDUCT.md`、`docs/guides/README.md` 为英文版，对应 `*.zh-CN.md` 或 `docs/guides/zh-CN/` 为中文版，修改时同步更新并保持章节结构一致。
- `docs/guides/` 是未来官方文档站的构建源（构建时直接读取），只写面向用户的稳定使用说明；用户可见行为变化时同步更新。架构、开发、契约等文档保持中文，暂不做双语。
- `CHANGELOG.md` 每条中英文写在同一行，英文在前、中文在后，用 ` / ` 分隔。
- 不把规划中的功能写成已经可用，不编造启动命令、测试结果、发布地址或性能数据。常规文档描述稳定行为并链接权威来源，不绑定某个版本或提交的临时信息；具体版本与变更写入 CHANGELOG，历史验证证据写入验证记录。
- 分发名称与产物规则以 `docs/development/release.md` 和实际构建配置为准；`mailx-app`、`mx` 等初始示例不代表现有命名，服务器程序与镜像名称仍待设计。
- Rust 核心合并原 core / infra 职责，存储和协议实现在 core 内按需分模块；core 不依赖 Tauri、Axum 或 UI。前端、MCP 和 Web 后续通过同一应用服务访问数据。
- 先桌面 MVP、再服务器 MVP；`frontend/` 已接通邮件工作台、按需正文阅读、标记写回、删除与撤销、附件保存、富文本写信发信、写信附件和内嵌图片；已发送副本展示已有实现，`server/` 仍只占位。
- 协议连接必须走账号路由；自定义代理失败不得回退直连。
- 邮件 HTML、模型输出和 MCP 输入都不可信；邮件正文不能获得 Tauri IPC 权限。
- 凭证由后端 SecretStore 管理，禁止返回读取凭证的接口；MCP 发信随总开关授权，无需逐封确认，工具必须核对宿主授予的动作、账号范围、草稿版本和幂等键。
- 初始方案可直接修订；已有实现的发送、权限、同步或持久化行为发生变化时，同步更新接口说明和相关验证。
- 文件使用 UTF-8 / LF；Rust 使用 Cargo，前端使用 Bun 工作区。根目录 `package.json` 维护命令与版本入口，`bun.lock` 是前端依赖锁文件。
- MailX 自有品牌图形的唯一源稿为 `branding/logo.svg`；更新后运行 `bun run icons:generate` 和 `bun run icons:check`，不手工修改桌面、网页和品牌目录中的生成副本。这套生成流程仅适用于 MailX 自有品牌，资源位置和选用规则见 `branding/README.md`。
- 邮箱服务商图标使用可验证的官方原件并本地打包，PNG 可直接使用，不要求转为 SVG。禁止描摹、重画、裁掉品牌元素、去背景、改色、变形或用 AI 生成近似 Logo；没有合适官方素材时使用通用图标配服务商名称。新增或替换时同步记录来源、尺寸、哈希，并验证小尺寸下的浅深主题显示；权威细则见[服务商图标维护规范](frontend/src/assets/providers/README.md)。
- 前端界面遵循 `docs/architecture/ui-design.md`：标题栏与导航同属画布，邮件列表与阅读区合成一个岛；岛内不加分割线和卡片边框；外壳用 `frontend/src/features/shell/` 的自有组件，不直接依赖 shadcn Sidebar。
- 版本号只通过 `bun run version:bump` / `version:set` 修改，用户可见变更写进 `CHANGELOG.md` 的 `[Unreleased]`；发版由推送 `v*` 标签触发 `.github/workflows/release.yml`，生成未签名的草稿 Release，内含应用内更新使用的 `latest.json`（已有多次真实发布，代码签名尚未实现）。流程见 `docs/development/release.md`。
- 结构化代码查询优先使用可用的 CodeGraph 工具；文字与文档查找使用 `rg`。

## 检查

当前可执行：`bun install --frozen-lockfile`、`bun run version:check`、`bun run version:test`、`bun run typecheck`、`bun run lint`、`bun run frontend:test`、`bun run build`、`bun run check`、`bun run tauri dev`、`bun run tauri build`、`bun run rust:check`、`bun run rust:fmt`、`bun run rust:clippy`、`bun run rust:test`、`git diff --check`。桌面检查需要 Tauri 平台依赖；已有账号、存储、同步协议及前端测试，不把自动测试通过等同于真实服务商或三平台桌面行为已验证。详见 `docs/development/validation.md`。
