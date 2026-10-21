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
| `updateAccount(request)` | `accounts_update`，`{ request: { accountId, expectedRevision, config } }` | `AccountService::update` | `Account` |

`config` 包含 `displayName`、`emailAddress`、可空 `providerId`、独立的 `imap` / `smtp` 连接配置、`authentication` 和 `route`。连接配置为 `{ host, port, tls, username }`，TLS 为 `implicit` 或 `startTls`。密码认证分 `shared` 和 `separate`；秘密输入必须使用 `keep` / `replace`，完整语义见[账号配置](../architecture/account-configuration.md)。OAuth2 输入类型目前仅用于明确返回 `unsupported`。

`Account` 返回账号与来源 ID、revision、非敏感配置、凭证是否已配置及信任状态、路由及版本、UTC 时间。没有密码或秘密引用；`connectionStatus` 当前只能是 `unverified`。主机变化不会隐式扩大旧凭证的信任，也不会执行自动连接测试。

命令仅授权本地可信 main 窗口，并由宿主再次检查来源 URL。原始 IPC 负载在受控边界反序列化，畸形参数只返回固定脱敏错误，不透传可能包含输入值的解析异常。客户端不能自报管理员或指定秘密引用；认证配置之间的账号归属由后端生成关系并由数据库复合外键约束。不存在读取秘密、任意 SQL、客户端指定数据库目录或通用主机文件访问接口。原骨架的 `core_info` 已移除。

创建使用 UUID `requestId` 去重：账号尚处初始 revision 且配置及秘密一致才返回原账号；重复键对应不同内容或已被编辑的账号时返回 `idempotencyConflict`。修改必须匹配 `expectedRevision`，不匹配返回 `conflict`；前端重新加载供用户处理，不静默覆盖。

### 桌面窗口命令

自定义标题栏使用两个仅属于桌面宿主的命令，不经过核心服务，也不受 Ready 检查影响，启动中和启动失败时标题栏仍可操作：

| 命令与参数 | 结果 |
| --- | --- |
| `window_get_chrome`，无参数 | `{ platform, leading, trailing, state }`：平台、标题内容前后要绘制的按钮、`{ maximized, fullscreen, focused }` |
| `window_perform`，`{ action }` | `action` 为 `minimize` / `toggleMaximize` / `close` / `startDragging` / `titleBarDoubleClick` |

状态变化通过定向到 main 窗口的 `mailx://window-state` 事件推送。`titleBarDoubleClick` 由宿主按系统偏好执行：macOS 读取“连按窗口标题栏”设置，Linux 读取 GtkSettings 的 `gtk-titlebar-double-click`，Windows 切换最大化。Linux 按钮布局来自 `gtk-decoration-layout`；macOS 保留系统红绿灯。命令同样只授权本地可信 main 窗口。

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

`AppSettings` 包含 `appearance`、`security` 与 `layout`。`layout.navWidth` 是左侧导航栏宽度（CSS 像素，默认 256，范围 220–420）；越界提交返回 `validation`，`field` 为 `layout.navWidth`，手工编辑文件中的越界值加载时钳制到范围内，旧文件缺少该节时补默认值。前端在拖动或键盘调整结束后才保存，不随每个像素写盘。`security.remoteImages` 为 `ask | automatic | never`，默认 ask；`security.senderAvatars` 默认 false，开启后才允许 `getAvatar` 访问 Gravatar（见文末）；旧文件缺少该节时自动补默认值。`appearance` 包含 `theme`（`system` / `light` / `dark`）、`accentColor`（空字符串或 `#rrggbb`）、`interfaceFont` 与 `mailFont`（均为空字符串或逗号分隔的字体名；后者只用于邮件正文文档的基础字体，由桌面宿主在生成正文文档时读取，邮件自带样式优先）和 `mailTheme`（`follow` / `light`，默认 `follow`：应用为深色时邮件正文反色显示，图片保持原色；`light` 始终白底）和 `fontSize`（`small` / `default` / `large`，同时缩放邮件正文基础字号）。纯文本正文由核心转义后生成 HTML，识别 http(s) 链接并把 `>` 引用渲染为嵌套引用块，与 HTML 正文走同一净化与隔离路径。空字符串表示内置默认值；字体列表按用户顺序回退，前端末尾补 `sans-serif`。更新只替换提交的设置节，在核心锁内合并，避免不同页面相互覆盖；返回完整规范形式：颜色转小写，字体名去首尾空白和成对引号并丢弃空项。非法字体字符、超过 16 个字体或单个超过 64 个字符返回 `validation`，`field` 为 `appearance.accentColor` / `appearance.interfaceFont` / `appearance.mailFont`。

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
| `drafts.update` | draftId / expectedRevision / 内容 → revision | PATCH `/api/v1/drafts/{id}` |
| `drafts.queueSend` | draftId / expectedRevision / clientMutationId → outboxId / state | POST `/api/v1/drafts/{id}/send` |
| `outbox.get` / `outbox.cancel` | outboxId → 当前状态 | GET / POST `/api/v1/outbox/{id}` / `…/cancel` |
| `sync.request` | accountId / folderId? → jobId | POST `/api/v1/sync` |
| `jobs.get` / `jobs.cancel` | jobId → job 状态 | GET / POST `/api/v1/jobs/{id}` / `…/cancel` |
| `ai.start` / `ai.get` / `ai.cancel` | Run 请求 / runId → Run 快照 | POST / GET `/api/v1/ai/runs` / `…/{id}` / `…/{id}/cancel` |

OAuth、附件上传、设置和删除账号的细分接口在对应实现任务中补充，不通过万能 `execute_sql` 或文件路径接口临时绕过边界。附件 Web 下载需检查所属账号和安全 `Content-Disposition`；正文读取默认不改变已读标记，标记由显式操作触发。

## 4. 邮件写操作一致性（后续）

`expectedRevision` 不匹配报 `REVISION_CONFLICT`，客户端加载最新草稿供用户处理，不静默覆盖。`clientMutationId` 用于本地创建 / 入队幂等，作用域为账号及操作类型；重复键带不同请求内容报冲突。MCP 创建草稿也使用该机制。

`drafts.queueSend` 只允许交互 UI 主体，后端核对账号权限、草稿版本和确认过的快照；MCP 或模型主体即使直接调用该方法也不能通过。发送结果按[发信状态机](../architecture/mail-sync.md)解释；未来错误的重试提示从不授权对 `unknown` SMTP 结果自动重发。

## 5. 事件草案（后续）

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
