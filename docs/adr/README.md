# 初始架构选型记录

状态：**Proposed**。这里使用 ADR 的结构保存启动阶段的判断与取舍，不代表已批准、已实现或不可更改的长期决策。

| 记录 | 初始建议 | 状态 |
| --- | --- | --- |
| [0001](0001-shared-core-and-hosts.md) | 共享 Rust 核心、桌面先行、Web 随后 | Proposed |
| [0002](0002-stack-and-storage.md) | React / Bun、SQLite 与有限协议范围 | Proposed |
| [0003](0003-ai-and-agent-boundaries.md) | 内置 Rust AI 与受限 MCP Server | Proposed |

这些记录可以在早期直接修订。若后续已有实现或公开讨论需要保留历史，再将旧记录标记 Superseded 并链接新方案；不要求每次探索都创建新 ADR，也不因偏离启动愿景额外设置审批。实际实现状态在相应架构文档维护。
