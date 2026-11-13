# EWS 邮件后端（无法启用 IMAP4 / SMTP 的 Exchange）

状态：**已实现完整收发闭环**：`core/src/ews/` 在 NTLMv2 + SOAP 之上补齐 `FindItem` / `GetItem(MimeContent)` / `UpdateItem` / `DeleteItem` / `CreateItem(SendAndSaveCopy)`，EWS 账号经 `sync/manager.rs::restart` 的独立轮询 round 收信、经 `send/worker.rs` 分派发信，账号契约与本地映射表已接线。添加账号前的端点发现见[Exchange 自动发现与邮箱设置探测](exchange-autodiscovery.md)。仍为后续方向：真实 Exchange 服务器端到端收发与三平台打包验证、EWS 的 `SyncFolderItems` 增量与删除同步（当前每轮 `FindItem` 取前 500 封）、EPA 通道绑定哈希、EWS 账号的「测试登录」按钮（EWS 模式隐藏）、共享邮箱与世纪互联运营的 Microsoft 365、OAuth 用于 EWS。IMAP / SMTP 账号的行为零改动。真实 Exchange 与平台证据见[验证记录](../development/validation.md)。

## 1. 问题与范围

### 1.1 现实约束

企业 Exchange 绝大多数只对客户端开放 443，IMAP4 与 SMTP AUTH 往往被关闭：

- Exchange 2013 / 2016 / 2019 安装后 IMAP4 与 POP3 服务默认**未启动**，需要管理员显式开启服务、设置启动类型与登录类型。
- SMTP AUTH 常被安全策略收紧，只允许特定网段或应用，避免口令喷射与开放中继。
- 部分组织只保留 OWA / EWS / ActiveSync，143、993、587 在边界被防火墙或负载均衡拦截。
- 仅启用 NTLM（集成 Windows 认证）的部署中，即使用户愿意，IMAP/SMTP 也无法用现有 `LOGIN` / `AUTH PLAIN` 登录。

这些账号的共同点是：**HTTPS 443 可用，EWS 可用**。

### 1.2 本方案覆盖

- 本地部署 Exchange 2013 及更高的邮箱，通过 EWS over HTTPS 完成收信、阅读、标记、删除与发信。
- 认证以 NTLMv2 为主；`EwsConnectionConfig.auth` 契约取值为 `ntlm` 或 `basic`，HTTP 基本认证只作为契约可选值与 Autodiscover 带凭据请求的凭据形式保留，当前收发主链路不依赖 Basic，也不支持 Kerberos。
- 复用现有本地存储、正文缓存、全文搜索、整理动作、发件队列与已发送副本语义。

### 1.3 非目标

- Exchange Online / Microsoft 365：该服务已停用基本认证，必须走 OAuth2 + XOAUTH2，已由现有 Microsoft 入口覆盖（见 [Microsoft 集成参考](../reference/microsoft-integration.md)）。
- Exchange 2010 及更早：EWS schema 版本不同，不纳入本轮。
- 日历、联系人、任务、忙闲、共享邮箱代理访问、EWS 模拟（`ExchangeImpersonation`）。
- 服务器端草稿：MailX 草稿仍只在本机，发信时才提交服务器。
- POP3、ActiveSync、MAPI/HTTP、Graph。
- `SyncFolderItems` 增量同步与删除同步：当前未实现，属后续方向（见 §3、§10）。

## 2. 协议选型

| 路线 | 覆盖 | 收发能力 | 结论 |
| --- | --- | --- | --- |
| **EWS（SOAP over HTTPS）** | 本地 Exchange 2013+、Exchange SE | 文件夹、MIME 正文、标记、删除、发信 | 采用，已实现收发闭环 |
| ActiveSync | 通常为移动设备策略放开 | WBXML，语义复杂，失败模式多 | 不采用，收益低于实现成本 |
| MAPI/HTTP | 仅 Exchange 2013+ | 全能力 | 不采用，实现规模不成比例 |
| 让管理员开 IMAP4 | 取决于组织策略 | 现有代码直接可用 | 仍是可选建议之一；自动探测现在也可切到 EWS，见[使用指南](../guides/zh-CN/README.md) |

