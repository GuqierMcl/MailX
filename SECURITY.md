# 安全政策

> English summary: please report vulnerabilities privately through GitHub's private vulnerability reporting (Security Advisories) and never in public issues; do not include real credentials or real mail content in reports.

MailX 仍处于早期开发阶段，尚未发布任何版本。

## 受支持的版本

目前没有已发布版本；只有 `main` 分支上的最新代码会被考虑修复。

## 如何报告漏洞

请通过本仓库 **Security** 页签的 “Report a vulnerability”（GitHub 私有漏洞报告 / Security Advisories）私下提交。

<!-- TODO（维护者）：若私有漏洞报告不可用，在此补充安全联系邮箱。此处尚未确定任何联系地址。 -->

请不要：

- 为安全漏洞创建公开 Issue、Pull Request 或讨论；
- 在报告中附带真实的邮箱凭证、令牌、私钥，或真实邮件内容、附件和包含真实地址的完整日志；复现请使用合成数据；
- 对他人的邮箱账号或服务发起测试。

报告中请尽量说明受影响的提交或分支、复现步骤、影响范围与你的判断。本项目为个人维护的早期项目，不承诺固定的响应或修复时限，但会认真对待并尽力沟通。

## 关注范围

以下方向与 MailX 的安全边界直接相关，欢迎重点审视：

- 邮件 HTML 与正文隔离：邮件内容不可信，不应能执行脚本、未经用户允许加载远程资源，或获得 Tauri IPC 权限；
- Tauri IPC 暴露面：命令与权限是否超出界面实际所需；
- 凭证存储：凭证应只由后端 SecretStore 管理，不应出现在返回值、日志、数据库或诊断输出中；
- 网络路由：协议连接必须走账号路由，自定义代理失败时不得回退直连；
- 模型输出与 MCP 输入同样不可信，AI 不能自行批准发信。

设计细节见[网络与安全](docs/architecture/network-security.md)。
