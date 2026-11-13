# 架构文档

状态：**桌面邮件与本地 MCP 已有实现 / 内置 AI 与服务器宿主为 Planned** · 账号、同步与 IDLE、隔离阅读、本地全文搜索、整理邮件、富文本发信、附件与内嵌图片、邮件翻译、应用内更新及中英文界面已有实现。各功能的真实服务商与平台证据见[验证记录](../development/validation.md)；这些方案可由后续设计修订或替换。

| 文档 | 主要问题 |
| --- | --- |
| [总览](overview.md) | 模块如何划分，代码放在哪里，依赖指向哪里 |
| [账号配置](account-configuration.md) | 账号、认证、路由、凭证恢复和操作生命周期 |
| [凭据保险库](secret-vault.md) | macOS 上把所有密码合并为单个系统凭据条目（已实现，其他平台不变） |
| [应用内更新](updater.md) | 检查、下载、验签安装、退出时更新与发版规则 |
| [桌面生命周期](desktop-lifecycle.md) | 关窗、后台与托盘、安全退出、登录自动启动与静默启动 |
| [数据库迁移与启动](database-migrations.md) | Implemented：版本化迁移、v1 兼容和业务启动准入 |
| [同步与发信](mail-sync.md) | 账号生命周期、增量同步、断网恢复与防重复发送 |
| [验证码识别](verification-codes.md) | core 本地规则识别、歧义结果、MIME 预算与开发探测；通知接入为后续阶段 |
| [邮箱服务商兼容层](provider-compatibility.md) | 主干接口与服务商模块边界、服务商识别、已发送副本身份与对账 |
| [写信与发送](sending.md) | 富文本草稿、写信窗口、附件与内嵌图片、SMTP 队列与已发送副本 |
| [存储与搜索](storage-search.md) | 数据身份、表结构、缓存、全文索引和一致性备份 |
| [网络与安全](network-security.md) | 代理、OAuth、密钥、HTML、日志和外部数据访问 |
| [远程图片调研与优化](remote-image-loading.md) | Implemented：外部项目证据、图片兼容、代理信任策略；真实代理矩阵待补 |
| [工具层与调用入口](agent-tools.md) | 共享工具、授权和结果整形；本地 MCP 已接入，内置 Agent 与远程 MCP 待接入 |
| [整理邮件工具](mail-organize-tools.md) | MCP 标记、归档、同步刷新与打开草稿，查询增强、事件和精确撤销 |
| [发送与附件工具](mail-send-tools.md) | MCP 直接发信、发送结果与取消、草稿附件增删 |
| [桌面与 Web](desktop-web.md) | 两种宿主、前端数据流、进程生命周期和部署 |
| [界面设计语言](ui-design.md) | 画布与浮动岛屿、无分割线、自有外壳组件、标题栏插槽与命令面板 |
| [邮件翻译](translation.md) | Implemented（仅自动测试）：打开邮件后经用户选定的翻译服务保留版式翻译，数据披露与同意、分段回写的安全不变量、专用网络路由、本机缓存与限额 |
| [AI 服务商与模型配置](ai-providers.md) | Implemented（仅自动测试）：连接模型服务商、内置服务商表、models.dev 目录、模型列表与测试调用、图标和密钥保存；尚无功能使用模型 |
| [中英文文本适配](internationalization.md) | Implemented：界面、错误、语言设置、多窗口同步及 Rust 文本；三平台体验待验，区域格式化 Planned |

接口结构以[应用契约](../contracts/app-api.md)与 [MCP 契约](../contracts/mcp.md)为入口；模型执行另见 [AI Runtime](../ai-runtime/README.md)，两者共用的工具层见[工具层与调用入口](agent-tools.md)。
