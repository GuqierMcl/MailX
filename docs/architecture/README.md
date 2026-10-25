# 架构文档

状态：**部分 Implemented / 其余 Initial draft** · 账号、收信、隔离阅读、富文本发信与首版中英文文本已有实现，完整服务商和三平台体验继续验证；这些方案可由后续设计修订或替换。

| 文档 | 主要问题 |
| --- | --- |
| [总览](overview.md) | 模块如何划分，代码放在哪里，依赖指向哪里 |
| [账号配置](account-configuration.md) | 账号、认证、路由、凭证恢复和操作生命周期 |
| [数据库迁移与启动](database-migrations.md) | Implemented：版本化迁移、v1 兼容和业务启动准入 |
| [同步与发信](mail-sync.md) | 账号生命周期、增量同步、断网恢复与防重复发送 |
| [存储与搜索](storage-search.md) | 数据身份、表结构、缓存、全文索引和一致性备份 |
| [网络与安全](network-security.md) | 代理、OAuth、密钥、HTML、日志和外部数据访问 |
| [远程图片调研与优化](remote-image-loading.md) | Implemented：外部项目证据、图片兼容、代理信任策略；真实代理矩阵待补 |
| [桌面与 Web](desktop-web.md) | 两种宿主、前端数据流、进程生命周期和部署 |
| [界面设计语言](ui-design.md) | 画布与浮动岛屿、无分割线、自有外壳组件、标题栏插槽与命令面板 |
| [中英文文本适配](internationalization.md) | Implemented：界面、错误、语言设置、多窗口同步及 Rust 文本；三平台体验待验，区域格式化 Planned |

接口结构以[应用契约](../contracts/app-api.md)与 [MCP 契约](../contracts/mcp.md)为入口；模型执行另见 [AI Runtime](../ai-runtime/README.md)。