EWS 在本地部署 Exchange 2019 与 Exchange SE 仍在支持范围内；微软对 Exchange Online 的 EWS 只保留给已迁移到 OAuth 的应用，长期方向是 Graph，但本地部署没有 Graph 替代品，因此 EWS 是本地部署唯一现实通道。这个前提不在 MailX 控制范围内，需要在验证记录中保留观察。

## 3. EWS 操作映射

请求头使用 `RequestServerVersion Version="Exchange2013"`，`soapAction` 与命名空间按 EWS 规范。下表是 MailX 能力到 EWS 操作的目标映射。

| MailX 能力 | EWS 操作 | 要点 |
| --- | --- | --- |
| 文件夹列表 | `FindFolder`（`Deep`，`FolderShape` 取 `DisplayName` / `TotalCount` / `ChildFolderCount`） | 角色文件夹用 `DistinguishedFolderId`（`inbox`、`sentitems`、`deleteditems`、`drafts`、`junkemail`、`archive`）映射到现有文件夹角色 |
| 增量收信 | `SyncFolderItems`（`SyncScope=NormalItems`，`MaxChangesReturned≤256`） | 后续方向：当前未使用，不维护 `SyncState` 游标 |
| 历史补抓 / 当前收信 | `FindItem` + `IndexedPageItemView` | 当前每轮按时间倒序取前 500 封，不分页取尽 |
| 头部与元数据 | `FindItem` 的 `ItemShape`（`BaseShape=IdOnly` + `AdditionalProperties`） | 只取主题、收发件人、`InternetMessageId`、时间、大小、`IsRead`、`HasAttachments`，不取正文 |
| 正文 | `GetItem` + `IncludeMimeContent=true` | 返回完整 MIME（base64），交给现有 MIME 解析并预渲染缓存正文 |
| 附件单独下载 | `GetAttachment` | 后续方向：仅当 MIME 路线不可用（超大邮件）时作为降级方案 |
| 标记已读 / 星标 | `UpdateItem` + `SetItemField`（`message:IsRead`、`item:Flag`） | 与现有写回队列同一语义：先本地提交，再按队列重试 |
| 删除 | `DeleteItem`（`DeleteType=MoveToDeletedItems`） | 撤销窗口与现有删除队列一致，超时后才提交 |
| 移动到文件夹 | `MoveItem` | 后续方向，当前未接线 |
| 发信 | `CreateItem`（`MessageDisposition=SendAndSaveCopy`） | 服务器自行存入「已发送」，不经过 SMTP；本地仍保留快照，按 `Message-ID` 与服务器副本对账 |
| 新邮件通知 | P1 轮询；P2 `Subscribe` + `GetEvents`（`StreamingSubscription`） | 当前沿用轮询节奏，不引入 IDLE 或订阅 |
| 服务器能力探测 | `GetServerTimeZones`（可选） | 后续方向，仅用于时间字段解释，缺失时按 UTC 处理 |

**当前实现与边界：** 已落地的操作为 `FindFolder`、每轮 `FindItem`（最多前 500 封，取头部与标记）、`GetItem(IncludeMimeContent=true)`（取完整 MIME 并预渲染缓存正文）、`UpdateItem`、`DeleteItem` 与 `CreateItem(SendAndSaveCopy)`。当前**没有** `SyncFolderItems` 增量同步，也没有服务器侧删除事件的增量同步：每轮以 `FindItem` 重新取范围内邮件，不维护 `SyncState` 游标；`MoveItem` / `GetAttachment` / `Subscribe` + `GetEvents` / `GetServerTimeZones` 仍为后续方向。表格描述的是完整目标映射，不代表每行都已接线。

请求边界（服务端不可信，必须自己设限）：

- 单次 `FindItem` 返回条目有上限；`GetItem` 批量不超过 100 个 `ItemId`；超出时继续分页，不放大单次请求。
- 单个 SOAP 响应体上限 32 MiB，MIME 响应上限 64 MiB，超出返回既有的大小错误，不截断解析。
- 超时：单次请求 30 秒；`GetEvents`（启用时）按其 `ConnectionTimeout` 计算，最长 5 分钟。
- 节流：识别 `ErrorServerBusy`（含 `BackOffMilliseconds`）与 `ErrorBatchProcessingStopped`，按服务端建议延迟重试；无建议值时沿用现有指数退避，上限 5 分钟。
- 多 CAS 部署需要 `X-AnchorMailbox: SMTP:<邮箱地址>` 头，避免邮箱被代理到错误的后端服务器而返回 `ErrorMailboxFailover`；真实拓扑下的必要性仍待验证。

