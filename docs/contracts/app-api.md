# 应用服务 API

状态：**启动状态、账号、文件夹、同步状态与邮件读取及按需正文接口已有实现 / 其余 Draft，均未冻结对外兼容承诺** · 2026-10-20。桌面通过受限 Tauri IPC 查询邮件、按需下载正文和打开外链；HTML 仅由隔离协议提供。HTTP、MCP、连接测试和邮件写操作尚未开放。Rust 类型位于 `core/src/accounts/types.rs`、`core/src/mail/mod.rs`、`core/src/error.rs` 和 `core/src/startup.rs`，`ts-rs` 生成 `frontend/src/generated/accounts.ts` 与 `startup.ts`。真实服务商验证另见[验证策略](../development/validation.md)。

## 1. 通用结构

请求中的 JSON 字段使用 `camelCase`，ID 为 UUID 字符串，时间为 UTC RFC 3339。UID 等远端标识留在核心内部，前端只使用稳定的本地 ID。权限主体由宿主注入，客户端不能提交 `actor=admin`。

应用响应统一为互斥结果，传输状态另外保留：

```ts
type ApiResult<T> =
  | { ok: true; data: T; requestId: string }
  | { ok: false; error: AppError; requestId: string };

type AppError = {
  code: string;
  message: string;
  field: string | null; // 参数错误的字段名；不包含原输入值
};

type Page<T> = { items: T[]; nextCursor: string | null };
type BodyState = "missing" | "fetching" | "ready" | "failed";
```

当前账号、文件夹和邮件响应使用上述 `ApiResult<T>` 结构，宿主生成响应 `requestId`；创建请求中的 `request.requestId` 则是客户端幂等键，两者用途不同。邮件分页实际返回下述 `EmailPage`，额外包含覆盖信息。

未来成功 HTTP 返回对应 2xx，失败使用合理的 4xx / 5xx，不能始终返回 200。Tauri adapter 和 HTTP adapter 将响应转换为同一前端结果类型。MCP 使用自己的 JSON-RPC / tool result 封装，复用 `AppError.code` 而非套一层 HTTP 状态。

## 2. 当前邮件分页与查询

| MailxClient 方法 | Tauri 命令与参数 | 核心入口 | 成功结果 |
| --- | --- | --- | --- |
| `listEmails(query)` | `emails_list`，`{ query }` | `MailxCore::list_emails` → `AccountService::list_emails` | `EmailPage` |
| `getEmail(emailId)` | `emails_get`，`{ emailId }` | `MailxCore::get_email` → `AccountService::get_email` | `EmailDetail` |

两个命令与账号、文件夹读取使用同一主窗口来源与启动 Ready 准入，参数结构拒绝未知顶层字段。不访问网络，不下载正文，也不改变已读或星标。宿主没有通用 SQL、路径或读凭证参数。

### 查询参数

```json
{
  "query": {
    "accountIds": ["00000000-0000-4000-8000-000000000001"],
    "folderId": null,
    "folderRole": "inbox",
    "text": "invoice",
    "unread": true,
    "hasAttachment": null,
    "receivedAfter": "2026-10-01T00:00:00Z",
    "receivedBefore": "2026-11-01T00:00:00Z",
    "limit": 100,
    "cursor": null
  }
}
```

- `accountIds` 为 null 或省略时查询所有本地账号，空数组返回空结果；数组元素必须为有效 UUID。当前只有受信本地 UI 入口，未来 HTTP / MCP 再与宿主授权账号范围取交集。
- `folderId` 指定本地文件夹 UUID；`folderRole` 在选定账号间按角色匹配，`inbox` 即统一收件箱。二者不能同时给出，否则返回 `validation`。
- `unread=true` 仅未读，false 仅已读，null 不限；`hasAttachment` 同样按 true / false / null 筛选。星标是返回字段，本阶段没有星标筛选参数或标记写回。
- `text` 去掉首尾空白后，对 subject、from.name、from.address 进行 LIKE 字面子串匹配，`%`、`_` 和反斜杠均转义；空白文本视为不限。当前不搜索正文，没有三字符最短限制，不接受全文 MATCH 表达式。
- `receivedAfter` / `receivedBefore` 为 RFC 3339 时间，规范化为 UTC 毫秒格式后按左闭右开区间过滤；无效时间返回 `validation`。
- `limit` 为 null 或省略时默认 50，有效范围 1–200；工作台请求 100。越界返回 `validation`。
- `cursor` 为 null 时从第一页开始，否则原样传回服务端提供的游标。更改查询条件后须清空游标重新开始；游标不是权限凭据。

仅返回 `remote_state=present`、账号当前 `mailboxSourceId` 下、与文件夹游标 UIDVALIDITY 一致的记录。排序为 `(receivedAt DESC, id DESC)`，下一页条件为 `(received_at, id) < (r, i)`；当前游标编码是 `{"r": receivedAt, "i": id}` 的无填充 base64url，客户端按不透明字符串处理。无法解码、字段结构错误、时间或 ID 无效均返回 `validation`，最后一页 `nextCursor=null`。

### 列表和覆盖信息

```ts
type EmailPage = {
  items: EmailSummary[];
  nextCursor: string | null;
  coverage: {
    headersComplete: boolean;
    windowCapped: boolean;
    bodiesComplete: boolean;
    syncedAfter: string | null;
    syncedBefore: string | null;
  };
};
```

`EmailSummary` 包含 id、accountId、folderId、subject、from、preview、receivedAt、isRead、isStarred、hasAttachment、bodyState、pendingChanges。`preview` 为缓存纯文本前 200 个 Unicode 字符，尚未下载时为空；`bodyState` 为 missing / fetching / ready / failed，`pendingChanges=false`，`isRead` / `isStarred` 来自服务器的 Seen / Flagged；附件标志来自 MIME 结构，不表示附件内容已缓存。缺主题保持空字符串，由 UI 显示“（无主题）”。

`coverage` 按查询的账号 / 文件夹作用域汇总当前可选文件夹游标，不计入 Noselect 容器：

- 至少有一个匹配文件夹，且每个都存在游标、`backfill_done=1`、`window_capped=0` 时，`headersComplete=true`。任一文件夹尚未同步、抓取未完成或窗口超过 1,000 封上限时为 false；不存在匹配文件夹时同样为 false。
- `syncedAfter` 是这些游标中最晚的固定 `window_start`，格式为 UTC 日期 `YYYY-MM-DD`；无游标时为 null。起点是各文件夹首次同步或代际切换当日减 30 天，之后不每日滚动。
- `windowCapped=true` 表示至少一个匹配目录达到每轮 1,000 封选取上限；它与是否仍在同步分别表达。
- `syncedBefore=null`、`bodiesComplete=false`。头部完整性只声明同步窗口，不表示整箱历史完整；文本、未读、附件与时间筛选不改变参与计算的文件夹范围。查询窗口之前的历史时，必须结合 `syncedAfter` 解读结果。

任意可选文件夹使用相同的列表、搜索、未读筛选及分页。空状态依据实际目录同步状态区分等待、同步、失败和成功空目录；覆盖数量受限单独提示，不能从 `headersComplete=false` 推断正在同步。搜索和未读筛选保留各自空提示；打开缺少正文的详情会自动请求一次下载，失败后仅手动重试。相关目录数据版本变化后刷新已加载结果并保留仍有效的选中；无关目录通知不重置当前分页。切换查询后丢弃旧请求结果。

### 详情与错误

