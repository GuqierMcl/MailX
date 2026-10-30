# MailX 文档入口

状态：**桌面端收信、阅读、富文本发信与首版中英文文本已有实现，邮件能力继续演进** · 2026-10-25。此目录记录产品愿景、当前架构和实施建议，不是已完成邮件客户端的使用手册，也不是长期冻结规范。Rust 核心经账号路由登录 IMAP，将所有可选文件夹的邮件头存入 SQLite，Tauri IPC 与 shadcn 工作台提供统一列表、分页和头部搜索；按需正文缓存和安全阅读、已读 / 星标写回、附件保存与新邮件通知已有实现；富文本写信、写信附件与经发送队列的 SMTP 发信已有实现，写信内嵌图片已有实现并经 macOS 用户手测，已发送副本展示尚未实现。实现代码与真实服务商兼容性分别记录，已有部分真实账号冒烟证据，完整矩阵仍待验证。

**演化约定：** 后续可以出现新的设计，技术栈、模块边界、产品范围、接口和里程碑都允许调整或替换。文中的“采用”“必须”“不允许”描述的是该草案内部为保证一致性所需的条件，不意味着永久冻结该方案。新方案落地时更新相关文档与状态即可，无需为偏离初始设想单独取得批准。

文档沿用 NexusPilot 的按职责分层方式，内容针对 MailX 重新设计，不继承该项目的数据库业务或 AI sidecar 架构。

## 阅读路径

- 了解项目：先看[产品范围](product/README.md)，再看[架构总览](architecture/overview.md)。
- 开始编码：先看[开发指南](development/README.md)和[实施路线图](roadmap/README.md)，按 M0 → M1 建立第一条可验证链路。
- 理解当前账号能力：阅读[账号配置与凭证生命周期](architecture/account-configuration.md)和[应用 API](contracts/app-api.md)。
- 数据库启动与迁移：阅读[应用数据库版本化迁移与启动](architecture/database-migrations.md)，当前编号迁移覆盖 v1 账号、v2 文件夹、v3 邮件头与 v4 正文缓存；启动准入和失败重试已实现。
- 桌面后台运行：阅读[关闭窗口、托盘与退出](architecture/desktop-lifecycle.md)，默认每次询问、平台差异和草稿安全退出已有实现，三平台实机证据分别记录。
- 修改界面：先看[界面设计语言](architecture/ui-design.md)，了解画布与岛屿结构、无分割线约定和外壳组件。
- 国际化适配：查看[中英文文本适配](architecture/internationalization.md)，首版已实现界面与错误文本、语言设置、多窗口同步及 Rust 通知 / 引用模板；三平台打包体验与原生通知待验，日期等区域格式化仍为 Planned。
- 实现邮箱：阅读[同步与发信](architecture/mail-sync.md)、[写信与发送设计](architecture/sending.md)、[存储与搜索](architecture/storage-search.md)、[网络与安全](architecture/network-security.md)。
- 实现 AI / Agent：阅读 [AI Runtime](ai-runtime/README.md)、[MCP 契约](contracts/mcp.md)及[应用契约](contracts/app-api.md)。
- 查阅调研与参考资料：阅读 [reference/](reference/README.md)，具体文档和状态由该目录入口维护。
- 审查选型：阅读 [ADR](adr/README.md)。

## 目录职责

| 目录 | 内容 | 本阶段状态 |
| --- | --- | --- |
| [product/](product/README.md) | 初始产品目标、兼容范围、非目标 | Initial draft |
| [architecture/](architecture/README.md) | 当前包边界与后续数据、流程设计 | 部分 Implemented |
| [contracts/](contracts/README.md) | 当前账号、同步与邮件读取 IPC；后续 HTTP / MCP 契约 | 部分 Implemented / 其余 Draft |
| [ai-runtime/](ai-runtime/README.md) | 模型适配、运行生命周期和权限 | Initial draft |
| [development/](development/README.md) | 当前开发命令与后续验证计划 | 账号、邮件头与宿主检查可执行；平台及服务商证据单列 |
| [adr/](adr/README.md) | 选型理由、代价与重新评估条件 | Accepted / Proposed |
| [roadmap/](roadmap/README.md) | 已有基础、后续工作及验收标准 | 部分完成 / M0 未完成 |
| [reference/](reference/README.md) | 外部调研、接入评估与设计参考资料 | Reference |

有可用客户端后再建立 `guides/` 用户手册；需要文档站时以它为唯一用户文档来源。本阶段不创建空站点或第二套 Markdown 副本。

## 状态与维护规则

- **Initial draft**：项目启动时的一版可实施设想，尚无运行证据，可由后续设计替换。
- **Draft**：可随首批实现调整的契约，尚未承诺对外兼容。
- **Proposed**：初始建议，待实现验证；不表示维护者已批准或冻结。
- **Accepted**：当前讨论采用的方向；不等于全部实现，也允许随后续设计调整。
- **Implemented**：已有对应代码和验证；更新时写明代码位置与验证方法。
- **Planned / Deferred**：未来工作，不覆盖当前边界。

冲突时先区分“初始设想”“最新设计”与“实际行为”：实际能力由代码和测试证明，最新明确设计优先于启动草案。变更时更新受影响的说明，避免把被替代的方案继续标成当前设计。只有接口实际对外发布并明确承诺兼容后，才按届时确定的版本策略维护；本轮没有冻结任何接口。

不要提交原始个人规划、AI 对话、临时工作日志、真实邮件、凭证或机器绝对路径；可复用结论写入相应权威文档。
