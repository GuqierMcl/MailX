# 初始架构选型记录

状态：**Accepted / Proposed 并存**。这里使用 ADR 保存当前判断与取舍；各记录分别注明已采用与仍待讨论的部分，不是不可更改的长期决策。

| 记录 | 初始建议 | 状态 |
| --- | --- | --- |
| [0001](0001-shared-core-and-hosts.md) | 合并 core / infra，顶级目录、桌面先行、服务器随后 | Accepted，骨架已落地 |
| [0002](0002-stack-and-storage.md) | Rust / Tauri 与 React + Vite + shadcn 基础，数据库驱动待定 | 部分 Accepted / 部分 Proposed |
| [0003](0003-ai-and-agent-boundaries.md) | 内置 Rust AI 与受限 MCP Server | Proposed |

这些记录可以在早期直接修订。若后续已有实现或公开讨论需要保留历史，再将旧记录标记 Superseded 并链接新方案；不要求每次探索都创建新 ADR，也不因偏离启动愿景额外设置审批。实际实现状态在相应架构文档维护。