`emails_get` 接受本地 `emailId` UUID，只能读取与列表相同可见范围中的邮件；返回 summary、to、cc、replyTo、body、bodyFailure 与 attachments。正文未就绪时 body 为 null；就绪时返回纯文本和安全元数据，attachments 来自非内联 MIME 部分。hasAttachment 仍来自远端结构，不能据此生成附件下载链接。此读取命令本身不触发网络下载；UI 单独调用 emails_fetch_body。

非法查询、游标、时间或 UUID 返回 `validation`；邮件不存在、已删除、旧来源或旧 UIDVALIDITY 不可见时返回 `notFound`。数据库错误使用现有 `storageUnavailable` 等脱敏错误；启动未 Ready 使用 `coreNotReady`，宿主来源或负载错误沿用受控 IPC 边界。无网络访问，因此这两个读取命令不报告 IMAP 登录错误；后台错误通过同步状态单独表达。

### 按需正文与外链

| MailxClient 方法 | Tauri 命令与参数 | 成功结果 |
| --- | --- | --- |
| `fetchBody(emailId)` | `emails_fetch_body`，`{ emailId }` | 最新 `EmailDetail` |
| `prepareBody(emailId, loadRemoteImages?)` | `emails_prepare_body`，`{ emailId, loadRemoteImages?: boolean }` | `{ documentToken }` |
| `releaseBody(emailId, documentToken)` | `emails_release_body`，`{ emailId, documentToken }` | null；幂等释放本次阅读 |
| `bodyLinks(emailId, documentToken)` | `emails_body_links`，`{ emailId, documentToken }` | `{ documentToken, links: [{ emailId, index, url }] }` |
| `requestLink(identity)` | `links_request`，`{ emailId, index, documentToken }` | null；旧编号兼容入口，生产模式拒绝 |
| `openLink(grant)` | `links_open`，`{ emailId, index, documentToken, requestId }` | null |

`EmailBody = { text: string | null, hasHtml: boolean, truncated: boolean, remoteImagesBlocked: number }`。
`bodyFailure` 为 null 或 `tooLarge | stale | network | authentication | protocol | storage`。
`AttachmentSummary` 包含 id、filename、mimeType、size、isInline；本轮不提供附件保存接口。
HTML、原始 MIME、凭证及文件路径不经 IPC 返回。

正文界面自动选择格式：hasHtml 为 true 时显示隔离的 HTML，否则显示纯文本；不提供格式切换或“重新加载正文”按钮。缺失正文仍自动下载，下载失败时保留“重试”入口。

下载使用独立的账号路由连接、EXAMINE 和 UID BODY.PEEK[]，不改变服务端已读状态。同一邮件共享 flight，每账号正文串行，账号之间无全局限额。已缓存正文只读本地数据；失败通常以 bodyState=failed 的成功详情返回，前端不能仅靠 Promise rejection 判断失败。邮件不可见时返回 notFound，存储不可写时返回受控错误，不假称已保存失败状态。

每次 HTML 阅读先 prepare，随后用 `new URL(convertFileSrc('', 'mailx-body'))` 设置 pathname `/document/<emailId>` 和 token 查询参数。空 sandbox 的 iframe 只访问该文档；协议 GET 不创建或重绑令牌。离开本次阅读时调用 release，删除令牌、待处理链接授权并释放远程图片内存；迟到的 prepare 结果同样释放，重复 release 无副作用。令牌有效期 30 分钟、最多 128 个，作为清理后备，各自独立；来源、revision、目录、UIDVALIDITY、present 和净化版本变化会令旧绑定失效。

`mailx://link-request` 载荷为 `{ emailId, index, url, documentToken, requestId }`。点击正文原位置的链接时，原生导航回调先取消 WebView 跳转，再产生受控事件；WebView2 保留协议 204 后备路径。UI 只接受当前已加载、可见文档的事件，立即提交身份与一次性 requestId 给 links_open，不提交 URL，不再显示编号列表或逐次确认弹窗。宿主重新读取链接并限制 http / https / mailto，由系统浏览器或邮件应用打开；悬停通过链接 title 显示真实地址。重复事件、切换邮件、隐藏或文档意外重载后的旧事件均被丢弃。旧编号接口保留内部兼容，生产模式返回空列表且拒绝 links_request。

远程图片策略来自 `settings.security.remoteImages`：`ask`（默认）、`automatic`、`never`。ask 在 HTML 正文上方显示单行提示；“加载图片”只授权当前打开，“不再提示”单独勾选不生效，勾选后加载须先成功保存 automatic；关闭仅隐藏本次提示。重新打开重新询问，never 不显示提示。CID 和已验证 data 图片不受策略影响。prepare 的 loadRemoteImages 默认 false，只有可信主窗口可提交；never 即使收到 true 仍不放行。图片协议每次检查当前策略和文档授权，收紧设置还会撤销已有授权。

正文只包含 `/remote/<emailId>/<index>?token=…`，原始图片 URL 不交给 WebView。Rust 按净化结果解析索引、沿账号路由加载、验证，并仅在本次阅读的内存中复用（保留字节最多 64 MiB，超出部分不额外缓存），不写入数据库或磁盘；列表、搜索和头部同步不会下载图片。当前支持 `<img src>` 的 http/https（含 // 地址），每封最多 32 个资源、响应最多 8 MiB、30 秒、至多 3 次重定向；远程 SVG 可由 Rust 转为静态 PNG 后显示，仅支持有界矢量图形，不加载关联资源；CSS URL、srcset、正文内联 SVG 等继续移除。系统 TUN 的 Fake-IP DNS 映射仅对通过原始域名 TLS 验证的 HTTPS 图片兼容，地址字面量和明文 HTTP 不适用。账号自定义代理代解析无法校验实际 IP 的域名请求仍拒绝加载，不使用本地 DNS 或直连绕行。详情缓存沿用 email(id) 前缀并增加来源 / revision / generation，下载结束刷新详情和已加载列表页并保留有效选中。

## 3. 当前账号操作

| MailxClient 方法 | Tauri 命令与参数 | 核心服务 | 成功结果 |
| --- | --- | --- | --- |
| `listAccounts()` | `accounts_list`，无参数 | `AccountService::list` | `Account[]` |
| `getAccount(accountId)` | `accounts_get`，`{ accountId }` | `AccountService::get` | `Account` |
| `createAccount(request)` | `accounts_create`，`{ request: { requestId, config } }` | `AccountService::create` | `Account` |
| `updateConnection(request)` | `accounts_update_connection`，`{ request: { accountId, expectedRevision, connection } }` | `AccountService::update_connection` | `Account` |
| `updateProfile(request)` | `accounts_update_profile`，`{ request: { accountId, profile } }` | `AccountService::update_profile` | `Account` |

账号分两部分，各自一个修改请求，生命周期不同：

- **资料**（`profile`：`displayName`、`color`、`senderName`）只影响显示或发件人署名。`senderName` 是收件人在 `From` 中看到的名字，保存时去首尾空白，空值或 `null` 清除；含控制字符（含 CR / LF，防头部注入）或超过 128 个字符返回 `validation`，`field` 为 `senderName`；省略等同 `null`。修改不推进 `revision`、不读写凭证、不重启同步；以最后一次写入为准，不做乐观锁。`color` 为 `#rrggbb`（规范为小写）或 `null`，空字符串等同 `null`。
- **连接**（`connection`：`emailAddress`、可空 `providerId`、`imap` / `smtp`、`authentication`、`route`）决定 MailX 如何访问邮箱。修改必须匹配 `expectedRevision`，成功后推进 `revision`，宿主立即重启该账号的同步与正文下载，旧连接不会沿用。修改连接不会改动资料。

