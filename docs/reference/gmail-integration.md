# Gmail 接入评估

状态：**应用专用密码预设已有实现；OAuth 登录未开放**。本文记录 Gmail 的三种接入方式、当前选择与后续条件。实现与验证证据见[验证记录](../development/validation.md#gmail-app-password)，用户说明见[添加账号指南](../guides/zh-CN/README.md#gmail)。

## 1. 三种登录方式

| 方式 | 客户端 ID | 用户操作 | MailX 状态 |
| --- | --- | --- | --- |
| 应用专用密码 | 不需要 | 开启两步验证，在 Google 账号生成 16 位密码，填入 MailX | 已实现，复用现有密码账号模型 |
| 用户自建 OAuth 应用 | 用户自己的 Google Cloud 客户端 | 创建项目、配置同意页、创建桌面客户端，填入客户端 ID 后在浏览器登录 | 未实现 |
| MailX 官方 OAuth 应用 | MailX 内置客户端 | 在浏览器选择账号并允许 | 未实现，需 Google 审核 |

后两种共用同一套 OAuth 流程，只是客户端凭据来源不同。应用专用密码与 OAuth 无关，不需要 Google Cloud 项目或审核。

## 2. 应用专用密码预设（当前实现）

- 服务商标识 `gmail`，新建时只接受 `gmail.com` / `googlemail.com`。
- 收信 `imap.gmail.com:993` 隐式 TLS，发信 `smtp.gmail.com:465` 隐式 TLS；收发用户名默认完整邮箱地址，共享一份应用专用密码，高级设置可分别覆盖。
- 复用 `appPassword` 凭证文案与官方帮助入口（[Google 账号帮助：使用应用专用密码登录](https://support.google.com/accounts/answer/185833)）；桌面与浏览器 mock 只把精确的 `gmail` 映射到该地址。
- 不增加认证类型、协议特判或数据库迁移；`imap.gmail.com` 沿用标准兼容策略。图标为 Google 官网引用的官方 SVG，来源见[图标说明](../../frontend/src/assets/providers/README.md#gmail)。

Google 账号帮助说明的前提与限制：创建应用专用密码需要两步验证；两步验证只配置了安全密钥、开启高级保护，或登录的是工作 / 学校等组织账号时没有此选项；修改 Google 账号密码会撤销已有应用专用密码。Gmail 帮助说明自 2025 年 1 月起 IMAP 始终开启，用户无需手动启用。

Google 显示的应用专用密码带分组空格。MailX 不改写秘密输入，引导用户粘贴时去掉空格；带空格能否通过以真实账号验证为准。

### 公开端点探测

2026-11-08 15:37（Asia/Shanghai）在本机 macOS 以 Python 标准库连接官方端点，默认信任链验证证书和主机名，只执行 CAPABILITY / EHLO / STARTTLS / QUIT，未登录、未使用凭据：

| 端点 | 结果 |
| --- | --- |
| `imap.gmail.com:993` | 隐式 TLS 成功；登录前公告 `IMAP4REV1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 SASL-IR AUTH=XOAUTH2 AUTH=PLAIN AUTH=PLAIN-CLIENTTOKEN AUTH=OAUTHBEARER`，未公告 LOGINDISABLED。 |
| `smtp.gmail.com:465` | 隐式 TLS 成功；公告 `AUTH LOGIN PLAIN XOAUTH2 PLAIN-CLIENTTOKEN OAUTHBEARER XOAUTH`，`SIZE 35882577`。 |
| `smtp.gmail.com:587` | 升级前不公告 AUTH，STARTTLS 后公告与 465 相同。 |

探测只说明端点可达、证书有效、公告了 MailX 已实现的 PLAIN / LOGIN，不代替 MailX 的 rustls 栈、账号路由与真实登录收发验证。`SIZE` 约 34.2 MiB，与 Gmail 面向用户的附件上限是不同层面的数据，不据此调整 MailX 的附件规则。

### 待验证的 Gmail 语义

Gmail 以标签模拟文件夹，同一封邮件可出现在多个 IMAP 文件夹（例如收件箱与 `[Gmail]/All Mail`）。以下行为沿用通用实现，需用真实账号确认后再决定是否在兼容层补充规则，不在主流程加品牌分支：

- 统一视图、搜索和未读数是否因 `All Mail` 等文件夹出现重复。
- 删除移入 `[Gmail]/Trash`、本机归档与 Gmail 归档语义的差异。
- SMTP 发信后 Gmail 自动保存到 `[Gmail]/Sent Mail`，按原生 `Message-ID` 对账是否稳定。
- 星标与 `[Gmail]/Starred` 的对应，IDLE 推送与大附件投递。

## 3. OAuth 的后续条件

需要 OAuth 的场景：让普通用户在浏览器一键登录；覆盖无法生成应用专用密码的账号（组织账号、高级保护、仅安全密钥）；Google 进一步限制密码类登录。

- IMAP / SMTP 的 XOAUTH2 需要 `https://mail.google.com/` scope（[Gmail XOAUTH2 说明](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)），属于受限 scope。面向公众的官方应用需通过 Google 验证并接受年度安全评估（[OAuth 应用验证](https://support.google.com/cloud/answer/13464321)）。
- 个人使用（少于 100 个用户）、开发测试状态和组织内部应用可不经验证（[无需验证的情形](https://support.google.com/cloud/answer/13464323)）；未验证应用会显示未验证提示并受 100 用户上限约束。对外部用户处于测试状态的项目，refresh token 7 天过期（[Google OAuth 2.0 说明](https://developers.google.com/identity/protocols/oauth2)）。
- 桌面应用支持 `127.0.0.1` 任意端口回调，必须使用 PKCE，`client_secret` 在文档中标为可选（[桌面应用 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)）。

通用 OAuth 基础按授权码 + PKCE、loopback 回调、刷新协调与 XOAUTH2 设计。在此之上接入 Gmail 还需：拆分“服务商策略”（代码固定的端点、邮件服务器、scope、身份校验）与“客户端凭据”（官方内置 / 用户提供）；token 请求支持可选 `client_secret`；允许经核验同一主体与邮箱的重新授权更换客户端，使自建应用账号可迁移到官方应用而保留本地数据；实现 Gmail 身份校验适配器。refresh token 绑定客户端，迁移时用户需重新登录一次。