## 4. 认证

### 4.1 允许的方式

| 方式 | 前提 | 状态 |
| --- | --- | --- |
| NTLMv2（NTLMSSP over HTTP） | 服务端在 `WWW-Authenticate` 提供 `NTLM` | **已实现**，见 §4.2 |
| HTTP 基本认证 | 管理员未关闭 EWS 的基本认证，且连接为 HTTPS | 契约可选值；当前收发主链路以 NTLMv2 为主，Basic 收发未接线 |
| Kerberos / `Negotiate`（SPNEGO） | 需要域内票据 | 不做；Autodiscover 可识别 `Negotiate`，但不实现 SPNEGO |
| OAuth2 | Exchange Online 专用 | 不做，后续方向 |
| 通道绑定（EPA / CBT） | 部署启用了扩展保护 | 后续方向：`MsvAvChannelBindings` 插槽已就位，哈希未接线 |

只允许 NTLMv2：拒绝服务端降级到 NTLMv1 或 LM 的协商结果，不做 DES 相关实现。用户名字段接受 `DOMAIN\user`、`user@domain` 与 `domain/user` 三种写法，按实际发送的凭证形式保存，作为端点信任的一部分。

### 4.2 NTLM 握手

1. 首次请求不带 `Authorization`，收到 `401` 且 `WWW-Authenticate: NTLM` 后发送 Type 1（`Negotiate Unicode`、`Negotiate NTLM`、`Negotiate Always Sign`）。
2. 服务端返回 `401` 与 Type 2 challenge，计算 NTLMv2 响应：`MD4(UTF-16LE(password))` 作为 NT hash，`HMAC-MD5(NT hash, UTF-16LE(user + domain))` 作为 response key，challenge 与 client challenge、时间戳、TargetInfo 组成 blob。
3. 发送 Type 3 后处理最终响应；失败按固定分类映射，不把服务端原文回传界面。
4. 中间 401 不算认证失败，只有 Type 3 之后的 401 才计入失败并触发退避。

依赖不新增：`ring` 不提供 MD4/MD5，离线环境也拿不到对应的 digest crate，所以 `core/src/ews/crypto.rs` 自己实现 MD4、MD5 与 HMAC-MD5（各以 RFC 1320/1321/2202 向量验证），`ntlm.rs` 直接调用它们。不引入完整 HTTP 认证框架；TLS 之上不做消息签名或封装。

### 4.3 连接与路由约束

- NTLM 是**连接绑定**的：Type 3 必须与 Type 1/2 使用同一条 TCP 连接。为此 EWS 客户端为每个账号维护独立的 HTTP/1.1 长连接，不复用连接池；连接断开后重新握手，不复用旧结果。
- 连接必须走账号路由：`direct` / `inherit` / `custom`（SOCKS5 或 HTTP CONNECT）。自定义代理失败返回 `proxy` 错误，**不回退直连**，与现有协议一致。
- 只允许 HTTPS，证书按系统信任库校验；不做证书固定，不提供“忽略证书错误”开关。
- 启用扩展保护（EPA / CBT）的部署需要在 NTLMv2 的 `MsvAvChannelBindings` 中带上 TLS 通道绑定哈希；当前未接线，这类部署的登录仍待验证，属后续方向，握手失败时明确报错而不静默降级。

### 4.4 失败分类

| 服务端表现 | 映射 | 用户可见 |
| --- | --- | --- |
| `401` 且未提供凭证 | 需要凭证 | 提示需要密码或域账号 |
| Type 3 后 `401` / `403` | 凭证被拒 | 凭证无效，不重试 |
| 证书错误 | TLS 失败 | 证书不匹配 |
| 代理连接失败 | 代理失败 | 代理不可用，不回退 |
| `ErrorServerBusy` / 5xx | 暂时失败 | 按退避重试 |
| 未知 SOAP 错误 | 内部错误 | 固定文案，不含 XML |

