# EWS 邮件后端（无法启用 IMAP4 / SMTP 的 Exchange）

状态：**方案保留；认证与传输层已实现，账号模型未接线**。本文保留本地部署 Exchange 走 Exchange Web Services 的完整方案：问题边界、协议选型、认证设计、模块划分、数据迁移与验证口径。`core/src/ews/` 已在 v0.2.0 之上完成 NTLMv2 登录并读取文件夹（见 §4、§5.2、§10），收信、发信操作与账号表单入口未实现。当前版本对 Microsoft 的支持是 Outlook.com / Microsoft 365 的 OAuth2 浏览器登录（见 [Microsoft 集成参考](../reference/microsoft-integration.md)），账号界面明确提示"公司自己搭建的 Exchange 服务器不在这里支持，请改用 IMAP/SMTP 手动配置"——本方案补的就是这个缺口。

## 1. 问题与范围

### 1.1 现实约束

企业 Exchange 绝大多数只对客户端开放 443，IMAP4 与 SMTP AUTH 往往被关闭：

- Exchange 2013 / 2016 / 2019 安装后 IMAP4 与 POP3 服务默认**未启动**，需要管理员显式开启服务、设置启动类型与登录类型。
- SMTP AUTH 常被安全策略收紧，只允许特定网段或应用，避免口令喷射与开放中继。
- 部分组织只保留 OWA / EWS / ActiveSync，143、993、587 在边界被防火墙或负载均衡拦截。
- 仅启用 NTLM（集成 Windows 认证）的部署中，即使用户愿意，IMAP/SMTP 也无法用现有 `LOGIN` / `AUTH PLAIN` 登录。

这些账号的共同点是：**HTTPS 443 可用，EWS 可用**。

### 1.2 本方案覆盖

- 本地部署 Exchange 2013 及更高的邮箱，通过 EWS over HTTPS 完成收信、阅读、标记、删除、移动与发信。
- 认证支持 NTLMv2（已实现）与 HTTP 基本认证（管理员仍允许时，未实现），不支持 Kerberos。
- 复用现有本地存储、正文缓存、全文搜索、整理动作、发件队列与已发送副本语义。

### 1.3 非目标

- Exchange Online / Microsoft 365：该服务已停用基本认证，必须走 OAuth2 + XOAUTH2，已由现有 Microsoft 入口覆盖（见 [Microsoft 集成参考](../reference/microsoft-integration.md)）。
- Exchange 2010 及更早：EWS schema 版本不同，不纳入本轮。
- 日历、联系人、任务、忙闲、共享邮箱代理访问、EWS 模拟（`ExchangeImpersonation`）。
- 服务器端草稿：MailX 草稿仍只在本机，发信时才提交服务器。
- POP3、ActiveSync、MAPI/HTTP、Graph。

## 2. 协议选型

| 路线 | 覆盖 | 收发能力 | 结论 |
| --- | --- | --- | --- |
| **EWS（SOAP over HTTPS）** | 本地 Exchange 2013+、Exchange SE | 文件夹、增量同步、MIME 正文、标记、删除、移动、发信 | 采用 |
| ActiveSync | 通常为移动设备策略放开 | WBXML，语义复杂，失败模式多 | 不采用，收益低于实现成本 |
| MAPI/HTTP | 仅 Exchange 2013+ | 全能力 | 不采用，实现规模不成比例 |
| 让管理员开 IMAP4 | 取决于组织策略 | 现有代码直接可用 | 保留为首选建议，见[使用指南](../guides/zh-CN/README.md) |

EWS 在本地部署 Exchange 2019 与 Exchange SE 仍在支持范围内；微软对 Exchange Online 的 EWS 只保留给已迁移到 OAuth 的应用，长期方向是 Graph，但本地部署没有 Graph 替代品，因此 EWS 是本地部署唯一现实通道。这个前提不在 MailX 控制范围内，需要在验证记录中保留观察。

## 3. EWS 操作映射

请求头使用 `RequestServerVersion Version="Exchange2013"`，`soapAction` 与命名空间按 EWS 规范。下表是 MailX 能力到 EWS 操作的映射。