创建仍是一个请求：`config` 同时包含资料字段（`displayName`、可省略的 `color` 与 `senderName`）和全部连接字段。连接配置为 `{ host, port, tls, username }`，TLS 为 `implicit` 或 `startTls`。密码认证分 `shared` 和 `separate`；秘密输入必须使用 `keep` / `replace`，完整语义见[账号配置](../architecture/account-configuration.md)。OAuth2 输入类型目前仅用于明确返回 `unsupported`。

`Account` 返回账号与来源 ID、连接配置的 `revision`、名称、`color` 与 `senderName`、非敏感配置、凭证是否已配置及信任状态、路由及版本、UTC 时间。没有密码或秘密引用；`connectionStatus` 当前只能是 `unverified`。主机变化不会隐式扩大旧凭证的信任，也不会执行自动连接测试。

命令仅授权本地可信 main 窗口，并由宿主再次检查来源 URL。原始 IPC 负载在受控边界反序列化，畸形参数只返回固定脱敏错误，不透传可能包含输入值的解析异常。客户端不能自报管理员或指定秘密引用；认证配置之间的账号归属由后端生成关系并由数据库复合外键约束。不存在读取秘密、任意 SQL、客户端指定数据库目录或通用主机文件访问接口。原骨架的 `core_info` 已移除。

创建使用 UUID `requestId` 去重：账号尚处初始 revision 且配置及秘密一致才返回原账号；重复键对应不同内容或已被编辑的账号时返回 `idempotencyConflict`。修改连接必须匹配 `expectedRevision`，不匹配返回 `conflict`；前端重新加载供用户处理，不静默覆盖。修改资料只校验账号存在，`expectedRevision` 不参与。

### 桌面窗口命令

自定义标题栏使用两个仅属于桌面宿主的命令，不经过核心服务，也不受 Ready 检查影响，启动中和启动失败时标题栏仍可操作：

| 命令与参数 | 结果 |
| --- | --- |
| `window_get_chrome`，无参数 | `{ platform, leading, trailing, state }`：平台、标题内容前后要绘制的按钮、`{ maximized, fullscreen, focused }` |
| `window_perform`，`{ action }` | `action` 为 `minimize` / `toggleMaximize` / `close` / `startDragging` / `titleBarDoubleClick` |

状态变化通过 `mailx://window-state` 事件推送，定向到变化所属的窗口（main 或各写信窗口）。`titleBarDoubleClick` 由宿主按系统偏好执行：macOS 读取“连按窗口标题栏”设置，Linux 读取 GtkSettings 的 `gtk-titlebar-double-click`，Windows 切换最大化。Linux 按钮布局来自 `gtk-decoration-layout`；macOS 保留系统红绿灯。这两个命令对 main 和写信窗口都开放，且只作用于调用它的窗口；写信窗口的双击偏好与 Linux 按钮布局与 main 完全相同。

### 写信窗口与发送命令（发信步骤 4a）

窗口分两类：`main` 与 `composer-<uuid>`（后缀必须是小写连字符格式的 UUID，`composer-x`、大写、无连字符、带花括号都被拒绝）。每个命令先过 Tauri capability（`capabilities/main.json`、`capabilities/composer.json`），再过宿主侧的 `window_access` 白名单；两者页面来源检查一样严格（打包内页面或调试构建的 `localhost:5173`，URL 不得带用户名或密码，路径与查询不限）。任何拒绝都是同一个脱敏的 `validation`（字段 `caller`）。传给核心的主体由窗口类别决定，请求里没有任何主体字段：main 为 `Caller::MainWindow`，composer 为 `Caller::Composer`。

| 命令 | main | composer | 说明 |
| --- | --- | --- | --- |
| `accounts_list`、`settings_get`、`window_get_chrome`、`window_perform` | 是 | 是 | 窗口命令只作用于调用窗口 |
| `drafts_get`、`drafts_update`、`drafts_delete` | 是 | 仅自己的草稿 | 草稿 id 必须等于窗口标签里的 id，否则拒绝 |
| `drafts_create { request }`、`drafts_create_from { request }`、`drafts_list { accountId? }` | 是 | 否 | 创建在 main 完成；`drafts_create` 由宿主读取 `compose.defaultAccountId` 传给核心 |
| `outbox_queue_send { draftId, expectedRevision, clientMutationId }` | 否（`validation`） | 仅自己的草稿 | 宿主读取 `compose.undoSendSeconds`；主体固定为 `Composer` |
| `outbox_list { accountId?, includeAll? }`、`outbox_get { id }`、`outbox_cancel { id }`、`outbox_retry_now { id }` | 是 | 否 | |
| `outbox_resolve_unknown { id, choice, clientMutationId }` | 是 | 否 | `choice` 为 `markSent` / `resend`；核心接受 `Composer` 与 `MainWindow`，MCP / 模型仍为 `forbidden` |
| `composer_open { draftId, fresh? }` | 是 | 否 | 草稿必须存在；创建或聚焦 `composer-{draftId}`。`fresh: true`（草稿刚为这个窗口创建）使页面 URL 带 `&fresh=1`，关闭时选择“不保存”会删除草稿；省略即 false |
| `composer_close { force }` | 否 | 是 | 销毁调用窗口；草稿由前端先写出，宿主只负责关闭 |
| 其余全部命令（账号修改、邮件 / 正文 / 链接 / 图片 / 附件、同步、设置修改、通知） | 是 | 否 | 与此前一致 |

返回值与错误沿用 `ApiResult`；错误码 `bodyNotCached`、`draftLocked`、`forbidden`、`alreadySending`、`conflict`、`idempotencyConflict` 原样透传。核心调用都在阻塞线程上执行。

前端用法（4b）：主窗口创建草稿时，`drafts_create` 的 `accountId` 在浏览单个账号的文件夹时传该账号，在统一收件箱 / 草稿 / 发件箱视图传 `null`（由宿主按 `compose.defaultAccountId` 解析）；`drafts_create_from` 遇 `bodyNotCached` 时先调用 `emails_fetch_body`，成功后用**同一个** `clientMutationId` 重试一次；写信窗口的每次保存都带上次保存的 `expectedRevision`，`conflict` 后重新 `drafts_get`，不静默合并。`mailx://notification-open` 的载荷可带 `outboxId`，主窗口据此选中发件箱项。

事件：`mailx://outbox`（载荷 `OutboxChanged`）与 `mailx://drafts`（载荷 `DraftChanged`）发给 main，并发给受影响草稿的 `composer-{draftId}`（若该窗口存在）。`mailx://app-closing`（无载荷）在 main 关闭时发给每个写信窗口；`mailx://composer-close-requested`（无载荷）在系统要求关闭某写信窗口时发给它。收到后前端写出草稿并调用 `composer_close`；宿主 3 秒内未收到则直接关闭。

### 文件夹与同步状态

| 命令与参数 | 结果 |
| --- | --- |
| `folders_list`，`{ accountId }` | `Folder[]`：该账号当前服务器身份（`mailboxSourceId`）下最近一次同步的文件夹，含 `parentId` 层级、角色、`selectable`（`false` 对应 IMAP `\Noselect` 分组）、`countsKnown` 与未读 / 总数；countsKnown=false 时计数未知 |
| `refreshSync(accountId)` | `sync_refresh`，`{ accountId }` | 无数据；让该账号的同步 worker 立即开始下一轮，跳过等待或退避。账号不存在返回 `notFound`；登录被拒、需改设置才会重试的账号不会被唤醒（避免锁号），也不报错。重复请求合并为一次 |
| `sync_states`，无参数 | `AccountSyncState[]`：每个账号的 `phase`（`syncing` / `idle` / `failed`）、`lastSyncedAt`、`failure`、来源 / 运行版本和每目录状态 |

