# Microsoft 365 / Outlook.com（Exchange）接入评估

状态：**未实现**。本文记录用户所说“Exchange”的实际含义、各接入路径的可行性、已完成的 Microsoft Entra 应用注册，以及接入前通用 OAuth 基础需要满足的条件。MailX 当前只有密码类认证，`oauth2` 输入返回 `unsupported`，见[账号配置](../architecture/account-configuration.md#5-oauth-扩展边界)。

## 1. “Exchange”指什么

Exchange 是微软的邮件服务器产品，不是单一服务商。用户反馈中的需求通常对应以下几类：

| 类型 | 典型特征 | 第三方客户端可用的登录方式 |
| --- | --- | --- |
| Microsoft 365 / Office 365（Exchange Online） | 公司或学校邮箱，网页版 `outlook.office.com`，登录页 `login.microsoftonline.com` | 只能 OAuth 2.0；密码直连已在所有租户关闭，管理员和微软支持都无法重新开启 [S1] |
| Outlook.com / Hotmail / Live 个人账号 | 个人邮箱，底层同为 Exchange Online | 2024-09-16 起第三方客户端必须 OAuth 2.0 [S2] |
| 自建 Exchange Server（本地部署） | 公司自有服务器，网页版常见 `mail.<公司域名>/owa` | 常见为 EWS / ActiveSync 加域账号密码；IMAP / SMTP 是否开启由管理员决定 |
| 世纪互联运营的 Office 365（中国版） | 独立云环境，登录与应用注册均与国际版分开 | OAuth 2.0，需要在该云环境单独注册应用 [S1] |

QQ 邮箱、腾讯企业邮等服务商设置里的“Exchange”指 ActiveSync 协议，与微软服务器无关，不属于本文范围。

## 2. 接入路径

| 路径 | 覆盖 | 评估 |
| --- | --- | --- |
| **OAuth + IMAP / SMTP（XOAUTH2）** | Microsoft 365、Outlook.com | **选定方案**。复用现有 IMAP 同步、SMTP 发送队列和账号路由，只在认证步骤改用 SASL XOAUTH2；微软确认该方式同时支持 Microsoft 365 与 Outlook.com 用户 [S3]。 |
| Microsoft Graph | Microsoft 365、Outlook.com | 功能更全，但需要与 IMAP 并列的另一套同步与发送实现。暂不采用；可考虑的局部用途是管理员关闭 SMTP AUTH 时改用 Graph `sendMail` 发信 [S1]。 |
| EWS | 自建 Exchange Server | Exchange Online 自 2026-10-01 起开始阻断 EWS 请求，自建 Exchange 仍完整支持 [S4]。需要 SOAP、Autodiscover 与 NTLM / Kerberos，成本高，仅在确认大量“自建且关闭 IMAP”的需求后再评估。 |
| Exchange ActiveSync | 移动端为主 | 不采用。 |

微软宣布 SMTP AUTH 的密码认证将于 2026 年 12 月底起对现有租户默认关闭，最终移除日期预计 2027 年下半年公布 [S5]。这项调整不影响 OAuth 方式的 SMTP AUTH，但说明密码方式不应作为 Microsoft 365 的接入方案。

## 3. Microsoft Entra 应用注册

2026-11 已为 MailX 注册公共客户端应用，2026-11-10 确认以下配置：

| 项 | 值 |
| --- | --- |
| 应用程序（客户端）ID | `44a7bc3e-d9fd-4af9-90a6-a03b62013b40`；公共客户端，不是机密，可写入客户端；无客户端密钥 |
| 支持的账户类型 | 任何组织目录中的账户和个人 Microsoft 账户（`AzureADandPersonalMicrosoftAccount`） |
| 授权端点 | `https://login.microsoftonline.com/common/oauth2/v2.0/authorize` |
| 令牌端点 | `https://login.microsoftonline.com/common/oauth2/v2.0/token` |
| 平台 | 移动和桌面应用程序 |
| 重定向 URI | `http://localhost`、`http://127.0.0.1`；后者只能通过应用清单添加 [S6] |
| 声明的委托权限 | Office 365 Exchange Online：`IMAP.AccessAsUser.All`、`SMTP.Send`；Microsoft Graph：`offline_access`、`openid`、`email`、`profile` |
| 请求的 scope | `offline_access openid email profile https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send` [S3] |
| 浏览器授权试跑 | 已成功取得授权码回调 |
| 所在目录 | 个人 Microsoft 账号的默认目录 |

注册规则对实现的约束：

- 本机回调匹配时只忽略端口，**路径必须与注册值一致**；同时支持个人账号的应用不允许回调地址带查询参数 [S6]。
- 微软建议使用 `127.0.0.1` 而非 `localhost`，避免防火墙或网络设置导致回调失败 [S6]。
- 应用建在个人 Microsoft 账号的目录中，**按微软规定不能进行发布者验证** [S7]。发布者验证需要以组织身份加入 Microsoft AI Cloud Partner Program 并通过审核，在工作或学校账号的目录中注册应用，并设置经 DNS 验证的自有发布者域名（不能是 `*.onmicrosoft.com`）。将来如需验证，要在组织目录中重新注册，客户端 ID 随之改变，已登录账号需重新登录一次。

## 4. 服务器与登录

| 方向 | 服务器 | 认证 |
| --- | --- | --- |
| 收信 | `outlook.office365.com:993`，隐式 TLS | `AUTHENTICATE XOAUTH2` |
| 发信 | `smtp.office365.com:587`，STARTTLS | `AUTH XOAUTH2` |

XOAUTH2 字符串为 `base64("user=" + 邮箱 + "\x01auth=Bearer " + access_token + "\x01\x01")` [S3]。共享邮箱可用授权用户的令牌、把 `user=` 换成共享邮箱地址 [S3]，当前不在首版范围。Outlook.com 个人账号是否同样使用 `smtp.office365.com`，还是需要 `smtp-mail.outlook.com`，以真实账号验证为准。

## 5. 对通用 OAuth 基础的要求

通用基础按授权码 + PKCE、本机回调、令牌刷新与 XOAUTH2 设计（另见 [Gmail 接入评估](gmail-integration.md#3-oauth-的后续条件)）。接入 Microsoft 还需满足：

1. **固定回调路径**。回调地址由服务商策略给出固定路径（与注册值一致），不能每次随机生成路径；防伪造依靠 `state` 与 PKCE。
2. **身份校验不依赖解析访问令牌**。访问 `outlook.office.com` 的访问令牌不供客户端解析，个人账号的令牌也不是可读的 JWT。身份校验需要拿到令牌响应中的 ID 令牌：授权主体取 `tid` 与 `oid` 组合；目标邮箱以 XOAUTH2 登录 IMAP 成功为准，不靠字符串比较登录名（UPN）与邮箱地址。
3. **签发方按租户变化**。`/common` 登录后 ID 令牌的 `iss` 为 `https://login.microsoftonline.com/{tid}/v2.0`，个人账号也有固定的租户 ID。账号绑定不能要求 `iss` 等于某个固定值。
4. **令牌存储容量**。Windows 凭据管理器单个条目上限 2560 字节，`keyring` 按 UTF-16 存储，即约 1280 个字符。微软的访问令牌与刷新令牌长度通常远超 Gmail，二者合存很可能超限。需要只持久化刷新令牌、分片存储或改用加密存储，并以真实令牌长度验证。
5. **错误分类**。令牌端点返回 `invalid_grant`、`interaction_required`（多因素认证或条件访问策略变化）时都应提示重新登录。管理员关闭 SMTP AUTH 时 SMTP 返回增强状态码 `5.7.139`；IMAP 被关闭时只返回通用的认证失败。这两种情况都不应陷入反复要求重新登录，而应用通俗文字提示联系管理员。服务器原文可能回显令牌，只能读取状态码，不保存原文。
6. **授权请求附加参数**。支持 `prompt=select_account`（多账号切换）和 `login_hint`（预填邮箱）。
7. **刷新令牌轮换**。微软每次刷新都可能返回新的刷新令牌，必须先持久化再使用新的访问令牌；所有令牌请求走账号路由，代理失败不回退直连。

## 6. 企业租户的限制

- **授权审批**：未经发布者验证的多租户应用，在开启基于风险的升级同意或只允许已验证发布者的租户中，员工无法自行授权，需要管理员批准 [S7]。个人账号不受影响，授权页显示“未验证”。帮助文档需说明如何请管理员为全公司批准。
- **协议开关**：管理员可按邮箱关闭 IMAP 或 SMTP AUTH，新租户默认关闭 SMTP AUTH [S1]。
- **企业自建应用**：可参照 Gmail 评估中的“用户自建 OAuth 应用”，允许管理员在本公司目录中注册单租户应用并填入客户端 ID，单租户应用不需要发布者验证。

## 7. 暂不覆盖与待验证

暂不覆盖：自建 Exchange Server 的 EWS 接入（管理员开启了 IMAP / SMTP 时，现有“手动配置 IMAP/SMTP”即可使用）、世纪互联中国版、共享邮箱、Graph 同步。

待真实账号验证：

- Microsoft 365 与 Outlook.com 的授权、IMAP 同步、IDLE、SMTP 发信全链路，以及令牌过期后长连接的行为。
- Outlook.com 的 SMTP 服务器选择。
- 令牌响应实际回显的 scope 格式与令牌长度。
- 企业租户禁止自行授权、关闭 IMAP、关闭 SMTP AUTH 时的实际返回与提示。
- 已发送邮件是否由服务器自动保存到“已发送邮件”，以及与本机已发送副本的对账。

## 参考资料

- **[S1] [Deprecation of Basic authentication in Exchange Online](https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online)**：Exchange Online 已在所有租户关闭密码认证，IMAP / POP / SMTP AUTH 支持 OAuth，世纪互联版本单独说明。
- **[S2] [Modern authentication methods now needed to continue syncing Outlook Email in non-Microsoft email apps](https://support.microsoft.com/en-us/office/modern-authentication-methods-now-needed-to-continue-syncing-outlook-email-in-non-microsoft-email-apps-c5d65390-9676-4763-b41f-d7986499a90d)**：Outlook.com 个人账号自 2024-09-16 起要求现代认证。
- **[S3] [Authenticate an IMAP, POP or SMTP connection using OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)**：scope、XOAUTH2 格式、共享邮箱，适用于 Microsoft 365 与 Outlook.com。
- **[S4] [Retirement of Exchange Web Services in Exchange Online](https://techcommunity.microsoft.com/blog/exchange/retirement-of-exchange-web-services-in-exchange-online/3924440)**：2026-10-01 起阻断 Exchange Online 的 EWS 请求，自建 Exchange 不受影响。
- **[S5] [Updated Exchange Online SMTP AUTH Basic Authentication Deprecation Timeline](https://techcommunity.microsoft.com/blog/exchange/updated-exchange-online-smtp-auth-basic-authentication-deprecation-timeline/4489835)**：SMTP AUTH 密码认证 2026 年底默认关闭。
- **[S6] [Redirect URI (reply URL) best practices and limitations](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)**：本机回调忽略端口、路径须一致、建议 `127.0.0.1` 及门户添加限制。
- **[S7] [Publisher verification overview](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview)**：发布者验证条件，个人账号注册的应用不能验证，未验证应用的同意限制。
