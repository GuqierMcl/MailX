# 邮箱服务商兼容层

本文是服务商差异的模块边界与扩展约定。共享核心提供协议、同步、发送和持久化的主流程；具体服务商对标准行为的偏差，由独立兼容模块解释并通过中立接口交回主流程。新增兼容能力时更新本文对应规则，真实服务商与平台的验证证据统一记录在[验证指引](../development/validation.md)。

## 1. 职责与依赖

服务商兼容规则属于 Rust core。桌面写信、本地 MCP 和后续宿主共享同一套行为，不在 UI、IPC 或工具层分别补偿。

| 层 | 职责 | 边界 |
| --- | --- | --- |
| 通用主流程 | 账号范围、网络路由、协议操作、同步计划、发送状态机、事务、持久化和事件 | 通过中立接口调用兼容能力，不散布按服务商品牌分支的判断 |
| 兼容接口与分派 | 提供主流程所需的最小输入 / 输出类型，根据可信账号连接信息选择实现，并提供标准默认行为 | 只按实际需求增加扩展点，不预建未使用的插件框架 |
| 具体服务商模块 | 服务商识别、已知偏差的解释、兼容证据的提取与校验 | 不持有数据库、凭证或网络连接，不自行重发、写回或删除数据 |
| UI 与工具层 | 展示核心返回的结果并继续使用既有应用服务 | 不依据服务商重写邮件身份，不自行合并已发送列表 |

发现有证据的服务商行为偏差时，优先在服务商模块中解释，并按需增加中立扩展点。遵循标准行为的账号继续使用默认策略，无需为新增前端预设建立空模块。通用主流程描述“需要什么结果”，服务商实现描述“怎样从该服务商的响应得到结果”。前端的服务商入口、图标与账号显示资料不是证明服务器行为的证据。