## 5. 架构与模块

### 5.1 后端分派

`core/src/sync/` 与 `core/src/send/` 原本直接调用 IMAP 与 SMTP。本次实现没有引入预建的 `MailBackend` 中立接口，而是按账号分派到两条互不共享连接对象的链路：

- IMAP / SMTP 账号：完全沿用 `sync/manager.rs` 的账号轮次、`sync/{folders,messages,bodies,writeback}` 与 `send/worker.rs` 的 SMTP 路径，行为零改动。
- EWS 账号：`sync/manager.rs::restart` 按账号分派到独立的轮询 round（`sync/ews.rs`）；`send/worker.rs` 按账号分派到 `CreateItem(SendAndSaveCopy)`。

EWS 轮询 round 沿用现有 `Cadence`：每轮 `FindFolder` → `FindItem` → 写 `MessageRecord` / flags → 取 MIME 并预渲染缓存正文 → 处理本地标记写回。**不启动 IMAP IDLE，也不启动 IMAP 标记写回**；按需正文与预览读取因此复用本地缓存，不再连 IMAP。中立接口（枚举分派、返回中立类型、IMAP 侧行为等价适配层）仍是可选的后续重构方向，不是当前实现的组成部分。

### 5.2 目录布局

```text
core/src/ews/
  mod.rs          # 后端入口：Endpoint、EwsClient、动作分派
  http.rs         # 连接绑定的 HTTP/1.1（hyper，不池化）+ 401 认证循环
  ntlm.rs         # NTLMv2 计算与 Type1/2/3 消息、用户名拆分
  crypto.rs       # MD4 / MD5 / HMAC-MD5（ring 不含，自实现）
  soap.rs         # 信封构造与有界响应解析（手写扫描器，拒绝 DTD）
  ...             # FindFolder / FindItem / GetItem(MimeContent) / UpdateItem /
                  # DeleteItem / CreateItem(SendAndSaveCopy) 的实现与 ID 映射
```

认证没有单独 `auth/` 目录（只有 NTLM 一种，Basic 仅作契约可选值）；XML 没有引入 `quick-xml`，沿用 Autodiscover 已验证的手写扫描器（按本地名匹配、拒绝 `<!DOCTYPE>`，因而天然不做实体展开）。上述操作统一由 `core/src/ews/` 提供，不再按「待建」拆分。

### 5.3 分派

账号的连接种类由账号契约决定：带 `ews` 连接配置的是 EWS 账号，只有 `imap` + `smtp` 的是现有账号。`sync/manager.rs::restart` 与 `send/worker.rs` 据此选择实现；两条链路不共享连接对象。EWS 账号仍保留 `imap` / `smtp` 两行作为存储与发件箱兼容占位，不据此发起 IMAP / SMTP 连接。

## 6. 数据模型与迁移

### 6.1 连接与端点

账号契约以可空的 `EwsConnectionConfig`（`url` 为 HTTPS、`auth` 为 `ntlm` | `basic`、`username`、`certPrincipalName` 可空）表达 EWS 连接，不改写 `mail_connections` 的 `protocol` 检查（仍只接受 `imap` / `smtp`）。EWS 账号仍保留 `imap` / `smtp` 两行作为存储与发件箱兼容占位；这两行不用于发起 IMAP / SMTP 连接。EWS 端点另存于 `exchange_endpoints`，由迁移 `0030_exchange_ews.sql` 引入，见[数据库迁移](database-migrations.md)。

### 6.2 远端身份与本地映射

IMAP 的 `messages` 以 `(folder_id, uid_validity, uid)` 为身份；EWS 没有 UID，只有 `ItemId` 与 `ChangeKey`。当前实现**不改动 `messages`**，用映射表把远端身份翻译成稳定的本地 uid：

- `ews_items`：`ItemId` → 稳定本地 uid 的映射，让上层 `MessageRecord`、搜索索引、分页、删除队列、归档表、已发送副本对账都沿用既有本地身份，不需要知道 EWS 存在。
- `ews_folder_state`：记录每个文件夹的 uid 水位（分配器），保证同一 `ItemId` 始终映射到同一本地 uid。
- `exchange_endpoints`：账号的 EWS 端点记录；账号删除时一并清理这三张表。