| MailX 能力 | EWS 操作 | 要点 |
| --- | --- | --- |
| 文件夹列表 | `FindFolder`（`Deep`，`FolderShape` 取 `DisplayName` / `TotalCount` / `ChildFolderCount`） | 角色文件夹用 `DistinguishedFolderId`（`inbox`、`sentitems`、`deleteditems`、`drafts`、`junkemail`、`archive`）映射到现有文件夹角色 |
| 增量收信 | `SyncFolderItems`（`SyncScope=NormalItems`，`MaxChangesReturned≤256`） | 每轮返回 `Created` / `Updated` / `Deleted` 与新的 `SyncState`；`IncludesLastItemInRange=false` 时继续同步直到取尽 |
| 历史补抓 | `FindItem` + `IndexedPageItemView` | 仅首次同步或游标失效时使用，按时间倒序分页 |
| 头部与元数据 | `SyncFolderItems` 的 `ItemShape`（`BaseShape=IdOnly` + `AdditionalProperties`） | 只取 `item:Subject`、`message:From`、`message:ToRecipients`、`message:CcRecipients`、`message:InternetMessageId`、`item:DateTimeReceived`、`item:Size`、`message:IsRead`、`item:HasAttachments`，不取正文 |
| 正文 | `GetItem` + `IncludeMimeContent=true` | 返回完整 MIME（base64），交给现有 MIME 解析与按需缓存；内嵌图片与附件随 MIME 一起得到 |
| 附件单独下载 | `GetAttachment` | 仅当 MIME 路线不可用（超大邮件）时作为降级方案 |
| 标记已读 / 星标 | `UpdateItem` + `SetItemField`（`message:IsRead`、`item:Flag`） | 与现有写回队列同一语义：先本地提交，再按队列重试 |
| 删除 | `DeleteItem`（`DeleteType=MoveToDeletedItems`） | 撤销窗口与现有删除队列一致，超时后才提交 |
| 移动到文件夹 | `MoveItem` | 对应现有文件夹移动动作 |
| 发信 | `CreateItem`（`MessageDisposition=SendAndSaveCopy`） | 服务器自行存入「已发送」，不经过 SMTP；本地仍保留快照，按 `Message-ID` 与服务器副本对账 |
| 新邮件通知 | P1 轮询；P2 `Subscribe` + `GetEvents`（`StreamingSubscription`） | P1 直接沿用现有 `Cadence` 轮询节奏，不引入新机制 |
| 服务器能力探测 | `GetServerTimeZones`（可选） | 仅用于时间字段解释，缺失时按 UTC 处理 |

请求边界（服务端不可信，必须自己设限）：

- 单次 `SyncFolderItems` 与 `FindItem` 返回条目上限 256；`GetItem` 批量不超过 100 个 `ItemId`；超出时继续分页，不放大单次请求。
- 单个 SOAP 响应体上限 32 MiB，MIME 响应上限 64 MiB，超出返回既有的大小错误，不截断解析。
- 超时：单次请求 30 秒；`GetEvents`（启用时）按其 `ConnectionTimeout` 计算，最长 5 分钟。
- 节流：识别 `ErrorServerBusy`（含 `BackOffMilliseconds`）与 `ErrorBatchProcessingStopped`，按服务端建议延迟重试；无建议值时沿用现有指数退避，上限 5 分钟。
- 多 CAS 部署需要 `X-AnchorMailbox: SMTP:<邮箱地址>` 头，避免邮箱被代理到错误的后端服务器而返回 `ErrorMailboxFailover`。

## 4. 认证

### 4.1 允许的方式