状态变化通过定向到 main 窗口的 `mailx://sync-state` 事件推送，载荷仍是单个 `AccountSyncState`，`sync_states` 返回同形状快照：

- `mailboxSourceId`：当前邮箱来源；`generation`：本进程内账号 worker 重启版本；`sequence`：本代任务内递增的状态版本。前端按当前账号来源及版本拒绝旧快照，包括初始查询与事件的竞态。
- `folderRevision`：成功提交目录列表后的版本；`folders` 为当前可选目录的状态快照。
- 每个 `FolderSyncState` 包含 `folderId`、`phase`（waiting / syncing / idle / failed）、`lastSyncedAt`、`failure` 和 `dataRevision`。只有可见数据实际变化（新增、恢复、删除或标志变化的邮件，目录计数，代际切换与覆盖状态）时才递增版本，未变化的轮次不递增；某步失败前已提交的变化同样会递增版本。前端仅刷新相关查询及目录计数，并把短时间内多个目录的变化合并为一次目录树重载；统一收件箱只响应 inbox，消息内容仍从本地缓存查询。
- 目录状态与数据 coverage 各有职责：同步失败保留已有缓存；`windowCapped` 说明窗口超限；尚未获得 STATUS 的计数用 `countsKnown=false` 表示。
- 账号 `lastSyncedAt` 仅在整轮全部成功时推进；局部目录失败不得宣称账号整轮成功。目录状态为运行态，重启后重建；窗口、邮件和最后已知计数仍持久化。

`failure.willRetry=false`（登录被拒、凭证需重新确认或缺失、服务器不支持所需连接能力）停止账号自动重试，直到配置修改；单目录的完整命令拒绝仍会在后续轮次重试，不使其他目录停摆。账号修改后启动新 generation，旧来源、旧配置 revision 或旧 generation 的排队写操作和状态通知会被拒绝。事件和查询继续遵循 main 窗口来源与启动 Ready 准入，没有新增写操作或读取凭证接口。

### 应用偏好设置

| MailxClient 方法 | Tauri 命令与参数 | 核心组件 | 成功结果 |
| --- | --- | --- | --- |
| `getSettings()` | `settings_get`，无参数 | `SettingsStore::get` | `SettingsSnapshot` |
| `updateSettings(settings)` | `settings_update`，`{ request: Partial<AppSettings> }` | `SettingsStore::patch` | `SettingsSnapshot` |

偏好只需要数据目录，由宿主单独打开 `SettingsStore`，不受 Ready 检查影响；核心启动失败时外观仍可读取和修改。打开失败（目录不可用等）不缓存，下次调用重试。命令同样只授权本地可信 main 窗口。

`AppSettings` 包含 `appearance`、`security`、`layout`、`notifications`（见文末“新邮件通知”）与 `compose`。`compose.defaultAccountId`（字符串，空表示未设置）是统一收件箱等无账号上下文时新建邮件的默认发件账号，设置存储没有数据库，不校验该账号是否存在，草稿创建时再解析（账号不存在则退回排序第一的账号）；`compose.undoSendSeconds` 只能取 0 / 5 / 10 / 20 / 30（默认 0，立即发送），其他值提交返回 `validation`，`field` 为 `compose.undoSendSeconds`，手工编辑文件中的非法值加载时回到 0，旧文件缺少该节时补默认值。`layout.navWidth` 是左侧导航栏宽度（CSS 像素，默认 256，范围 220–420）；越界提交返回 `validation`，`field` 为 `layout.navWidth`，手工编辑文件中的越界值加载时钳制到范围内，旧文件缺少该节时补默认值。前端在拖动或键盘调整结束后才保存，不随每个像素写盘。`layout.listWidth` 是邮件列表（阅读区左侧）的宽度，同样是 CSS 像素，默认 420，范围 300–2000，越界提交返回 `validation`，`field` 为 `layout.listWidth`，手工编辑的越界值加载时钳制，旧文件缺该项时补默认值；它只在启动时作为列表的初始宽度使用，窗口过窄时列表与阅读区按比例缩小。前端的 `layout` 更新只提交变化的字段，保存时再与已保存的值合并，所以侧栏和列表两处各自保存不会互相覆盖。`layout.readerCollapsed`（默认 false）记录阅读区是否折叠，下次启动恢复，旧文件缺该项时按展开处理。拖动类的布局变化由前端抖动写入（停止 500ms 后一次提交），核心不需要为此提供额外接口。`security.remoteImages` 为 `ask | automatic | never`，默认 ask；`security.senderAvatars` 默认 false，开启后才允许 `getAvatar` 访问 Gravatar（见文末）；旧文件缺少该节时自动补默认值。`appearance` 包含 `theme`（`system` / `light` / `dark`）、`accentColor`（空字符串或 `#rrggbb`）、`interfaceFont` 与 `mailFont`（均为空字符串或逗号分隔的字体名；后者只用于邮件正文文档的基础字体，由桌面宿主在生成正文文档时读取，邮件自带样式优先）和 `mailTheme`（`follow` / `light`，默认 `follow`：应用为深色时邮件正文反色显示，图片保持原色；`light` 始终白底）和 `fontSize`（`small` / `default` / `large`，同时缩放邮件正文基础字号），以及 `previewLines`（`none` / `one` / `two` / `three`，默认 `one`：邮件列表每行显示的正文预览行数，`none` 不显示预览；旧文件缺少该项时取默认值）。纯文本正文由核心转义后生成 HTML，识别 http(s) 链接并把 `>` 引用渲染为嵌套引用块，与 HTML 正文走同一净化与隔离路径。空字符串表示内置默认值；字体列表按用户顺序回退，前端末尾补 `sans-serif`。更新只替换提交的设置节，在核心锁内合并，避免不同页面相互覆盖；返回完整规范形式：颜色转小写，字体名去首尾空白和成对引号并丢弃空项。非法字体字符、超过 16 个字体或单个超过 64 个字符返回 `validation`，`field` 为 `appearance.accentColor` / `appearance.interfaceFont` / `appearance.mailFont`。