新浪免费邮、飞书邮箱（中国版）和 Lark 邮箱（国际版）均沿用标准协议策略；新浪 IMAP 端点另需系统 TLS 完成握手，见[第 7 节](#7-连接-tls-实现)。新浪按地址后缀选择预先登记的端点；飞书与 Lark 接受企业自有域名，但分别使用用户所选服务商的固定端点。失败时不向另一个服务提交凭证，已有账号也不因地址后缀被重新归类。不以品牌标识在 core 中改变已发送副本、同步或连接路由。端点、凭证与范围见[接入评估](../reference/mail-provider-preset-candidates.md)。

空文件夹缺少 SEARCH 响应的容错属于通用 IMAP 同步规则。只读 EXAMINE 明确给出 `0 EXISTS` 且该证据在搜索期间仍有效时，匹配的 tagged OK 可以完成空结果。规则适用于各 IMAP 端点，详见[邮件同步](mail-sync.md)。

## 2. 共同约束

- 协议连接始终通过账号路由，兼容规则不能绕过代理、TLS、凭证管理或宿主授予的账号范围；自定义代理失败仍不得回退直连。
- SMTP 回复、IMAP 头部和邮件正文均是不可信输入。兼容解析需要限制格式与长度，拒绝缺失、畸形或歧义证据；不得把服务器原文直接暴露给用户或写入普通诊断日志。
- 兼容能力不得改变发送队列的可靠性规则。已接受发送的副本没有匹配成功，不代表 SMTP 发送失败，也不能触发重新发送。`unknown` 不得因猜测匹配而被自动确认为已发送。
- 不以主题、显示名称或相近时间猜测两封邮件相同。只有明确的关联证据才能隐藏本机副本并释放其快照；证据不足时保留本机副本。
- 对账的幂等性、数据来源隔离与事务由主流程保证。服务商模块只能提供经校验的证据，不能自行改变邮件的远端身份或本地 UUID。
- 通用测试验证主流程与边界，服务商模块使用不含真实邮件和个人信息的固定样例验证兼容规则；自动测试与真实服务商验收分别记录。

## 3. 已发送副本的扩展范围

SMTP 接受发送后保留本机快照，IMAP 同步到服务器副本后再对账。标准实现以本账号已发送文件夹中的 `Message-ID` 为关联证据；兼容层用于解释服务商改写该标识时仍可确定验证的关联信息。

对账只改变 MailX 对本机副本的展示和快照引用，不合并两条远端位置，不删除服务器邮件，也不增加 IMAP `APPEND` 或 SMTP 请求。桌面与 MCP 查询由同一存储结果决定是否显示 `localOnly` 副本，详细列表与阅读行为见[写信与发送](sending.md#9-已发送副本)。

本机发送结果和服务器副本可能按任意顺序进入数据库。主流程需要在两侧状态变化后都能对账，并能重新处理已有缓存；不能把一次头部下载当作唯一的对账机会。已记录的匹配应保持幂等，不能恢复已释放的快照。

### 3.1 中立接口

`core/src/mail/compatibility.rs` 定义 `ProviderCompatibility`，`core/src/providers/` 负责注册和具体实现。主流程只消费以下能力：

| 接口 | 用途 |
| --- | --- |
| `policy_key` | 标识当前兼容策略及版本，判断缓存头部是否需要重新处理 |
| `extra_header_fields` | 声明标准邮件头之外需要获取的字段，由既有 IMAP 头部通道按账号路由读取 |
| `original_message_id` | 从已获取的头部提取并校验原始邮件标识；没有确定证据时返回空 |
| 会话线索（`In-Reply-To`、`References`） | 不是服务商扩展点：这两个标准头部属于所有端点的基础头部集合，由 `parse_header` 解析后随头部入库，供会话归并使用（`storage/conversations.rs`）。引入时两种策略的 `policy_key` 都升到 v2（`default:v2`、`qq:v2`），已缓存邮件因此按下文补抓一次；归并只在同一账号内进行，不依据服务商品牌分支 |

策略根据实际 IMAP 连接端点选择。未知端点采用标准默认实现，不请求服务商扩展头部，也不启用其他服务商的规则；不能只凭账号邮箱后缀、UI 的 `providerId` 或用户选择的图标启动兼容行为。

iCloud 预设的 `imap.mail.me.com` 端点当前沿用标准默认策略，以原生 `Message-ID` 对账；没有已验证的 iCloud 专属身份规则，也未建立空的 iCloud 兼容模块。若真实账号显示出协议或副本差异，再按本页边界补充证据与适配，不能将品牌分支散入同步、发送或存储主流程。

Gmail 预设的 `imap.gmail.com` 端点同样沿用标准默认策略。Gmail 以标签模拟文件夹，同一封邮件可出现在多个 IMAP 文件夹，删除、归档和已发送副本的实际语义尚待真实账号验证；有证据后再按本页边界决定是否增加兼容规则，见 [Gmail 接入评估](../reference/gmail-integration.md#待验证的-gmail-语义)。

### 3.2 QQ 邮箱

QQ 模块位于 `core/src/providers/qq.rs`，仅对实际 IMAP 主机名精确等于 `imap.qq.com` 的账号启用，比较不区分 ASCII 大小写。后缀相似的其他主机不匹配。

该模块额外读取 `X-OQ-MSGID`，要求头部只出现一次，且值为带尖括号的单个 ASCII dot-atom `Message-ID`（与 MailX 生成格式一致）；折行、注释和带引号的旧式语法不参与匹配。重复字段、多个标识、缺失或畸形值都不作为关联证据。服务器原生 `Message-ID` 保持原值并优先匹配；QQ 保存的原始标识仅作为补充证据，不覆盖服务器邮件身份。

补充标识匹配还要求服务器邮件的 `from_address` 与 outbox 中冻结的 SMTP 信封发件地址一致。如果服务器原生 `Message-ID` 已对应同账号的任何 outbox 行，就不再用补充标识去匹配另一行；不能让一封服务器邮件的扩展头部覆盖其已有的原生关联。

这项规则解决服务商给已发送副本重新分配 `Message-ID` 的情况。规则按端点启用，不等于所有使用该端点的账号类型和别名均经过真实收发验证；具体范围以[验证指引](../development/validation.md)为准。

### 3.3 存储与恢复

对账只考虑同一账号、当前邮箱来源和当前 `UIDVALIDITY` 的服务器邮件，还必须属于可选的已发送角色文件夹、远端状态为 `present`，且没有待处理的删除记录。只有已接受的 outbox 行可以匹配并释放快照；`sending`、`queued`、未处理的 `unknown` 不因头部证据自动改变发送状态。

迁移 `0024_sent_copy_identity.sql` 为 `messages` 增加 `original_message_id` 和 `sent_identity_policy`：前者保存经校验的补充标识，后者保存已处理的 `policy_key`，空串表示旧头部尚未按当前策略处理。服务器原生标识仍保存在 `message_id_header`，不会被补充标识替换。

已有已发送缓存缺少当前策略的头部时，通过既有同步通道按每个文件夹（不限已发送角色，会话线索头部也需要补抓收件箱等文件夹）、每轮最多补抓 200 个旧 UID；已缓存但不在常规同步窗口内的 UID 也参与。`policy_key` 变化后重新获取，成功处理即持久化策略标记，即使没有提取出合法补充标识也不反复抓取。失败不写完成标记，后续同步重试；重启后按持久化标记继续。补抓不下载正文、不改变已读状态、不触发发送。

头部入库、SMTP 接受和用户 `markSent` 的事务都执行对账：头部入库处理服务器副本后到，接受路径反查已缓存的服务器身份，处理服务器副本先到；启动时还会对已有缓存补做标准 `Message-ID` 的历史对账。头部入库和同步对账限定当前文件夹，接受与启动对账按账号检查当前所有合格的已发送文件夹。

同步每轮都会重新对账当前已发送文件夹的缓存，包括 `STATUS` 未变而跳过完整同步的轮次，使撤销删除后重新可见的邮件、刚被识别为已发送角色的文件夹都能补做匹配。完整同步先提交本轮已确认的远端删除，再写入新头部或补抓身份并对账，避免用本轮已确认消失的服务器副本释放快照。匹配与快照引用释放在同一事务内提交，随后由既有事件刷新查询结果。未匹配的行继续保留本机副本，不因升级或补抓失败释放内容。

## 4. OAuth 身份与授权策略边界

`core/src/auth/` 提供与品牌无关的授权码 / PKCE、一次性 loopback 会话、按账号路由的 token HTTP、grant 刷新协调、按 resource / scopes 隔离的 access token 缓存及 XOAUTH2。生产 OAuth 服务商列表目前只有 Microsoft 的两个条目（见 4.1）；通用实现及本地受控测试不代表任何实际邮箱服务商已通过真实账号验证。

边界划分：`core/src/auth/` 只放与服务商无关的流程、数据类型和 `OAuthProvider` trait（注册项、身份验证器、可选的拒绝分类钩子）；服务商的客户端 ID、端点、主机、scope 与租户规则只写在 `core/src/providers/`，由 `providers/oauth.rs` 的列表和唯一查找函数提供。同步、发信和存储主流程不出现品牌分支。同一客户端 ID 可有多个注册项（端点不同），以注册项标识区分。

未来服务商适配器负责受信 issuer、固定授权 / token 端点、精确的 IMAP / SMTP host、port、TLS 与登录身份规则、允许的 resource / scope 目标、客户端注册策略以及身份响应的后端验证，输出稳定主体和目标邮箱证据。应用客户端注册项与账号 grant 分开存储；当前没有开放 BYOA 或任意端点配置，授权会话绑定监听器前、创建、恢复和重新授权时，均须校验邮件端点与登录身份策略；恢复持久化客户端项时还必须与受信策略核对端点、客户端 / 注册标识和允许目标，不能仅凭同名 provider 信任数据库中的 URL。邮箱别名、共享邮箱和委派授权必须由适配器明确验证，不由前端传入 subject 证明。

主流程持有凭证与存储，负责有界 HTTP、TLS、代理失败不回退、scope / 到期检查、轮换落盘、grant generation 和移除 / 重授权竞态。具体适配器不自行复制 refresh token、启动独立刷新任务或绕过账号路由；浏览器授权网页的网络路径与后端账号代理分别处理。MCP 不开放客户端注册、授权会话或 token 配置能力。
### 4.1 Microsoft（`core/src/providers/microsoft.rs`）

Microsoft 的全部知识（客户端 ID、端点、主机、声明、错误码）只在这个文件及其测试里，经 `providers/oauth.rs` 列出；`auth/`、同步、发信与存储保持品牌中立，前端和桌面宿主只通过 `providerId` 与既有命令访问。

- **两个条目，共用一个公共客户端。** `microsoft-personal`（界面“Outlook”）使用 `/consumers/` 端点，SMTP 为 `smtp-mail.outlook.com:587` STARTTLS；`microsoft-work`（界面“Microsoft 365 / Exchange”）使用 `/organizations/` 端点，SMTP 为 `smtp.office365.com:587` STARTTLS。两者 IMAP 都是 `outlook.office365.com:993` 隐式 TLS，登录名为邮箱地址。回调为 `http://127.0.0.1:{port}`，无路径（注册项 `redirectPath` 为空）。
- **一枚 token 同时用于 IMAP 与 SMTP。** scope 为 `offline_access openid profile email https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send`，不设 resource；授权请求带 `prompt=select_account` 与 `login_hint`。
- **身份验证。** 先用通用 `id_token` 解析（`aud`、`exp`、`nonce`），再要求 `iss` 等于 `https://login.microsoftonline.com/{tid}/v2.0`，`tid` 与 `oid` 为标准写法的 GUID。个人条目要求 `tid` 为消费者租户 `9188040d-6c67-4c5b-b112-36a304b66dad`，工作条目要求其他租户；入错条目返回 `oauthUsePersonalEntry` / `oauthUseOrganizationEntry`，提示改用对应入口。绑定主体为 `{tid}/{oid}`，issuer 取 `iss`。邮箱地址不用先输入：`check` 返回候选地址（先 `email`，没有再 `preferred_username`，须长得像邮箱），它只是可变提示；`prove_mailbox` 以 XOAUTH2 登录本条目受信 IMAP 端点，成功才算邮箱确认。候选地址被拒或不存在时，由用户填写地址再求证一次，失败返回 `oauthMailboxRefused`。
- **管理员关闭 SMTP AUTH。** 仅 SMTP `535` 且增强状态 `5.7.139` 归为“管理员已关闭此协议”。IMAP 只返回通用的 `NO AUTHENTICATE failed`，无法区分，不猜测，也不分类。
- **Windows 凭据大小。** Microsoft refresh token 常超过 1000 个字符，Windows 由桌面端与服务商无关的分条目包装存放，见[凭据库](secret-vault.md#7-oauth-凭证的大小限制)。
- **尚未用真实账号验证。** 以上规则由受控测试和 Microsoft 文档支撑；授权、IMAP / SMTP 收发、IDLE 与各类租户策略下的实际表现仍待真实账号确认，见[验证指引](../development/validation.md)和[接入评估](../reference/microsoft-integration.md)。

## 5. 自建 Exchange 与 EWS

EWS 是独立邮件协议后端，实现在 `core/src/ews/`，通过账号的实际连接类型分派；不是依据微软品牌或邮箱后缀激活的 IMAP 兼容规则。独立「Exchange（自建服务器）」入口与 Microsoft 365 的 OAuth / IMAP/SMTP 入口分开。发现结果只是待选端点，不能自动授予密码信任。

EWS 使用共用账号路由、SecretStore、消息缓存和发送队列。协议实现提供目录、邮件和 MIME 操作；事务、来源校验、重试、撤销及副本对账由应用服务负责。暂不依赖专有邮件头猜测已发送副本身份，证据不足时保留本机快照。架构和限制见[Exchange EWS 接入](exchange-ews.md)，实测范围见[验证指引](../development/validation.md#exchange-ews-validation)。

## 6. AI 服务商：只按协议适配

本文的服务商模块针对邮箱。AI 连接只区分两种协议（OpenAI 兼容、Anthropic 兼容），流式客户端与智能体循环里没有按 AI 服务商品牌分支的判断；目录里的服务商 id 只用于名称、图标和默认地址。

- **思考（推理）按协议处理。** OpenAI 兼容读 `delta.reasoning_content` 或字符串形式的 `delta.reasoning`，Anthropic 读 `thinking` / `redacted_thinking` 块。回传规则是“服务器发来什么就原样发回什么，别的不加”：OpenAI 兼容用收到时的同一字段名放回那条助手消息，从没发过思考的服务器收不到这个字段；Anthropic 的思考块连同签名与 `data` 原样放回原位置。只回传给产生它的同一协议、连接、地址与模型。细节见 [AI Runtime](../ai-runtime/README.md) 第 3 节。
- **没有服务商专属的思考开关。** 请求不带 `thinking`、`reasoning_effort` 等只有部分服务商认识的参数；「此刻」概述同样不发。需要按服务商调整时，先确认无法在协议层解决，再按本文的模块化约定增加中立扩展点并记录验证。
- 真实服务商的实测结果写入[验证指引](../development/validation.md)；自动测试只覆盖协议样例。

## 7. 连接 TLS 实现

邮件协议连接默认使用 rustls，证书按操作系统信任库验证（`core/src/net/mod.rs`）。rustls 有意不实现有限域 DHE 和静态 RSA 密钥交换。少数服务器只提供这两类套件，rustls 与其没有共同的协议参数，握手失败，用户看到“无法建立安全连接”。这类连接改由系统 TLS 完成：macOS 为 Secure Transport，Windows 为 SChannel，Linux 为 OpenSSL。

IMAP 与 SMTP 建连时按以下顺序选择（`net::with_tls_fallback`）：

1. **登记的端点**直接使用系统 TLS。`core/src/providers/mod.rs` 的 `imap_tls_stack` 按 IMAP 主机名精确匹配（不区分 ASCII 大小写），只看实际连接的主机名，不看邮箱后缀、`providerId` 或图标；后缀相似的主机不匹配。
2. **其他端点**先用 rustls。仅当握手因协商失败而终止——服务器回 `handshake_failure`、`insufficient_security`、`protocol_version` 告警，或 rustls 判定对方不兼容（`PeerIncompatible`）——才从建立连接开始用系统 TLS 重试一次，STARTTLS 的明文阶段也重新进行。证书错误、超时、连接中断、代理失败和其他握手错误都不重试。
3. 结果按端点（小写主机名、端口）记在进程内存中，不持久化。重试成功的端点在本次运行内直接使用系统 TLS，正文连接、IDLE 和重连不再先失败一次；重试失败不记录，下次仍从 rustls 开始。用 rustls 成功过的端点在本次运行内不再回退。

约束如下：

- 两次尝试都走同一账号路由，代理失败不回退直连。
- 系统库按系统信任库验证证书，最低 TLS 1.2；握手错误与 rustls 一样只返回脱敏的通用原因。
- EWS 与 HTTPS（图片、翻译、AI、OAuth）只用 rustls，不经过此选择。
- 回退存在降级风险：主动攻击者可伪造告警，把从未用 rustls 成功连接过的端点推向系统 TLS，进而协商没有前向保密的套件。证书仍由系统验证，攻击者无法冒充服务器。登记的端点不先尝试 rustls，在本次运行内用 rustls 成功过的端点也不再回退。
- Linux 安装包因此依赖系统 `libssl3`（DEB `libssl3t64 | libssl3`，RPM `libssl.so.3()(64bit)`）。

已知只能经系统 TLS 连接的服务商端点应当登记，省去首次失败的尝试，也不给伪造告警留机会：

| 服务商 | 登记的 IMAP 主机 | 原因 |
| --- | --- | --- |
| 新浪（`core/src/providers/sina.rs`） | `imap.sina.com`、`imap.sina.cn`、`imap.vip.sina.com`、`imap.vip.sina.cn` | 只提供 TLS 1.2 的 `DHE-RSA-*` 与静态 `RSA` 套件；SMTP 端点支持 ECDHE，保持 rustls |
| 139 邮箱（`core/src/providers/mail139.rs`） | `imap.139.com` | 只提供 TLS 1.2 的静态 `RSA` 套件；SMTP 端点支持 ECDHE，保持 rustls |

Windows 上能否握手取决于系统启用的套件。Windows 11 默认启用静态 RSA（`TLS_RSA_*`），未启用 `TLS_DHE_RSA_*`，因此与上述端点协商的是静态 RSA，没有前向保密。Windows Server 2022 起默认只启用 TLS 1.3 与 ECDHE 套件，这些端点在其上无法握手；手动禁用静态 RSA 的客户端也是如此。若 Windows 11 日后默认禁用静态 RSA 而服务器仍未支持 ECDHE，需要改用其他 TLS 实现。回退到系统 TLS 的端点同样受此限制。真实端点的检查方式见[验证指引](../development/validation.md)。服务器开始支持 ECDHE 后，可从登记中移除对应主机。