`folder_sync_cursors` 保持 IMAP 专用，EWS 状态放在独立表，避免把两套游标语义塞进同一列。当前没有 `SyncState` 增量游标，相关字段留待后续方向。

### 6.3 迁移规则

- 只追加迁移，不改写历史 SQL；清单条目与 SQL 同时提交，编号按落地时的清单顺延。`0030_exchange_ews.sql` 一次引入上述三张表，属于 append-only 结构变更。
- 迁移不联网、不读写 `SecretStore`；旧账号行为不变：没有 `ews` 连接的账号完全走原路径。
- 账号删除时清理 `exchange_endpoints`、`ews_items`、`ews_folder_state` 三张表，随既有移除事务一并处理。
- 测试必须使用隔离目录，禁止在真实数据目录上试迁移（一次被回退的实验迁移曾把正式库升到不存在的版本，导致应用拒绝启动）。

## 7. 与现有子系统的接线

| 子系统 | 影响 | 处理方式 |
| --- | --- | --- |
| 同步计划 / 节奏 | 按账号连接种类选择链路 | `sync/manager.rs::restart` 把 EWS 账号分派到独立轮询 round（`sync/ews.rs`）；沿用 `Cadence`，不启动 IMAP IDLE 与 IMAP 标记写回 |
| 文件夹与角色 | EWS 用 `DistinguishedFolderId` | 映射到现有角色，识别不出角色时保留为普通文件夹 |
| 正文缓存 | `GetItem` 返回 MIME | 每轮取 MIME 并预渲染缓存正文；复用现有 MIME 解析、blob 存储、按需下载与隔离渲染，按需正文与预览复用本地缓存，不再连 IMAP |
| 全文搜索 | 无 | 索引来源仍是入库后的头部与正文 |
| 标记写回 / 删除 | `UpdateItem` / `DeleteItem` | 复用现有队列、撤销窗口与重试；`MoveItem` 仍为后续方向 |
| 本机归档 | 无 | 保持仅本机状态 |
| 发件队列 | 无 SMTP，走 `CreateItem` | 成功包装为 `Accepted` 以复用现有 outbox 状态机、已发送副本对账与草稿清理；请求写出后失败记为 `Unknown`（可能已被接收，绝不自动重发） |
| 已发送副本对账 | 服务器自行保存副本 | 仍按 `Message-ID` 对账；EWS 账号不需要 SMTP 兼容规则 |
| 新邮件通知 | 轮询 | 沿用轮询节奏，不引入 IDLE 或订阅 |
| 本地 MCP | 无 | 工具层通过同一应用服务，不感知后端 |
| 启动准入 | 无 | 数据库迁移仍先于一切联网任务 |

## 8. 安全约束

- SOAP 响应与 MIME 都是不可信输入：解析器禁用 DTD 与外部实体，限制元素深度、属性数量与总长度，拒绝畸形响应而不是尽力猜测。
- 服务端错误文本、XML 片段、`ItemId` 只进入脱敏诊断，不返回界面，也不写入普通日志。
- 凭证信任仍按端点：EWS 端点身份包含 `https` + 主机 + 端口 + 路径 + 用户名 + 认证方式；服务器改地址、改登录名或改认证方式后旧密码不会被自动使用，必须重新确认。
- 不提升扩展权限：不使用 `ExchangeImpersonation`，不请求其他邮箱，不写入 `SendAs`。
- HTML 正文仍走现有隔离 WebView，`frame-src mailx-body:` 与 CSP 不变；EWS 不改变任何前端权限。
- NTLM 凭证不缓存明文：与现有密码一致，只保存在 `SecretStore`，内存中零化。

## 9. 界面与契约