`SettingsSnapshot` 为 `{ settings, stored, recovered }`。`stored=false` 表示磁盘上尚无设置文件；`recovered=true` 表示原文件无法使用，已改名为 `settings.json.bak`，下次保存成功后清除。文件格式、损坏处理与前端首帧缓存见[桌面与 Web](../architecture/desktop-web.md#应用偏好)。

### 后续应用操作草案

启动状态查询 `app_get_startup_status`、手动重试 `app_retry_startup`、`StartupStatus` 和 `coreNotReady` 错误已按[数据库迁移与启动方案](../architecture/database-migrations.md#6-启动-ipc-与前端边界)注册并生成。账号、文件夹和邮件操作通过宿主的 Ready 检查；这不是前端可修改的权限状态。

下表描述后续应用能力和拟定 HTTP 映射；不存在对应 HTTP 端点。邮件列表、详情与按需正文已通过上文桌面命令提供，其他操作仍未实现；当前接口不因此开放网络访问。

| 应用操作 | 主要输入 → 输出 | 拟定 HTTP 映射 |
| --- | --- | --- |
| `accounts.test` | accountId → 收 / 发测试结果 | POST `/api/v1/accounts/{id}/test` |
| `accounts.reauthorize` | accountId / authProfileId / expectedRevision → 受控授权流程 | POST `/api/v1/accounts/{id}/reauthorize` |
| `emails.list`（桌面 `emails_list` 已有） | 结构化过滤 / 游标 → EmailPage | GET `/api/v1/emails` |
| `emails.search` | query / 过滤 / 游标 → Page + coverage | POST `/api/v1/search` |
| `emails.read`（桌面 `emails_get`） | emailId → 正文缓存状态、纯文本、附件元信息 | GET `/api/v1/emails/{id}` |
| `emails.fetchBody` | emailId → jobId | POST `/api/v1/emails/{id}/fetch` |
| `emails.setFlags` | emailId / flags / clientMutationId → mutationId | PATCH `/api/v1/emails/{id}/flags` |
| `attachments.fetch` | attachmentId → jobId | POST `/api/v1/attachments/{id}/fetch` |
| `attachments.download` | attachmentId → 受控流 / 本地保存结果 | GET `/api/v1/attachments/{id}/content` |
| `drafts.create` | accountId / 内容 / clientMutationId → draftId / revision | POST `/api/v1/drafts` |
| `drafts.createFrom` | emailId / reply · replyAll · forward / clientMutationId → draftId / revision（核心生成收件人、主题、引用与串联头） | POST `/api/v1/emails/{id}/drafts` |
| `drafts.list` / `drafts.get` / `drafts.delete` | 账号过滤 / draftId → 草稿摘要 / 草稿 / 结果 | GET / DELETE `/api/v1/drafts` / `…/{id}` |
| `drafts.update` | draftId / expectedRevision / 内容 → revision | PATCH `/api/v1/drafts/{id}` |
| `drafts.queueSend` | draftId / expectedRevision / clientMutationId → outboxId / state | POST `/api/v1/drafts/{id}/send` |
| `outbox.list` / `outbox.get` / `outbox.cancel` | 过滤 / outboxId → 状态；`cancel` 仅对 `queued` 生效，已开始发送返回 `alreadySending` | GET / POST `/api/v1/outbox` / `…/{id}` / `…/cancel` |
| `outbox.retryNow` | outboxId → 状态（只对 `queued` 立即尝试） | POST `/api/v1/outbox/{id}/retry` |
| `outbox.resolveUnknown` | outboxId / markSent · resend / clientMutationId → 状态（`resend` 是用户明确确认的新尝试） | POST `/api/v1/outbox/{id}/resolve` |
| `sync.request` | accountId / folderId? → jobId | POST `/api/v1/sync` |
| `jobs.get` / `jobs.cancel` | jobId → job 状态 | GET / POST `/api/v1/jobs/{id}` / `…/cancel` |
| `ai.start` / `ai.get` / `ai.cancel` | Run 请求 / runId → Run 快照 | POST / GET `/api/v1/ai/runs` / `…/{id}` / `…/{id}/cancel` |

OAuth、附件上传、设置和删除账号的细分接口在对应实现任务中补充，不通过万能 `execute_sql` 或文件路径接口临时绕过边界。附件 Web 下载需检查所属账号和安全 `Content-Disposition`；正文读取默认不改变已读标记，标记由显式操作触发。

## 4. 邮件写操作一致性

**草稿核心、桌面 IPC 与写信界面已实现；HTTP / MCP 未开放**（发信步骤 1，`core/src/drafts/`、`MailxCore::create_draft` / `create_draft_from` / `get_draft` / `list_drafts` 等）。`create_draft(request, default_account_id)`、`create_draft_from`、`get_draft`、`list_drafts(account_id?)`（按 `updatedAt`、`id` 倒序）、`update_draft`、`delete_draft` 均已有单元测试；DTO 为 `Draft`、`DraftSummary`、`CreateDraftRequest`、`CreateDraftFromRequest`、`UpdateDraftRequest`，已生成到 `frontend/src/generated/accounts.ts`。版本冲突复用 `conflict`（尚未单设 `REVISION_CONFLICT`）；`clientMutationId` 重放返回同一草稿，同键不同内容为 `idempotencyConflict`。`create_draft_from` 不下载正文：正文未缓存时返回新增的错误码 `bodyNotCached`，调用方先 `fetchBody` 再重试。校验失败的 `field` 为 `draft.to` / `draft.cc` / `draft.bcc` / `draft.subject` / `draft.bodyHtml` / `draft.bodyText` / `draft.recipients`。收件人总数不超过 100，主题不超过 998 字节，输入 HTML 不超过 2 MiB，核心派生纯文本不超过 1 MiB。发件箱的 core 层见下。

`CreateDraftRequest` 与 `UpdateDraftRequest` 的正文只有 `bodyHtml: string`，不接受 `bodyText`（未知字段被拒绝）。`Draft` 返回核心净化后的 `bodyHtml` 与只读派生的 `bodyText`。核心使用独立于阅读端的 HTML 白名单，删除样式、事件属性、图片与非 http/https/mailto 链接；支持 h1–h3；段落和标题仅接受 `data-align="center|right"`，`span.data-color` 与 `mark.data-highlight` 仅接受第 15 节的固定色板名，非法颜色包装去掉标签保留文字。纯文本保留标题文字、段落、嵌套列表、引用和链接地址，忽略对齐与颜色；出站 HTML 由核心把受限属性转换为固定内联样式。详见[富文本设计](../architecture/sending.md#14-富文本写信已实现)。HTML 不超过 2 MiB，派生纯文本不超过 1 MiB，超限分别返回 `draft.bodyHtml` / `draft.bodyText` 校验错误。

v9 为旧草稿增加可空 `body_html`；读取旧草稿时由核心把纯文本按行转义为段落，下次保存落盘。回复 / 转发使用缓存原文并降级复杂格式，不加载外部资源。新入队邮件固定为 `multipart/alternative`（纯文本在前、HTML 在后）；迁移不重建已入队的 MIME 快照。

兼容旧草稿：旧纯文本经 HTML 转义可能超过 2 MiB；更新请求正文与核心当前返回值完全一致时，复用已保存的正文，只更新其他字段，不重新套用新正文限额，也不截断内容。新建或实际修改正文仍执行完整净化与限额校验。

`expectedRevision` 不匹配报 `conflict`，客户端加载最新草稿供用户处理，不静默覆盖。`clientMutationId` 用于本地创建 / 入队幂等，作用域为账号及操作类型；重复键带不同请求内容报冲突。MCP 创建草稿也使用该机制。

`drafts.queueSend` 只允许交互 UI 主体（桌面 v0.1 为 `composer-*` 写信窗口），后端核对账号权限、草稿版本和确认过的快照；入队时按设置 `compose.undoSendSeconds` 写入 `notBefore`。MCP 或模型主体即使直接调用该方法也不能通过。发送结果按[发信状态机](../architecture/mail-sync.md#5-本地草稿与可靠发送)与[写信与发送设计](../architecture/sending.md)解释；未来错误的重试提示从不授权对 `unknown` SMTP 结果自动重发。

**发件箱核心、桌面 IPC 与界面已实现；HTTP / MCP 未开放**（`core/src/send/`）。`MailxCore` 方法：

- `queue_send(caller, QueueSendRequest { draftId, expectedRevision, clientMutationId }, undo_send_seconds) -> OutboxItem`：`caller` 必须是 `Caller::Composer`（宿主提供的枚举，`MainWindow` / `Mcp` / `Model` 一律 `forbidden`，不依赖 UI）；`undo_send_seconds` 只能是 0 / 5 / 10 / 20 / 30（`validation`，字段 `undoSendSeconds`）。版本不符 `conflict`，没有收件人 `validation`（字段 `draft.recipients`），草稿已被队列占用 `draftLocked`。同一 `clientMutationId` 且同草稿同版本返回同一项；其他内容 `idempotencyConflict`。发件人地址与名称取入队时账号的当前值，Message-ID 此时生成。
- `list_outbox(account_id?, include_all)`：默认只含 `queued` / `sending` / `failed` / 未处理的 `unknown`；`include_all` 加上 `accepted`、`canceled` 与已处理的 `unknown`。新到旧。`get_outbox(id)`。条目 `OutboxItem` 不含 MIME、正文，只有主题、最多 3 个收件人与总数、状态、时间、尝试次数、`error { reason, smtpCode, text, rejected[] }`、`serverReply`、`resolution`、`supersededBy`。
- `cancel_outbox(id)`：仅 `queued`；`sending` 返回 `alreadySending`，其他状态 `conflict`。`retry_outbox_now(id)`：仅 `queued`，清除退避等待并唤醒（等待期内的 `notBefore` 不跳过，要提前发请用撤销后重新入队）。`resolve_unknown(caller, { id, resolution: markSent | resend, clientMutationId })`：接受 `Caller::Composer` 与 `Caller::MainWindow`（两者都是交互 UI；`Mcp` / `Model` 为 `forbidden`），只对未处理的 `unknown` 生效，`resend` 返回新项。
- 新错误码：`draftLocked`、`forbidden`、`alreadySending`（均已生成到 `ErrorCode`）。`unknown` 永不自动重发。
- 新事件：`CoreEvent::Outbox(OutboxChanged { accountId, outboxId, draftId, state, attempts, nextAttemptAt, errorReason })` 在每次状态变化（含入队、取走、重试排队、恢复）后发出；`CoreEvent::DraftChanged { draftId, accountId, change: created | updated | deleted | locked | unlocked }` 在草稿创建、保存、删除以及队列锁定、释放、接受后删除时发出。载荷不含地址、主题和正文。桌面宿主已转发为 `mailx://outbox` / `mailx://drafts`（见“写信窗口与发送命令”）。

## 5. 事件

### 当前：核心事件出口

核心不再按事件种类各开一个回调，而是只有一个出口：宿主在 `MailxCore::open` 时传入一个 `EventSink`（实现 `emit(&CoreEvent)`；普通闭包即可），由宿主决定每种事件送到哪里。`CoreEvent` 现有五种（后两种见上文发件箱）：

| 事件 | 含义 | 桌面宿主的去向 |
| --- | --- | --- |
| `Sync(AccountSyncState)` | 账号同步状态变化 | `mailx://sync-state`，前端据修订号重新读取 |
| `FlagWriteBack(FlagWriteBack)` | 一轮已读 / 星标写回结束 | `mailx://flags` |
| `NewMail(NewMail)` | 收件箱有新的未读邮件 | 系统通知（见文末“新邮件通知”） |
| `Outbox(OutboxChanged)` | 发送队列项状态变化 | `mailx://outbox`；进入 `accepted` / `failed` / `unknown` 且主窗口不在前台时发系统通知 |
| `DraftChanged(DraftChanged)` | 草稿创建 / 保存 / 删除，或被队列锁定 / 释放 | `mailx://drafts` |

事件在核心的工作线程上发出，宿主必须很快返回，送不到可以丢：事件只是提示，真实数据始终以查询为准。以后加事件种类只需扩展枚举，宿主 `match` 未处理的分支即可，不改 `open` 的签名。服务器宿主可以实现另一个出口（例如把 `NewMail` 投递到 webhook），核心无需改动。

### 后续：事件流草案

```ts
type DomainEvent = {
  streamEpoch: string; // 每次核心进程启动变化
  seq: number;        // 当前 epoch 内单调递增
  type: string;
  resourceId?: string;
  accountId?: string;
  revision?: number;
  occurredAt: string;
};
```

事件类型初案：`account.changed`、`sync.progress`、`emails.changed`、`draft.changed`、`outbox.changed`、`job.changed`、`ai.delta`、`ai.completed`。AI 增量额外带 runId、chunkSeq 和文本，只发给该运行的授权订阅者；普通事件不带正文。

Tauri 事件与 Web SSE 都是可丢失通知，不提供可靠消息队列语义。发现 epoch 变化、序号缺口或重连时读取快照。UI 列表按账号批量失效，不能每封邮件触发一次全库查询。AI delta 不保证重放，以 Run 的累积输出恢复。

## 6. 当前错误与后续分类

当前 `AppError.code` 使用下列 camelCase 值，`message` 为固定脱敏文本，`field` 为可空字段名。当前没有 `retryable` 或任意 `details` 对象。

| code | 含义 / 处理 |
| --- | --- |
| `validation` | 输入无效，按 field 修改；秘密原值不进入错误 |
| `notFound` | 账号不存在，或邮件不存在 / 不属于当前可见来源和代际 |
| `conflict` / `idempotencyConflict` | 加载最新配置或处理重复创建请求 |
| `unsupported` | 当前不支持该认证方式，例如 OAuth2 |
| `storageUnavailable` / `storageIncompatible` | 读写失败或 schema/校验不兼容；保留原数据 |
| `alreadyOpen` | 数据目录已被另一核心实例持有 |
| `secretStoreLocked` / `secretStoreUnavailable` | 解锁或恢复系统凭据库后重试，不降级明文 |
| `secretMissing` | 凭证条目确认不存在，需要重新输入 |
| `coreNotReady` | 核心尚未完成启动，读取启动状态或重试初始化 |
| `internal` | 脱敏内部故障 |

MailxClient 额外表达 `hostUnavailable`（普通浏览器无桌面宿主）与 `transport`（通信失败）；不展示原始 IPC 异常。以下大写代码及 HTTP 映射是未来邮件功能的原始分类草案，尚未加入当前生成的错误枚举，实施时统一命名与验证。

| code | 含义 / 处理 | HTTP |
| --- | --- | --- |
| `INVALID_ARGUMENT` / `SEARCH_QUERY_TOO_SHORT` | 修正输入 | 400 |
| `UNAUTHENTICATED` | 应用 Session / MCP 授权无效 | 401 |
| `PERMISSION_DENIED` | 未授予相应动作 | 403 |
| `NOT_FOUND` | 资源不存在，或不可向当前主体披露其存在 | 404 |
| `REVISION_CONFLICT` / `IDEMPOTENCY_CONFLICT` | 刷新版本或修正重复键 | 409 |
| `AUTH_REQUIRED` / `AUTH_FAILED` | 邮箱或模型需重新配置，不是 Web 登录失败 | 422 |
| `CAPABILITY_UNAVAILABLE` / `BODY_NOT_CACHED` | 能力缺失或先触发获取任务 | 422 |
| `LIMIT_EXCEEDED` / `RATE_LIMITED` | 超大小或速率，遵循 retryAfter | 413 / 429 |
| `PROXY_UNREACHABLE` / `TLS_FAILED` / `NETWORK_TIMEOUT` | 网络路由或握手问题 | 502 / 504 |
| `STORAGE_FULL` / `CORE_UNAVAILABLE` | 本地资源或核心服务不可用 | 503 |
| `SEND_OUTCOME_UNKNOWN` | 可能已提交，需人工核查 | 409 |
| `CANCELED` / `INTERNAL` | 取消 / 脱敏内部故障 | 409 / 500 |

未知内部异常不原样泄漏路径、SQL、凭证或正文。未来若增加 `details`，按错误类型白名单输出，例如 `retryAfterMs`、`currentRevision`，禁止直接序列化底层异常对象。

### 发件人头像与图片缓存

`getAvatar(accountId, address)` 对应 `avatars_get`，`{ accountId, address }`，返回 `AvatarImage { mimeType, data, kind }`（`data` 为 base64，`kind` 为 `photo` 或 `icon`）或 `null`。`security.senderAvatars` 为 false 时宿主直接返回 null，不发任何请求。

开启后的查找顺序：
1. Gravatar：规范化（去空白、转小写）地址的 SHA-256，`https://www.gravatar.com/avatar/<hash>?s=96&d=404`，`kind` 为 `photo`。
2. 发件域名不属于常见个人邮箱（核心内置名单，含子域名匹配，另加用户已配置账号的域名）时，取域名的网站图标，`kind` 为 `icon`：依次尝试发件域名与其可注册父域名（用常见二级后缀的启发式判断，非公共后缀表），每个域名先 DuckDuckGo（`icons.duckduckgo.com/ip3/<domain>.ico`）、失败再 Google（`www.google.com/s2/favicons?domain=<domain>&sz=128`）。只接受由小写字母、数字、连字符组成的 ASCII 域名。个人邮箱域名跳过这一步，避免所有 Gmail 发件人都显示 Google 标志。

披露范围：Gravatar 得知地址的哈希；图标服务得知发件域名的明文。所有请求都经**该账号的网络路由**下载，复用远程图片的下载约束（公网地址、重定向复检、大小与解码校验，结果重新编码）；自定义代理路由无法验证目的地，因此不取、也不回退直连。图标常为 ICO，核心用受限解码取最大尺寸并重新编码为 PNG，ICO 因此也成为远程图片可接受的类型之一。同一时刻最多 3 个头像查找在途，其余排队，避免长列表尾部因超时被误记为无头像。前端始终先显示按地址稳定着色的首字母头像，取到后覆盖；`icon` 在白底上完整显示而不裁切。

远程图片下载：地址是明文 `http://`、使用默认端口且主机是域名时，先尝试同一地址的 `https://`（单独 10 秒上限），失败才回退到原地址；带显式端口或 IP 字面量的地址不升级。这样既避免明文传输，也让邮件里常见的 `http://` 图片在 TUN / Fake-IP 代理下可用——该环境下域名解析为 198.18.0.0/15 的映射地址，明文 HTTP 无法验证目的地而被拒绝，只有 HTTPS 通过证书校验才放行。HTTPS 到 HTTP 的重定向降级仍然拒绝。

图片缓存：核心在数据目录下维护 `image-cache/`，保存已校验并重新编码的 PNG / JPEG，文件名是键的 SHA-256，磁盘上不出现 URL、地址或域名。内容包括头像（按地址）、域名图标（按域名）和已下载的邮件远程图片（按 URL）。命中后无需联网；邮件图片是否可显示仍由宿主按图片策略和文档授权先判断，命中不放宽策略，也仍要求当前有效的文档绑定。总量上限约 200 MB，超限按最近使用时间清理到约 160 MB；头像与图标保留 7 天，邮件图片保留 30 天，超过 60 天未使用的条目在启动时删除。查不到头像或图标的结果只在内存记 1 小时，不落盘。缓存读写失败一律当作未命中，不影响显示。

`clearImageCache()` 对应 `image_cache_clear`，无参数、无数据：删除磁盘与内存中的全部缓存条目。应用自身 CSP 的 `img-src` 为显示头像增加了 `data:`；邮件正文所在的隔离文档不受影响。

### 附件保存

附件内容不经过 IPC 返回。核心提供与宿主无关的两个接口，各宿主决定交付方式：

- `attachment_info(emailId, attachmentId)` → `{ fileName, mimeType, size }`；`attachment_content(emailId, attachmentId)` → 同样的 `info` 加解码后的字节。字节从本地保存的原始邮件取，**不联网**，邮件正文尚未下载时返回 `notFound`。附件 id 沿用详情里列出的 `<emailId>:<部件序号>`，只接受该邮件列出的非内联附件：正文、内联图片、容器部件、其他邮件的 id、格式不对或越界的 id 一律 `notFound`。返回字节前会再核对原始邮件仍是读取时的那一份，已删除或重新同步的邮件不会放出内容。
- `fileName` 由核心清洗，是单个路径分量：只取最后一段（`/`、`\` 都当分隔符）；`< > : " / \ | ? *`、控制字符以及能让扩展名视觉反转的文字方向控制符变为 `_`；去掉开头结尾的点和空格；Windows 设备名（`CON`、`NUL`、`COM1`……含带扩展名的 `NUL.txt`）前加 `_`；长度不超过 200 字节且在字符边界截断，保留扩展名；没有可用名字时用 `attachment`，并按声明的类型补常见扩展名。`mimeType` 只是发件人的声明，宿主不得据此决定打开或渲染方式，列表界面也不显示它。文本类附件由 MIME 解析器按声明的字符集转成 UTF-8，其他类型是传输解码后的原样字节。

桌面宿主的 `attachments_save`，`{ emailId, attachmentId }` → `'saved' | 'cancelled'`：核心取到字节后由 Rust 弹出系统“另存为”对话框（初始文件名为清洗后的名字），用户确认的路径只在 Rust 内使用，**请求里没有路径**，页面最多只能让用户看到一个保存对话框。写入先写同目录下的临时文件再重命名，失败不留半个文件；目标是符号链接或目录时拒绝，不穿过链接写入。关闭对话框返回 `cancelled`，不算错误。文件系统相关的错误统一为固定文案的 `storageUnavailable`，不带路径。

服务器宿主将通过 HTTP 下载提供同一份内容：响应使用 `Content-Disposition: attachment` 与 `X-Content-Type-Options: nosniff`，内容类型一律 `application/octet-stream`，文件名用同一个清洗结果；该宿主尚未实现。

附件元数据和保存都只在正文下载后可用。列表里的 `hasAttachment` 在正文下载前来自邮件头估算（`multipart/mixed`、非文本单体或 `attachment` 处置都算有），所以估算可能偏多也可能偏少；正文下载提交的同一个事务里，它被改写为实际结果——存在任何非内联部件即为 true。已校正（正文为 ready）的邮件，之后邮件头同步在它被删除又重新出现时不会把它冲回估算值；没有下载过正文的邮件，仍用新邮件头的估算。因此“有附件”筛选对已打开过的邮件是准确的，对其余邮件只是估算。

### 已读与星标

`setFlag(emailId, flag, wanted)` 对应 `emails_set_flag`，`{ emailId, flag: 'read' | 'starred', wanted }`，返回 `'unchanged' | 'queued' | 'localOnly'`。先改本地：邮件在列表里立即显示新值，同时在同一个事务里记一条待同步变更，**不等服务器**。`unchanged` 表示已经是这个值；`queued` 表示稍后写回；`localOnly` 表示该文件夹的服务器不保存星标，这个星标只留在本机，是最终状态。只对可见邮件有效，已删除、不存在或已被新一代服务器取代的邮件返回 `notFound`。

**本地为准。** 一封邮件的某个标记只要还有未完成的变更（或是仅本机的星标），后台头部同步拿到的服务器值就不会覆盖它，否则刚点完的星标会闪回去；原始 `flags` 列表仍然跟随服务器。`EmailSummary.pendingChanges` 现在是真实值：存在尚未发出的变更时为 true，仅本机的星标不算。每封邮件每个标记只保留最新的一个意愿，连点两次等于最后一次，不会重放。

**写回。** 每个账号有一个独立的写回任务，和头部同步并列，**用自己的连接**：同步连接用只读的 `EXAMINE`，写入需要 `SELECT`，两者不混用，同步的只读保证不变。它与同步走同一条账号路由与凭据信任检查（自定义代理失败不回退直连）。流程：`SELECT` 文件夹，读 `PERMANENTFLAGS`，核对服务器返回的 UIDVALIDITY 与本地一致（不一致说明文件夹被重建，UID 已不是同一封邮件，不写入），再用 `UID STORE` 逐封设置，且只要求服务器回报 flags 才算确认：没有确认、确认的是别的 UID、或回报里没有这个标记，都当作没写成。只会发送 `\Seen` 和 `\Flagged` 两个标记。收到 NO / BAD（连接仍可用）的变更留在队列里，按 5 秒起指数退避重试，上限 10 分钟，单条失败 12 次后不再自动重试；用户新的操作会唤醒任务并重置计数。登录被拒或凭据不可信时停止，避免锁号。

**不支持星标的服务商。** 服务器的 `PERMANENTFLAGS` 不含 `\Flagged`（例如 2925），或从未声明、而 STORE 被拒绝时，该文件夹记为不可存星标：现有和以后的星标都转为“仅本机”，已读仍照常同步。核心通过 `mailx://flags` 事件（`FlagWriteBack { accountId, sent, localOnlyStars }`）告知宿主，前端据此提示用户一次；取消星标不提示。

**文件夹未读数。** 改已读 / 未读时，核心在同一个事务里给邮件所在文件夹的 `unread_count` 加减 1（只对已知的计数；未知计数保持未知，不凭空造数；不会小于 0；邮件值没变则不动；星标不影响它），所以侧栏和列表立刻一致，也没有额外请求。头部同步的 `STATUS` 在该文件夹还有未发出的“已读”变更时**不覆盖**本地未读数，因为服务器此时数的还是旧状态，否则数字会先降后弹回；总数仍跟随服务器。变更确认后，下一轮同步自然取到服务器的数。

**垃圾邮件的显示。** 垃圾邮件文件夹不显示数字，有未读时只显示一个圆点（`aria-label="有未读邮件"`）；它的未读数不计入账号行，也不计入折叠父文件夹的汇总，折叠的父文件夹在其下有未读垃圾邮件时显示圆点以免被折叠藏起来。“全部收件箱”本来只汇总收件箱角色，不受影响。

**自动已读。** 前端在邮件详情显示出来之后才把它标为已读，只经过列表而没打开的邮件保持未读；已读、有待同步变更的邮件不处理。已读、星标仅作用于当前打开的邮件，批量操作尚未实现。

### 全局提示（Toast）

所有短提示都经由 `@/components/ui/toast` 的 `toast.info / success / error`，可在 React 之外调用，唯一的出口 `Toaster` 挂在应用根部、启动闸门之外，核心启动失败时也能提示。位置只有一处常量（`VIEWPORT_POSITION`），默认右下角。普通提示自动消失（信息 5 秒、成功 3 秒、错误 8 秒），最多同时显示 4 条；错误以 `role="alert"` 立即播报，其余走礼貌的 live region。关闭按钮在鼠标悬停或键盘聚焦进入提示后才对辅助技术可见，这是 Base UI 的设计。

### 新邮件通知

**判定在核心。** 头部同步每轮对每个文件夹记下此前见过的最大 UID；本轮新存入、UID 高于它的邮件才算“新到达”，再筛掉不在收件箱（`role = inbox`）、已读（例如已在手机上读过）、或收到时间早于 24 小时前（离线多日后补齐的）的邮件。以下情况一律不算新邮件：首次同步一个文件夹；UIDVALIDITY 变化导致整夹重建；补齐 30 天窗口内的旧邮件。首轮为空的收件箱完成之后，下一封到达的邮件可以通知。新邮件按“账号 × 同步轮次 × 文件夹”合并成一个 `NewMail { accountId, accountName, count, messages }`，`messages` 只带最新的 3 封（发件人、主题、时间、邮件 id），其余只计数。已经入库的新邮件即使这一轮后面失败也会通知：游标已越过它们，以后不会再出现。

**设置也在核心，所有宿主一致。** `notifications` 设置节：`enabled`（默认开）、`showPreview`（默认开）、`whenFocused`（默认关）、`sound`（`system | chime | ding | droplet | bubble | marimba | sparkle | knock | chord | none`，默认 `system`）、`mutedAccounts`（账号 id 列表）。核心的 `NewMail::for_delivery(&settings)` 统一套用前三项中与内容相关的规则：总开关关闭或账号被静音时什么都不发；关闭预览时把 `messages` 清空，发出的内容里就不含发件人和主题，任何宿主（包括以后的 webhook）都不会泄露。`NewMail::summary()` 给出标题和正文：一封时为“发件人 / 主题”，多封时为“账号名 / N 封新邮件，来自 A、B 等”。`mutedAccounts` 保存时去空白、排序、去重，含空项或控制字符返回 `validation`（`field` 为 `notifications.mutedAccounts`）；手工编辑出的非法列表加载时整体清空。

**桌面宿主。** 使用 `tauri-plugin-notification`。窗口可见、未最小化且处于焦点时，除非开启 `whenFocused`，否则不弹通知（未读数照常更新）。提示音：`system` 由系统通知自带的默认提示音播放；`chime / ding / droplet / bubble / marimba / sparkle / knock / chord` 是 MailX 自带的短音效，在本机合成（几个正弦音加包络，没有音频文件），通知本身静音，由应用经默认输出设备播放，两者不会同时响；`none` 静音。点击通知时窗口被带到前台，并向页面发 `mailx://notification-open`（`{ accountId, emailId | null }`）：只有一封邮件时在阅读区打开它（不改动列表所在的邮箱），多封时切到该账号的收件箱。关闭预览时通知不指向具体邮件。

| MailxClient 方法 | Tauri 命令与参数 | 结果 |
| --- | --- | --- |
| `testNotification()` | `notifications_test`，无参数 | 按已保存的设置发送一条示例通知（忽略“前台时不通知”） |
| `previewSound(sound)` | `notifications_preview_sound`，`{ sound }` | 播放 MailX 自带音效；`system` 与 `none` 不播放 |
| `onNotificationOpen(listener)` | 事件 `mailx://notification-open` | 通知被点击 |

**平台差异（来自插件与系统实现，尚未在 Windows / Linux 实机确认）。** macOS：开发模式（`tauri dev`）下通知以“终端”的名义发出，真实效果以打包版本为准；每条可点击的通知占一个等待线程，直到它被点击、关闭或自动消失（横幅样式会自动消失；用户改成“提醒”样式时会一直等到被处理，数量受按账号合并限制）。Windows：只有通知还在屏幕上时点击才会回报，进入操作中心后点击只会唤起应用；开发构建不设 AppUserModelID。Linux：取决于通知服务器，部分服务器不支持声音提示或点击回报；自带音效走 ALSA。

### 发送结果通知（桌面）

`Outbox` 事件把某项带入 `accepted`、`failed`、`unknown` 时，若主窗口不在前台（或用户开了“窗口在前台也通知”）且通知总开关为开，宿主发系统通知，标题分别为“已发送”“发送失败”“可能已发送，需要确认”。仅当设置 `notifications.showPreview` 为开时才读取并显示主题（无主题显示“（无主题）”）；关闭时正文为空，也不读取主题。`accepted` 静音，另两种用所选提示音。点击通知聚焦主窗口，并发送 `mailx://notification-open`，载荷 `{ accountId, emailId: null, outboxId }`（新增 `outboxId`，新邮件通知该字段为 `null`）；前端界面尚未处理它。
