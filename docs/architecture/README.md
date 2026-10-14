# 架构文档

状态：**Initial draft** · 尚未实现；这些方案可由后续设计修订或替换。

| 文档 | 主要问题 |
| --- | --- |
| [总览](overview.md) | 模块如何划分，代码放在哪里，依赖指向哪里 |
| [同步与发信](mail-sync.md) | 账号生命周期、增量同步、断网恢复与防重复发送 |
| [存储与搜索](storage-search.md) | 数据身份、表结构、缓存、全文索引和一致性备份 |
| [网络与安全](network-security.md) | 代理、OAuth、密钥、HTML、日志和外部数据访问 |
| [桌面与 Web](desktop-web.md) | 两种宿主、前端数据流、进程生命周期和部署 |

接口结构以[应用契约](../contracts/app-api.md)与 [MCP 契约](../contracts/mcp.md)为入口；模型执行另见 [AI Runtime](../ai-runtime/README.md)。
