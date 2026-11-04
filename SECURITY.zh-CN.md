# 安全政策

[English](SECURITY.md) | 简体中文

> English summary: please report vulnerabilities privately through GitHub's private vulnerability reporting (Security Advisories) and never in public issues; do not include real credentials or real mail content in reports.

已发布版本以 [Releases](https://github.com/GuqierMcl/MailX/releases) 为准，版本变更见 [CHANGELOG](CHANGELOG.md)。

## 受支持的版本

安全问题在 `main` 分支修复，随后通过新版本发布；不承诺为旧版本单独回补。报告问题时请附上应用版本号。

## 如何报告漏洞

请通过本仓库 **Security** 页签的 “Report a vulnerability”（GitHub 私有漏洞报告 / Security Advisories）私下提交。

本仓库已启用私有漏洞报告。入口：[Report a vulnerability](https://github.com/GuqierMcl/MailX/security/advisories/new)。

请不要：

- 为安全漏洞创建公开 Issue、Pull Request 或讨论；
- 在报告中附带真实的邮箱凭证、令牌、私钥，或真实邮件内容、附件和包含真实地址的完整日志；复现请使用合成数据；
- 对他人的邮箱账号或服务发起测试。

报告中请尽量说明受影响的提交或分支、复现步骤、影响范围与你的判断。本项目由个人维护，不承诺固定的响应或修复时限，但会认真对待并尽力沟通。

## 关注范围

以下方向与 MailX 的安全边界直接相关，欢迎重点审视：

- 邮件 HTML 与正文隔离：邮件内容不可信，不应能执行脚本、未经用户允许加载远程资源，或获得 Tauri IPC 权限；
- Tauri IPC 暴露面：命令与权限是否超出界面实际所需；
- 凭证存储：凭证应只由后端 SecretStore 管理，不应出现在返回值、日志、数据库或诊断输出中；
- 网络路由：协议连接必须走账号路由，自定义代理失败时不得回退直连；
- 模型输出与 MCP 输入同样不可信，不能自行扩大宿主授予的权限。当前 MCP 总开关确认统一授权所有账号的邮件读取、整理、草稿与附件管理和直接发送，无需逐封确认；工具仍须校验动作、账号范围、草稿版本与幂等键。关闭开关应撤销访问，MCP 不提供删除邮件能力。

设计细节见[网络与安全](docs/architecture/network-security.md)。