| 方式 | 前提 | 状态 |
| --- | --- | --- |
| NTLMv2（NTLMSSP over HTTP） | 服务端在 `WWW-Authenticate` 提供 `NTLM` | **已实现**（2026-11-12），见 §4.2 |
| HTTP 基本认证 | 管理员未关闭 EWS 的基本认证，且连接为 HTTPS | 未实现。实测的 Exchange 2016 在 `/EWS/Exchange.asmx` 上只给 `Negotiate` 与 `NTLM`，只有 Autodiscover 才给 `Basic`，所以先做 NTLM |
| Kerberos / `Negotiate`（SPNEGO） | 需要域内票据 | 不做 |
| OAuth2 | Exchange Online 专用 | 见 [Outlook / Exchange 接入](outlook-exchange.md#3-outlookcom-与-microsoft-365需要的-oauth2-参数未实现) |
| 通道绑定（EPA / CBT） | 部署启用了扩展保护 | 字段与算法位置已留出，加密未接线（§4.3） |

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
- 启用扩展保护（EPA / CBT）的部署需要在 NTLMv2 的 `MsvAvChannelBindings` 中带上 TLS 通道绑定哈希，否则握手被拒。P2 一并实现，无法计算时明确报错，不静默降级。

### 4.4 失败分类

| 服务端表现 | 映射 | 用户可见 |
| --- | --- | --- |
| `401` 且未提供凭证 | 需要凭证 | 提示需要密码或域账号 |
| Type 3 后 `401` / `403` | 凭证被拒 | 凭证无效，不重试 |
| 证书错误 | TLS 失败 | 证书不匹配 |
| 代理连接失败 | 代理失败 | 代理不可用，不回退 |
| `ErrorServerBusy` / 5xx | 暂时失败 | 按退避重试 |
| `ErrorInvalidSyncStateData` | 游标失效 | 触发该文件夹重建，不报错给用户 |
| 未知 SOAP 错误 | 内部错误 | 固定文案，不含 XML |

## 5. 架构与模块

### 5.1 传输抽象

当前 `core/src/sync/` 与 `core/src/send/` 直接调用 IMAP 与 SMTP：`sync/manager.rs` 持有 IMAP 会话，`sync/{folders,messages,bodies,writeback}` 按 IMAP 语义实现，`send/worker.rs` 走 SMTP。EWS 是第二个实现，按仓库约定（有多个实现时才引入接口，不预建框架），接线时引入中立接口：

```text
core/src/mail/backend.rs（接线时新建）
  enum MailBackend { Imap(Session) | Ews(EwsClient) }
  folders()                                 # 文件夹列表
  folder_view(folder)                       # 计数 + 必要时打开该文件夹
  mailbox(folder)                           # 身份信息（UIDVALIDITY 对应 EWS 的 generation）
  uids_since(date)                          # 范围内全部 uid
  headers(uids) / flags(uids)               # 有界批次的头部与标记
  identity_policy()                         # 头部解释所需的服务商策略
  logout()
```

接口尚未落地，落地时只覆盖主流程真正需要的动作，不照搬 IMAP 概念，再按此边界把 `sync/manager.rs` 的账号轮次与 `sync/messages.rs` 的按文件夹同步改为调用 `MailBackend`，其余路径（正文、预览、写回、删除、发送、IDLE）逐阶段迁入。

约束：

- 接口返回**中立类型**，不暴露 IMAP 的 `UIDVALIDITY`、`UID` 或 EWS 的 `ItemId`；远端身份由后端自己维护到本地 `(folder_id, uid_validity, uid)` 的映射。
- 分派用**枚举**而不是 `dyn Trait`：trait 里的 `async fn` 不是对象安全的，而仓库不打算为此引入 `async-trait`。新后端是新增一个变体，调用方不变。
- IMAP 侧是**行为等价的适配层**：不改变协商、窗口、批大小与写回逻辑，全量测试证明没有回归。
- EWS 侧实现同类方法，复用同一批上层能力（存储事务、搜索索引、事件、退避、日志脱敏）。

### 5.2 目录布局

```text
core/src/ews/
  mod.rs          # 后端入口：Endpoint、EwsClient、动作分派
  http.rs         # 连接绑定的 HTTP/1.1（hyper，不池化）+ 401 认证循环
  ntlm.rs         # NTLMv2 计算与 Type1/2/3 消息、用户名拆分
  crypto.rs       # MD4 / MD5 / HMAC-MD5（ring 不含，自实现）
  soap.rs         # 信封构造与有界响应解析（手写扫描器，拒绝 DTD）
  folders.rs      # 待建：SyncFolderHierarchy → 文件夹记录
  sync.rs         # 待建：SyncFolderItems / FindItem → 头部批次与游标
  bodies.rs       # 待建：GetItem IncludeMimeContent → MIME
  writeback.rs    # 待建：UpdateItem / DeleteItem / MoveItem
  send.rs         # 待建：CreateItem SendAndSaveCopy
  mapping.rs      # 待建：DistinguishedFolderId、ItemId、时间与大小的规范化
```

与原文的差别：认证没有单独 `auth/` 目录（只有 NTLM 一种，且 Basic 未实现）；XML 没有引入 `quick-xml`，沿用 Autodiscover 已验证的手写扫描器（按本地名匹配、拒绝 `<!DOCTYPE>`，因而天然不做实体展开）；`FindFolder` 暂时放在 `soap.rs`，等 `folders.rs` 落地再拆分。

### 5.3 分派

账号的连接种类由 `mail_connections` 中存在的行决定：只有 `ews` 行的是 EWS 账号，只有 `imap` + `smtp` 行的是现有账号（双协议仍要求成对出现）。`sync_plan` 在生成计划时给出 `backend` 字段，`sync/manager.rs` 与 `send/worker.rs` 据此选择实现；两条链路不共享连接对象。

## 6. 数据模型与迁移

### 6.1 连接记录

`mail_connections` 当前的 `protocol` 检查只接受 `imap` / `smtp`，`tls` 只接受 `implicit` / `startTls`。EWS 需要：

- `protocol` 增加 `ews`（一个账号一行）。
- 新增可空列 `ews_path`（默认 `/EWS/Exchange.asmx`，允许负载均衡使用自定义路径）与 `auth_scheme`（`basic` / `ntlm`）。
- 约束：`ews` 行要求 `ews_path`、`auth_scheme` 非空且 `tls='implicit'`；`imap` / `smtp` 行要求两列为空。

SQLite 不能修改 `CHECK`，只能重建表；沿用既有表重建迁移的模式（如 `0024_sent_copy_identity.sql`、`0025_oauth_bindings.sql`），在迁移中搬结构、不改数据，旧行按新约束的 `imap` / `smtp` 分支通过校验。凭证仍由既有凭证记录（`kind='password'`）与 `SecretStore` 管理，不新增认证类型。新迁移从 `0029` 起编号。

### 6.2 同步游标与远端身份

IMAP 的 `messages` 以 `(folder_id, uid_validity, uid)` 为身份；EWS 没有 UID，只有 `ItemId` 与 `ChangeKey`。方案是**不改动 `messages`**，用映射表把远端身份翻译成现有身份：

- `ews_folder_state(folder_id, sync_state, generation, next_uid, last_sync_at)`：`sync_state` 是 EWS 游标；`generation` 对应 `uid_validity`（游标失效时递增，等价于 IMAP 的 UIDVALIDITY 重置）；`next_uid` 为该文件夹单调分配器。
- `ews_items(folder_id, item_id, change_key, uid)`：`UNIQUE(folder_id, item_id)` 与 `UNIQUE(folder_id, uid)`，`ItemId` 变化（移动、重建）时更新映射而不是新增行。

这样搜索索引、分页、删除队列、归档表、已发送副本对账都不需要知道 EWS 存在。`folder_sync_cursors` 保持 IMAP 专用，EWS 状态放在独立表，避免把两套游标语义塞进同一列。

### 6.3 迁移规则

- 只追加迁移，不改写历史 SQL；清单条目与 SQL 同时提交，编号按落地时的清单顺延。
- 迁移不联网、不读写 `SecretStore`；重建表只搬结构。
- 测试必须使用 `MAILX_DATA_DIR` 指向的隔离目录，禁止在真实数据目录上试迁移（一次被回退的实验迁移曾把正式库升到不存在的版本，导致应用拒绝启动）。
- 迁移后旧账号行为不变：没有 `ews` 行的账号完全走原路径。

## 7. 与现有子系统的接线

| 子系统 | 影响 | 处理方式 |
| --- | --- | --- |
| 同步计划 / 节奏 | 需要按连接种类选择后端 | `sync_plan` 增加 `backend`，`manager` 分派；轮询节奏、暂停、前台加速不变 |
| 文件夹与角色 | EWS 用 `DistinguishedFolderId` | `mapping.rs` 映射到现有角色，识别不出角色时保留为普通文件夹 |
| 正文缓存 | `GetItem` 返回 MIME | 复用现有 MIME 解析、blob 存储、按需下载与隔离渲染，不改 CSP |
| 全文搜索 | 无 | 索引来源仍是入库后的头部与正文 |
| 标记写回 / 删除 / 移动 | `UpdateItem` / `DeleteItem` / `MoveItem` | 复用现有队列、撤销窗口与重试；后端只负责提交动作 |
| 本机归档 | 无 | 保持仅本机状态 |
| 发件队列 | 无 SMTP，走 `CreateItem` | outbox 状态机不变；`SendAndSaveCopy` 成功即视为已接受 |
| 已发送副本对账 | 服务器自行保存副本 | 仍按 `Message-ID` 对账；EWS 账号不需要 SMTP 兼容规则 |
| 新邮件通知 | P1 轮询，P2 订阅 | P1 不发桌面通知之外的新机制；P2 处理 `GetEvents` 超时与重新订阅 |
| 本地 MCP | 无 | 工具层通过同一应用服务，不感知后端 |
| 启动准入 | 无 | 数据库迁移仍先于一切联网任务 |

## 8. 安全约束

- SOAP 响应与 MIME 都是不可信输入：解析器禁用 DTD 与外部实体，限制元素深度、属性数量与总长度，拒绝畸形响应而不是尽力猜测。
- 服务端错误文本、XML 片段、`ItemId` 只进入脱敏诊断，不返回界面，也不写入普通日志。
- 凭证信任仍按端点：EWS 端点身份包含 `https` + 主机 + 端口 + 路径 + 用户名 + 认证方式；服务器改地址或改认证方式后旧密码不会被自动使用，必须重新确认。
- 不服力扩展权限：不使用 `ExchangeImpersonation`，不请求其他邮箱，不写入 `SendAs`。
- HTML 正文仍走现有隔离 WebView，`frame-src mailx-body:` 与 CSP 不变；EWS 不改变任何前端权限。
- NTLM 凭证不缓存明文：与现有密码一致，只保存在 `SecretStore`，内存中零化。

## 9. 界面与契约

- 服务商入口：新增 `exchange`（本地部署）预设，带“连接方式”选择（IMAP/SMTP 或 EWS）；EWS 分支显示端点路径与认证方式，隐藏 IMAP/SMTP 字段。当前账号界面的 Microsoft 入口只覆盖 Outlook.com / Microsoft 365，并提示自建 Exchange 请改用 IMAP/SMTP 手动配置——EWS 落地后同步更新该提示与[Microsoft 集成参考](../reference/microsoft-integration.md)。
- 服务器信息获取：远端当前没有 Autodiscover 客户端；EWS 分支首期让用户手动填写主机与路径（默认 `/EWS/Exchange.asmx`），Autodiscover（POX `EwsUrl`）作为后续增强，不阻塞主链路。
- 契约：`ConnectionDraft` 增加显式 `kind`（`imap` | `ews`）与 `ews` 子结构。字段均为可选，旧客户端与旧账号不受影响。
- 凭据输入不变：仍是账号密码，NTLM 时用户名可用 `DOMAIN\user`。
- 文案：中英文同步补齐连接方式、端点、认证方式、NTLM 失败与“需要管理员启用 IMAP 或 EWS”的提示，`bun run i18n:check` 为门禁。

## 10. 分阶段

| 阶段 | 内容 | 完成标准 |
| --- | --- | --- |
| P1 ✅（2026-11-12） | EWS 传输层 + NTLMv2：SOAP 信封、连接绑定的 HTTP/1.1、`FindFolder`、有界响应解析、自实现 MD4/MD5/HMAC-MD5 与 Type1/2/3 | MS-NLMP 4.2.4.1.1 与 RFC 向量通过；一条连接上的完整握手由假服务器验证；模块独立于账号模型，`cargo test -p mailx-core` 全绿 |
| P2 | 收信与发信操作：`SyncFolderItems`、`GetItem`、`UpdateItem`、`DeleteItem`、`CreateItem SendAndSaveCopy`，以及 `ews_folder_state` / `ews_items` 迁移与 `MailBackend` 接线 | 真实 Exchange 账号可完成收发闭环；错误分类与节流生效 |
| P3 | 账号模型与表单：`mail_connections.protocol='ews'`、`exchange` 预设的连接方式选择、NTLM 用户名提示、双语文案 | 真实账号从账号表单完成端到端配置 |
| P4 | 通道绑定（EPA）、流式订阅、EWS 附件降级、文件夹移动完善、Autodiscover `EwsUrl` | 启用 EPA 的部署可登录；通知延迟与 IMAP IDLE 相当 |

顺序说明：原设计把 HTTP 基本认证排在 NTLM 之前，但实测的目标服务器（Exchange 2016）在 EWS 上**只提供 Negotiate 与 NTLM**，先做 Basic 会得到一条永远走不通的路径，因此 NTLMv2 与 EWS 传输层一起落地。基本认证仍可作为后续补充，给那些只开了 Basic 的老部署用。

每阶段独立可发布，前一阶段不依赖后一阶段的字段；数据库迁移只在 P2 引入一次。

## 11. 验证与未验证

自动测试需覆盖：SOAP 请求与响应往返（固定样例，不含真实邮件）、命名空间与必填元素、分页与 `IncludesLastItemInRange` 处理、游标失效重建、节流退避、401 分类、NTLMv2 计算向量（RFC 与公开测试向量）、通道绑定哈希、MIME 上限、`DistinguishedFolderId` 映射、迁移的旧行兼容与幂等、传输抽象下 IMAP 行为不变。

已通过：MD4/MD5/HMAC-MD5 的 RFC 1320/1321/2202 向量；MS-NLMP 4.2.4.1.1 的 NTLMv2 响应；`Type 2` 解析的越界与降级拒绝；`Type 3` 字段布局；一条 TCP 连接上 `401 → Type1 → 401(挑战) → Type3 → 200` 的完整序列；错误密码映射为“凭证被拒”；无 NTLM 时映射为“不支持”；SOAP 的 DTD 拒绝、前缀无关、自闭合元素、服务端错误分类。

真机自测入口（不需要凭据入库）：

```text
MAILX_EWS_HOST=mail.contoso.com MAILX_EWS_USER='contoso.com\alice' MAILX_EWS_PASSWORD=… cargo test -p mailx-core --lib a_real_exchange -- --ignored --nocapture
```

它只打印文件夹名称与计数。

尚未验证且必须在真实环境确认：Exchange 2013 / 2016 / 2019 的 EWS 行为差异、`X-AnchorMailbox` 在多 CAS 部署中的必要性、启用 EPA 的部署、通过企业代理访问 EWS 的稳定性、`SendAndSaveCopy` 后副本与本地快照的对账时机。记录写入[验证记录](../development/validation.md)。

## 12. 风险与回退

- **协议前景**：微软已对 Exchange Online 停用基本认证并推动 Graph，本地部署的 EWS 仍受支持但没有长期承诺。方案把 EWS 实现隔离在 `core/src/ews/`，上层只依赖中立接口；如果未来必须迁移（例如 Graph 或新的服务器协议），替换的是一个后端实现，而不是整个同步与发送流程。
- **认证收紧**：只留 Kerberos 的部署仍然无法接入，属于已知限制，需在指南中说明。
- **连接成本**：NTLM 要求每账号独占长连接，账号数很多的场景需要限制并发（沿用现有 worker 模型，每账号一个任务）。
- **回退**：功能以连接方式区分，EWS 账号出问题时可以删除该账号重新按 IMAP 配置，不影响其他账号与本地数据；迁移只重建小表，不做数据搬迁，回退由备份恢复承担。