- 服务商入口：通用「IMAP/SMTP」入口（`GENERIC_PROVIDER`）输入邮箱地址后先做只读自动探测；发现 EWS 端点即切到 Exchange(EWS) 通道，端点、认证方式与登录名来自[自动发现](exchange-autodiscovery.md)，不再要求用户手填主机与路径。Microsoft 浏览器登录入口仍只覆盖 Outlook.com / Microsoft 365，不处理自建 Exchange。
- 契约：`ConnectionInput` / `AccountConfigInput` / `Account` 增加可空 `ews: EwsConnectionConfig | null`（`url` 为 https、`auth` 为 `ntlm` | `basic`、`username`、`certPrincipalName` 可空）。字段均为可空，旧客户端与旧账号不受影响；EWS 账号仍保留 `imap` / `smtp` 占位，密码只用 `shared` 模式，详见[账号配置](account-configuration.md)与[应用 API](../contracts/app-api.md)。
- 端点信任：保存时把 EWS 端点加入凭证信任范围，URL 或登录名变更即不再信任，需重新确认。
- 测试登录：EWS 模式隐藏「测试登录」按钮（不提供该能力）。
- 文案：中英文同步补齐自动探测、Exchange(EWS) 通道、端点信息与 NTLM 失败提示。

## 10. 已实现与后续方向

| 范围 | 内容 |
| --- | --- |
| 已实现（传输与认证） | SOAP 信封、连接绑定的 HTTP/1.1、有界响应解析、自实现 MD4/MD5/HMAC-MD5 与 NTLMv2 Type1/2/3 |
| 已实现（收信） | `FindFolder`、每轮 `FindItem`（前 500 封）、`GetItem(IncludeMimeContent)`、`UpdateItem`、`DeleteItem`、本地 MIME 解析与预渲染缓存正文、独立轮询 round、本地标记写回 |
| 已实现（发信） | `CreateItem(SendAndSaveCopy)`，包装为 `Accepted` 复用 outbox 状态机、已发送副本对账与草稿清理 |
| 已实现（账号与存储） | 添加账号自动探测、可空 `ews` 契约、`0029` 三张表与账号删除清理 |
| 后续方向 | 真实 Exchange 服务器端到端收发与三平台打包验证；`SyncFolderItems` 增量与删除同步（当前每轮 `FindItem` 前 500 封）；EPA 通道绑定哈希；EWS 账号「测试登录」按钮；共享邮箱与世纪互联运营的 Microsoft 365；OAuth 用于 EWS；`MoveItem`、`GetAttachment`、流式订阅、HTTP 基本认证收发 |

原设计把 HTTP 基本认证排在 NTLM 之前，但被动探测的目标服务器在 EWS 上只提供 `Negotiate` 与 `NTLM`（只有 Autodiscover 给 `Basic`），先做 Basic 会得到一条走不通的路径，因此 NTLMv2 与 EWS 传输层一起落地。基本认证仍作为契约可选值保留，给只开了 Basic 的老部署作后续补充。

数据库迁移只在需要本地映射时引入一次，不改写历史 SQL。

## 11. 验证与未验证

自动测试覆盖的协议向量、SOAP 往返、NTLMv2 计算、迁移幂等与旧行兼容等证据集中记录在[验证记录](../development/validation.md)；本文不复制测试进度与结果，也不列出启动命令。

**未做真机验证 / 待验证：** 真实 Exchange 服务器上的端到端收发闭环、三平台打包验证、Exchange 2013 / 2016 / 2019 的 EWS 行为差异、`X-AnchorMailbox` 在多 CAS 部署中的必要性、启用 EPA 的部署、通过企业代理访问 EWS 的稳定性、`SendAndSaveCopy` 后副本与本地快照的对账时机，以及 `SyncFolderItems` 增量与删除同步。上述项目不能写成已验证；记录中的主机名、证书主体与账号一律为占位，真实标识不入仓库。

## 12. 风险与回退

- **协议前景**：微软已对 Exchange Online 停用基本认证并推动 Graph，本地部署的 EWS 仍受支持但没有长期承诺。方案把 EWS 实现隔离在 `core/src/ews/`，上层按账号分派；如果未来必须迁移（例如 Graph 或新的服务器协议），替换的是一个后端实现，而不是整个同步与发送流程。
- **认证收紧**：只留 Kerberos 的部署仍然无法接入，属于已知限制，需在指南中说明。
- **连接成本**：NTLM 要求每账号独占长连接，账号数很多的场景需要限制并发（沿用现有 worker 模型，每账号一个任务）。
- **回退**：功能以连接方式区分，EWS 账号出问题时可以删除该账号重新按 IMAP 配置，不影响其他账号与本地数据；迁移只新增小表，不做数据搬迁，回退由备份恢复承担。
