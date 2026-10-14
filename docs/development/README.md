# 开发落地建议

状态：**Initial draft**。当前可直接编辑的是文档，Rust / Tauri / 前端工程尚未创建。以下是建议的启动顺序，可随后续设计调整。

## 1. 当前仓库

本轮建立 Git、基础忽略 / 换行 / 编辑器规则、简要 README 和初始设计。没有 Cargo workspace、package.json、应用 CI、运行中的 API 或构建产物，因此现在不能运行 `cargo test`、`bun run dev` 或 Docker 示例。

当前文档修改可执行 `git diff --check`，并检查链接、状态用语和敏感信息。未跟踪的新文件也需单独检查；该命令本身不会检查未跟踪文件。

## 2. M0 建立真实工程

1. 按[目标布局](../architecture/overview.md)创建 Cargo workspace，先建立 core / infra；桌面壳随后装配，不提前创建所有未来目录。
2. 固定一个经过 Tauri 与依赖验证的 Rust 工具链、MSRV 和 Bun 版本；提交应用锁文件，并记录平台依赖。
3. 建立 React / TypeScript / Vite 工作台和 Tauri 宿主，先验证空窗口、IPC、退出和数据目录锁。
4. 编写最小协议探针，验证 IMAP 与 SMTP 在 Direct / SOCKS5 / HTTP CONNECT 下的 TLS、认证与收发；证明代理失败不会直连。
5. 建立 SQLite 执行器、第一条迁移和 FTS5 探针；选定 DTO 生成工具，跑通一个“列表请求 → Rust → TS”的闭环。
6. 为已存在工程配置真实 CI；在 README 中新增已经验证过的安装、开发与测试命令。

系统依赖按 Tauri 官方前置条件核验，包括 Windows WebView2 / 构建工具、macOS 工具链和 Linux WebKitGTK 相关依赖；不要照抄其他项目某一平台的安装命令。[Tauri 前置条件](https://v2.tauri.app/start/prerequisites/)。

## 3. 第一条纵向链路

M1 的最小演示建议是：配置一个受控测试邮箱 → 验证 TLS → 同步最近 5 封头部 → SQLite 去重入库 → 重启后列出 → 显示一封缓存正文。然后增加第二个不同路由账号，验证两个账号互不影响。

在这条链路可恢复之前，不并行铺开复杂收件箱动效、通用 Agent 框架或 Web 登录系统。此顺序是为尽早发现协议与持久化风险，后续可以根据实际证据重新安排。

## 4. 增加邮箱预设

预设候选格式包含：providerId、匹配域名、IMAP / SMTP 主机端口、TLS 模式、认证方法、用户名模板、开通说明、官方来源、核验日期和已验证能力。没有来源或测试证据时标记 experimental。

读取可信内置预设后允许用户查看、修改并测试，不能仅凭邮箱域名自动连接任意远程发现结果。测试覆盖收信与发信两个方向、失败提示和网络路由。连接配置、OAuth 注册信息和账号授权在不同层管理，真实凭证不得放进资源文件。

## 5. 建议工程习惯

- Rust domain / service 单元测试用端口替身；协议适配用可故障注入的真实协议测试环境。
- 每个任务显式携带取消与账号范围；统一超时、路由和结构化错误，不由 UI 分散补救。
- 跨边界 DTO 从同一个来源生成；不长期维护两套手写 Rust / TypeScript 定义。
- SQL 通过版本化迁移演进，已发布迁移的 checksum 不改写；未发布草案可在团队确认后重整。
- 更新设计时写明哪些是最新选择、哪些是已实现事实，删除失效示例；历史上有价值的取舍保留在 ADR。

详细验收方法见[验证策略](validation.md)，任务依赖见[路线图](../roadmap/README.md)。
