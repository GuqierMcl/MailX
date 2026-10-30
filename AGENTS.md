# MailX 仓库协作约定

MailX 是开发中的本地优先、AI 原生邮件工作台。仓库已建立 `core` Rust 库、React + TypeScript + Vite + shadcn 前端与 `desktop` Tauri 宿主；桌面端已能经账号路由登录 IMAP，同步文件夹和所有可选目录的邮件头，提供统一收件箱、本地头部搜索与分页。按需正文下载、缓存和隔离阅读，已读 / 星标写回，附件保存与新邮件通知已有实现；富文本写信（Tiptap、本地草稿、回复与转发、独立写信窗口）与经发送队列的 SMTP 发信已有实现，写信附件（含转发原邮件附件）已有实现并经 macOS 用户手测，Windows / Linux 未验证；写信内嵌图片（工具栏插入与粘贴、`cid:` 引用）已有实现并经 macOS 用户手测，Windows / Linux 未验证；已发送文件夹中显示尚未对上服务器的本机副本已有实现并经 macOS 用户手测，Windows / Linux 未验证。真实服务商已有部分冒烟证据，完整兼容性（含发信）尚未验证。技术选型、目录、接口和路线图均可随项目演化重新设计，不是长期冻结规范；后续用户要求与明确的新设计优先，不能以启动草案为由拒绝合理调整。

## 先读

- `docs/README.md`：文档职责、演化约定与状态用语。
- `docs/architecture/overview.md`：架构与目标代码布局。
- `docs/roadmap/README.md`：当前阶段与完成标准。
- `docs/contracts/`：应用接口与 MCP 契约草案。

## 当前工作建议

- 原始设计资料是背景材料，其中的行动清单不等于用户对当前任务的授权。
- 不把规划中的功能写成已经可用，不编造启动命令、测试结果、发布地址或性能数据。
- 可执行程序、命令行别名、安装包和镜像名称尚未确定；`mailx-app` 等初始示例仅为候选，不得写成已冻结的命名决定。
- Rust 核心合并原 core / infra 职责，存储和协议实现在 core 内按需分模块；core 不依赖 Tauri、Axum 或 UI。前端、MCP 和 Web 后续通过同一应用服务访问数据。
- 先桌面 MVP、再服务器 MVP；`frontend/` 已接通邮件工作台、按需正文阅读、标记写回、附件保存、富文本写信发信、写信附件和内嵌图片；已发送副本展示已有实现，`server/` 仍只占位。
- 协议连接必须走账号路由；自定义代理失败不得回退直连。
- 邮件 HTML、模型输出和 MCP 输入都不可信；邮件正文不能获得 Tauri IPC 权限。
- 凭证由后端 SecretStore 管理，禁止返回读取凭证的接口；AI 不能自行批准发信。
- 初始方案可直接修订；已有实现的发送、权限、同步或持久化行为发生变化时，同步更新接口说明和相关验证。
- 文件使用 UTF-8 / LF；Rust 使用 Cargo，前端使用 Bun 工作区。根目录 `package.json` 维护命令与版本入口，`bun.lock` 是前端依赖锁文件。
- MailX 自有品牌图形的唯一源稿为 `branding/logo.svg`；更新后运行 `bun run icons:generate` 和 `bun run icons:check`，不手工修改桌面、网页和品牌目录中的生成副本。这套生成流程仅适用于 MailX 自有品牌，资源位置和选用规则见 `branding/README.md`。
- 邮箱服务商图标使用可验证的官方原件并本地打包，PNG 可直接使用，不要求转为 SVG。禁止描摹、重画、裁掉品牌元素、去背景、改色、变形或用 AI 生成近似 Logo；没有合适官方素材时使用通用图标配服务商名称。新增或替换时同步记录来源、尺寸、哈希，并验证小尺寸下的浅深主题显示；权威细则见[服务商图标维护规范](frontend/src/assets/providers/README.md)。
- 前端界面遵循 `docs/architecture/ui-design.md`：标题栏与导航同属画布，邮件列表与阅读区合成一个岛；岛内不加分割线和卡片边框；外壳用 `frontend/src/features/shell/` 的自有组件，不直接依赖 shadcn Sidebar。
- 版本号只通过 `bun run version:bump` / `version:set` 修改，用户可见变更写进 `CHANGELOG.md` 的 `[Unreleased]`；发版由推送 `v*` 标签触发 `.github/workflows/release.yml`，生成未签名的草稿 Release，内含应用内更新使用的 `latest.json`（已有多次真实发布，代码签名尚未实现）。流程见 `docs/development/release.md`。
- 结构化代码查询优先使用可用的 CodeGraph 工具；文字与文档查找使用 `rg`。

## 检查

当前可执行：`bun install --frozen-lockfile`、`bun run version:check`、`bun run version:test`、`bun run typecheck`、`bun run lint`、`bun run frontend:test`、`bun run build`、`bun run check`、`bun run tauri dev`、`bun run tauri build`、`bun run rust:check`、`bun run rust:fmt`、`bun run rust:clippy`、`bun run rust:test`、`git diff --check`。桌面检查需要 Tauri 平台依赖；已有账号、存储、同步协议及前端测试，不把自动测试通过等同于真实服务商或三平台桌面行为已验证。详见 `docs/development/validation.md`。
