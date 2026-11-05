# MailX 文档入口

状态：**桌面收发、本地全文搜索与本地 MCP 已有实现；内置 AI、OAuth 与服务器 Web 宿主为后续规划**。此目录记录产品行为、架构边界、开发流程和设计依据，接口与技术选型仍可演化。平台、服务商、客户端的具体测试结果统一记录在[验证策略](development/validation.md)，发布状态以 [CHANGELOG](../CHANGELOG.md) 与 [GitHub Releases](https://github.com/GuqierMcl/MailX/releases) 为准。

**演化约定：** 后续可以出现新的设计，技术栈、模块边界、产品范围、接口和里程碑都允许调整或替换。文中的“采用”“必须”“不允许”描述的是该草案内部为保证一致性所需的条件，不意味着永久冻结该方案。新方案落地时更新相关文档与状态即可，无需为偏离初始设想单独取得批准。

文档沿用 NexusPilot 的按职责分层方式，内容针对 MailX 重新设计，不继承该项目的数据库业务或 AI sidecar 架构。

## 阅读路径

- 了解项目：先看[产品范围](product/README.md)，再看[架构总览](architecture/overview.md)。
- 安装与使用：阅读[用户入门](guides/README.md)（[简体中文](guides/zh-CN/README.md)），了解账号接入、本地缓存、发送、MCP 配置与已知限制。
- 开始编码：先看[开发指南](development/README.md)和[实施路线图](roadmap/README.md)，根据当前实现与验证缺口选择任务。
- 准备发布：阅读[发版流程](development/release.md)和[验证策略](development/validation.md)，核对版本、更新日志、安装包与实际平台证据。
- 理解当前账号能力：阅读[账号配置与凭证生命周期](architecture/account-configuration.md)和[应用 API](contracts/app-api.md)。
- 数据库启动与迁移：阅读[应用数据库版本化迁移与启动](architecture/database-migrations.md)，迁移覆盖账号、邮件缓存、草稿、发件箱、全文索引与邮件整理等状态；具体编号清单以 `core/src/storage/migrations.rs` 为准，启动准入和失败重试已实现。
- 桌面后台运行：阅读[关闭窗口、托盘与退出](architecture/desktop-lifecycle.md)，默认每次询问、平台差异和草稿安全退出已有实现，三平台实机证据分别记录。
- 修改界面：先看[界面设计语言](architecture/ui-design.md)，了解画布与岛屿结构、无分割线约定和外壳组件。
- 国际化适配：查看[中英文文本适配](architecture/internationalization.md)，首版已实现界面与错误文本、语言设置、多窗口同步及 Rust 通知 / 引用模板；三平台打包体验与原生通知待验，日期等区域格式化仍为 Planned。
- 实现邮箱：阅读[同步与发信](architecture/mail-sync.md)、[写信与发送设计](architecture/sending.md)、[存储与搜索](architecture/storage-search.md)、[网络与安全](architecture/network-security.md)。
- 适配服务商差异：阅读[邮箱服务商兼容层](architecture/provider-compatibility.md)，主干提供中立接口，具体服务商规则在独立模块中实现。
- 实现 AI / Agent：阅读[工具层与调用入口](architecture/agent-tools.md)、[AI Runtime](ai-runtime/README.md)、[MCP 契约](contracts/mcp.md)及[应用契约](contracts/app-api.md)。
- 查阅调研与参考资料：阅读 [reference/](reference/README.md)，具体文档和状态由该目录入口维护。
- 审查选型：阅读 [ADR](adr/README.md)。

## 目录职责

| 目录 | 内容 | 本阶段状态 |
| --- | --- | --- |
| [product/](product/README.md) | 产品目标、当前范围、兼容边界与非目标 | 当前范围 / 后续 Planned |
| [guides/](guides/README.md) | 安装、账号接入、使用与已知限制（英文为主，`zh-CN/` 为中文页）；未来官方文档站的构建源 | 当前用户指南 |
| [architecture/](architecture/README.md) | 当前包边界与后续数据、流程设计 | 部分 Implemented |
| [contracts/](contracts/README.md) | 当前桌面 IPC 与本地 MCP 契约；后续 HTTP 设计 | 部分 Implemented / 接口未冻结 |
| [ai-runtime/](ai-runtime/README.md) | 模型适配、运行生命周期和权限 | Initial draft |
| [development/](development/README.md) | 开发、发布流程与验证记录 | 自动检查可执行；平台及服务商证据单列 |
| [adr/](adr/README.md) | 选型理由、代价与重新评估条件 | Accepted / Proposed |
| [roadmap/](roadmap/README.md) | 已有能力、桌面发布准备与后续工作 | 部分 Implemented / 后续 Planned |
| [reference/](reference/README.md) | 外部调研、接入评估与设计参考资料 | Reference |

下载入口放在根 README，安装、账号接入、MCP 配置和已知限制由 `guides/` 维护，并与实际行为同步。`guides/` 是未来官方文档站的构建源：站点构建时直接读取该目录的 Markdown，不另行维护副本，因此其中只放面向用户的稳定说明，并保证页面单独渲染时链接与图片仍然有效。站点结构与构建方式尚未确定。

## 状态与维护规则

常规文档描述长期适用的能力、行为与边界，不复制当前发布版本、候选提交、核对日期或最新测试进度。版本与变更以根 `package.json`、[CHANGELOG](../CHANGELOG.md) 和 Releases 为准；工具链与依赖版本指向各自配置文件；具体测试环境、日期和结果集中写入[验证记录](development/validation.md)。历史设计依据、迁移编号与命令示例保留必要的上下文。

- **Initial draft**：项目启动时的一版可实施设想，尚无运行证据，可由后续设计替换。
- **Draft**：可随首批实现调整的契约，尚未承诺对外兼容。
- **Proposed**：初始建议，待实现验证；不表示维护者已批准或冻结。
- **Accepted**：当前讨论采用的方向；不等于全部实现，也允许随后续设计调整。
- **Implemented**：已有对应代码和验证；更新时写明代码位置与验证方法。
- **Planned / Deferred**：未来工作，不覆盖当前边界。

冲突时先区分“初始设想”“最新设计”与“实际行为”：实际能力由代码和测试证明，最新明确设计优先于启动草案。变更时更新受影响的说明，避免把被替代的方案继续标成当前设计。只有接口实际对外发布并明确承诺兼容后，才按届时确定的版本策略维护；当前草案不代表额外的兼容性承诺。

不要提交原始个人规划、AI 对话、临时工作日志、真实邮件、凭证或机器绝对路径；可复用结论写入相应权威文档。
