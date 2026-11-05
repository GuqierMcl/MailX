# iCloud 邮箱接入可行性评估

状态：**Implemented / 专用预设、官方图标、应用专用密码引导与官方帮助入口已有实现；真实账号兼容尚待验收**。

本评估区分 Apple 官方要求、MailX 当前实现和待验证行为。公开端点探测及调研基线集中记录在[验证记录](../development/validation.md#icloud-mail-feasibility)，不将公开握手成功视为真实邮箱兼容通过。

## 1. 结论与建议范围

**已采用 IMAP + SMTP + Apple 应用专用密码，复用现有协议与账号模型。** iCloud 专用入口提供端点预填、双语引导和固定官方帮助链接，凭证、账号路由、同步、正文读取和发送队列沿用现有实现。本次没有新增认证类型、数据库迁移或协议适配层；是否需要小型兼容修复，由真实账号验收决定。

| 配置 | 建议 |
| --- | --- |
| 首轮账号 | 已开通 iCloud Mail 的主邮箱，优先验证 `@icloud.com` |
| 接收服务器 | `imap.mail.me.com:993`，隐式 TLS |
| 发送服务器 | `smtp.mail.me.com:587`，强制 STARTTLS，要求认证 |
| IMAP 用户名 | 预设默认完整 iCloud Mail 地址；高级设置可单独改为地址的本地部分，例如 `you` |
| SMTP 用户名 | 完整 iCloud Mail 地址，例如 `you@icloud.com` |
| 凭证 | 同一份 Apple 应用专用密码，收发共享 |

以上参数来自官方服务器说明 [S1]。587 必须配置为 `startTls`，不能复制现有服务商的 465 / 隐式 TLS 预设。无需增加明文端口、忽略证书错误或失败后自动切换主机的逻辑。iCloud Mail 不支持 POP [S1]。

Apple 首先建议 IMAP 使用地址的本地部分，并在连接失败时使用完整地址 [S1]。预设沿用 MailX 的完整邮箱用户名默认值，因为官方明确允许该格式；引导说明需要时在高级设置中单独修改 IMAP 用户名，SMTP 保持完整地址。收发均不在后台自动尝试多个身份，编辑已有账号也不会重新套用预设。

入口精确接受 `icloud.com`、`me.com` 和 `mac.com`，首轮优先验证已开通的 `@icloud.com` 主地址。别名登录、别名发件、历史地址和 iCloud+ 自定义域兼容均待独立验收，不通过后缀直接推断其主地址或登录身份 [S6]。自定义域可使用通用 IMAP / SMTP 入口；Apple 授权登录尚未实现。

## 2. 认证与账号前提

### 主邮箱与应用专用密码

Apple 要求使用 iCloud Mail 前先创建主邮件地址 [S7]。Apple 账号本身与 iCloud Mail 邮箱是不同概念，不能把任意 Apple 账号的外部登录邮箱直接当成 iCloud Mail 地址。

生成和使用应用专用密码的账号须启用双重认证。用户在 `account.apple.com` 的“登录与安全 → 应用专用密码”生成密码，再粘贴到 MailX；不应填写 Apple 账号主密码或双重认证验证码 [S2]。

- 最多同时保有 25 个有效应用专用密码，可单独撤销或全部撤销。
- 修改或重置 Apple 账号主密码会自动撤销全部应用专用密码，需要重新生成并更新 MailX。
- 官方页面没有给出固定的周期性到期规则，不套用 139 邮箱的 90 天说明，也不承诺永不过期。
- 沿用现有 password 秘密模型和 SecretStore，保留用户输入原值，不增加固定长度、删除连字符或读取凭证的接口。

### Apple 授权登录

Apple 已提供面向**受支持第三方应用**的账号授权方式，允许访问 iCloud 邮件、日历和通讯录；不支持该方式的应用可使用应用专用密码 [S2][S3]。因此不能写成“iCloud 没有授权登录”。

本轮查阅的官方用户说明未提供 MailX 可直接采用的应用注册、授权端点、scope、token 和邮件协议接入规范。公开端点公告 XOAUTH2 也不证明 MailX 已有应用资格或可用授权流程。普通 Sign in with Apple 是应用身份登录，不能直接当成已获 iCloud Mail 访问权限 [S8]；如要采用，需另行取得对应开发者规范和接入资格。

MailX 当前 `oauth2` 输入返回 `unsupported`（`core/src/accounts/mod.rs`），尚无 Apple 授权回调和 token 生命周期管理。首轮采用应用专用密码可以独立推进，不以此证明 Apple 授权方式不可实施。

## 3. 当前实现

| 范围 | 当前实现与边界 |
| --- | --- |
| TLS 与路由 | `core/src/imap/mod.rs::connect_client` 使用隐式 TLS；`core/src/smtp/mod.rs::authenticated_connection` 使用强制 STARTTLS，升级后重新 EHLO；连接经过既有账号路由，真实直连与代理分别验证。 |
| 用户名与秘密 | `core/src/accounts/types.rs` 支持独立收发用户名、shared/separate password；`frontend/src/features/accounts/account-draft.ts` 空用户名默认完整邮箱地址。高级设置保留独立覆盖，应用专用密码按原值进入现有 SecretStore。 |
| 协议认证 | IMAP 使用 LOGIN 命令；SMTP 优先 AUTH PLAIN，仅在未公告 PLAIN 时使用 LOGIN，认证失败不自动切换机制。SMTP 公开能力匹配；IMAP LOGIN 与真实应用专用密码仍待验，不把 AUTH=PLAIN 与 LOGIN 命令混为一谈。 |
| 账号预设 | `frontend/src/features/accounts/providers.ts` 注册 `icloud`，提供搜索词和 993 / 587 默认端点。预设仅初始化新账号，不覆盖已保存的手动配置。 |
| 入口域名 | `preset.domains` 在创建时精确校验 `icloud.com` / `me.com` / `mac.com`，子域和近似后缀不匹配。自定义域使用通用入口，不改变其他服务商的域名校验。 |
| 凭证文案 | 连接表单与故障帮助提供中英文“应用专用密码”、主地址开通、双重认证及撤销规则；后端仍是 password，不新增秘密格式约束。 |
| 官方帮助 | `desktop/src/commands.rs::provider_help_url` 与 mock client 将 `icloud` 映射到 Apple 官方应用专用密码帮助 [S2]，仍只接受 provider ID 白名单。 |
| 服务商图标 | 本地打包 iCloud 官网用于 Mail / Mail2 的蓝底白信封 PNG 原件，保留完整图形、背景、透明度和细字。复用账号图标偏好，默认显示官方图标，关闭后回到首字母头像；来源链、尺寸与 SHA-256 见[图标说明](../../frontend/src/assets/providers/README.md#icloud-mail)。 |
| 服务商兼容 | `imap.mail.me.com` 使用标准默认策略，没有 iCloud 专属兼容模块；已有前端入口不构成新增服务端特判的证据，见[兼容层约定](../architecture/provider-compatibility.md)。 |

本次同步更新双语用户指南、账号说明、相关入口与 CHANGELOG。预设和引导已有实现，真实登录、收读发及写回仍待验证；不把入口存在或公开握手成功写成完整兼容证明。

## 4. 重点兼容边界

### 目录、标记、删除与 IDLE

现有 `core/src/sync/folders.rs` 优先识别 SPECIAL-USE 属性，并包含 `Sent Messages`、`Deleted Messages` 等常见目录名兜底；支持修改版 UTF-7。仍需采集登录后的 CAPABILITY、LIST、分隔符和中文自定义目录，确认角色判断及目录访问。

公开登录前能力没有公告 IDLE、MOVE 或 UIDPLUS，不足以判断登录后是否支持。MailX 登录后重新查询能力：IDLE 不可用时保留轮询；移入废纸篓优先 MOVE，否则要求 UIDPLUS 执行 COPY + `\Deleted` + UID EXPUNGE；永久删除要求 UIDPLUS。无法安全执行时不使用普通 EXPUNGE。需分别验证新邮件推送、标记写回、移入废纸篓及撤销，不能由收信成功外推删除可用。

`XAPPLEPUSHSERVICE` 是公开端点公告的另一能力，并不等于 MailX 已接入 Apple 推送。首轮继续采用已有 IMAP IDLE / 轮询，无需先实现 Apple 专有推送。并发连接限制、睡眠唤醒和与 Apple Mail 同时在线的稳定性待验，不编造固定配额。

同步仍受 MailX 已有固定 30 天窗口、每轮最新 1,000 UID 和按需正文缓存等边界约束；添加 iCloud 预设不会变为全历史下载，见[同步设计](../architecture/mail-sync.md)。

### 已发送、草稿与归档

MailX 已为 SMTP accepted 的邮件保留本机副本，标准策略按同账号当前来源、当前 UIDVALIDITY 的已发送目录原生 `Message-ID` 对账（`core/src/storage/outbox.rs::reconcile_sent_copies`）。SMTP 接受与服务器副本同步的先后顺序均可处理。iCloud 沿用该默认策略，未借用 QQ 的扩展头部规则；匹配成立后释放本机快照，当前不执行 IMAP APPEND，边界见[服务商兼容层](../architecture/provider-compatibility.md)。

必须通过真实 iCloud SMTP 发信检查服务器是否自动保存副本、目录和 Message-ID 是否正确、是否重复。若服务商不自动保存，MailX 本机可见不能保证 iCloud 网页或 Apple Mail 可见；是否增加 APPEND 是独立行为设计，不为预设直接加入，也不因缺少副本重发 SMTP。草稿和归档沿用当前本机行为，不承诺跨设备同步。

### 大小、配额与空间

Apple 官方指引列出每天 1,000 封邮件、每天 1,000 位收件人、单封最多 500 位收件人，收发邮件大小 20 MB；Mail Drop 可扩大至 5 GB。免费 iCloud 空间为共享的 5 GB，用满后无法收发邮件 [S4]。不将这些上限当成投递保证，不用 MailX 本地发送数推算剩余额度，也不把 Mail Drop 写成标准 SMTP 已具备的能力。

MailX 当前限制的是普通附件和内嵌图片**原始总字节 20 MiB**，不是完整 MIME 大小；编码、正文和头部会增加体积。公开 SMTP 本次公告的 `SIZE` 与官方 20 MB 指引分别记录在[验证记录](../development/validation.md#icloud-mail-feasibility)，不能据一次公告上调产品承诺或推导安全的原始附件上限。

现有 SMTP 会根据当次 EHLO SIZE 对完整 MIME 做发送前检查，并提供附件容量提示，但不能保证低于 SIZE 的邮件一定获准发送。真实验收应覆盖普通小附件和接近限制的合成样本；MailX 没有 Mail Drop 或超限自动上传功能 [S9]。

### 别名与自定义域

历史域名和别名可能指向同一个 iCloud 账号，不能直接视为独立邮箱 [S6][S10]。iCloud+ 支持自定义邮件域 [S5]，但这些功能说明不足以确定第三方 IMAP / SMTP 的登录用户名和允许的发件身份。

首轮优先主地址；别名、历史地址和自定义域分别核实认证身份、From、SMTP envelope 与实际投递。MailX 可分别设置显示邮箱和协议用户名，但没有独立的发件别名选择器，也不自动合并指向同一邮箱的多个账号。

## 5. 推进顺序与支持门槛

| 工作包 | 内容与完成标准 |
| --- | --- |
| A：真实最小 PoC（待验） | 经 iCloud 或通用入口接入一个已开通的主邮箱，显式填写用户名与应用专用密码；验证收发登录、LIST / EXAMINE、正文读取和一次获授权的实际投递，核对服务器已发送副本。排除认证和目录访问阻断。 |
| B：专用预设（已实现） | iCloud 入口、993/587 参数、应用专用密码双语引导和固定帮助链接；复用高级用户名、路由及凭证修改，不改变已有手动账号。自动回归与本轮检查结果见[验证记录](../development/validation.md#icloud-mail-feasibility)。 |
| C：基础兼容验收 | 首次/增量同步、中文与自定义目录、正文和附件、已读/星标写回、删除与撤销、IDLE 或轮询、重启离线缓存、应用专用密码撤销及替换恢复；分别记录 SMTP accepted、收件端到达和已发送可见。 |
| D：扩大范围 | `me.com` / `mac.com`、别名、自定义域、SOCKS5 / HTTP CONNECT、其他平台分别取得证据；Apple 授权登录另行评估。 |

B 已完成，下一步以 A 和 C 的真实证据界定公开兼容范围。真实服务商的登录、目录写回与已发送副本仍是主要不确定性；本次未使用真实 iCloud 账号。公开握手、自动回归和单一平台主地址成功，均不替代其他组合的验收。真实账号读取、写回和发信在届时获得相应授权后进行。

## 6. 官方来源

- **[S1] [iCloud Mail server settings for other email client apps](https://support.apple.com/en-us/102525)**：IMAP / SMTP 主机、端口、用户名、应用专用密码和不支持 POP。
- **[S2] [Sign in to apps with your Apple Account using app-specific passwords](https://support.apple.com/en-us/102654)**：双重认证前提、生成与撤销、25 个上限、主密码变更后全部撤销。
- **[S3] [Access your iCloud Mail, Calendar, and Contacts in third-party apps](https://support.apple.com/en-us/121539)**：受支持第三方应用的 Apple 账号授权，与应用专用密码替代路径；不是 MailX 已取得接入资格的证据。
- **[S4] [Mailbox size and message sending limits in iCloud](https://support.apple.com/en-us/102198)**：共享空间、发信/收件人配额、20 MB 与 Mail Drop 边界。
- **[S5] [Use Custom Email Domain with iCloud Mail](https://support.apple.com/en-us/102540)**：iCloud+ 自定义域功能；不据此推断第三方协议登录身份。
- **[S6] [About your @icloud.com, @me.com, and @mac.com email addresses](https://support.apple.com/en-us/118230)**：历史地址与别名关系，不据此推断第三方认证规则。
- **[S7] [Create a primary email address for iCloud Mail](https://support.apple.com/guide/icloud/create-a-primary-icloudcom-email-address-mmdd8d1c5c/icloud)**：使用 iCloud Mail 前须先创建主地址。
- **[S8] [Sign in with Apple — Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/sign-in-with-apple)**：应用身份登录和用户信息，不等于 iCloud Mail 数据授权。
- **[S9] [Mail Drop limits](https://support.apple.com/en-us/108329)**：Apple Mail / iCloud.com 大附件功能及限制，不是标准 SMTP 自动具备的能力。
- **[S10] [Add and manage email aliases for iCloud Mail on iCloud.com](https://support.apple.com/guide/icloud/add-and-manage-email-aliases-mm6b1a490a/icloud)**：别名与主地址关系及 Apple 客户端用法，不据此保证第三方 SMTP 兼容。

仓库能力依据另见[账号配置与凭证生命周期](../architecture/account-configuration.md)、[同步设计](../architecture/mail-sync.md)、[写信与发送](../architecture/sending.md)和[用户指南](../guides/zh-CN/README.md)。
