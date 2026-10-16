# 应用服务 API

状态：**账号配置接口已实现 / 其余 Draft，均未冻结对外兼容承诺**。当前只有桌面账号管理 IPC，没有 HTTP、MCP、连接测试或邮件 API。Rust 类型位于 `core/src/accounts/types.rs` 和 `core/src/error.rs`，`ts-rs` 生成 `frontend/src/generated/accounts.ts`。

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

当前账号响应使用上述 `ApiResult<T>` 结构，宿主生成响应 `requestId`；创建请求中的 `request.requestId` 则是客户端幂等键，两者用途不同。`Page` 与邮件状态类型尚属后续草案。

未来成功 HTTP 返回对应 2xx，失败使用合理的 4xx / 5xx，不能始终返回 200。Tauri adapter 和 HTTP adapter 将响应转换为同一前端结果类型。MCP 使用自己的 JSON-RPC / tool result 封装，复用 `AppError.code` 而非套一层 HTTP 状态。

## 2. 邮件分页与查询（后续）

列表默认 `limit=50`，范围 1–100。游标不透明，绑定查询条件和最后一条 `(receivedAt, emailId)`；改变过滤条件后重新开始。无效游标报参数错误。权限范围始终由服务端与查询指定的账号取交集，显式请求未授权账号直接拒绝。

```json
{
  "accountIds": ["00000000-0000-4000-8000-000000000001"],
  "folderId": null,
  "query": "invoice",
  "unread": true,
  "hasAttachment": null,
  "receivedAfter": "2026-10-01T00:00:00Z",
  "receivedBefore": "2026-11-01T00:00:00Z",
  "limit": 50,
  "cursor": null
}
```

时间范围为左闭右开。`accountIds` 省略表示当前主体的获授权账号，空数组表示空结果；布尔筛选 `null` 表示不限。全文范围与短词限制见[搜索设计](../architecture/storage-search.md)。返回 `items`、`nextCursor` 与 `coverage { headersComplete, bodyComplete, syncedAfter?, syncedBefore? }`，完整性只针对当前过滤范围。

`EmailSummary` 至少包含 `id`、`accountId`、`folderId`、`subject`、`from`、`receivedAt`、`isRead`、`isStarred`、`hasAttachment`、`bodyState`、`pendingChanges`。不把正文塞进每行，也不向列表暴露 blob 文件路径。

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

### 后续应用操作草案

下表没有运行中的命令或 HTTP 端点。Web 映射会随实现明确，当前账号接口不因此开放网络访问。

| 应用操作 | 主要输入 → 输出 | 拟定 HTTP 映射 |
| --- | --- | --- |
| `accounts.test` | accountId → 收 / 发测试结果 | POST `/api/v1/accounts/{id}/test` |
| `accounts.reauthorize` | accountId / authProfileId / expectedRevision → 受控授权流程 | POST `/api/v1/accounts/{id}/reauthorize` |
| `folders.list` | accountId → 文件夹列表 | GET `/api/v1/accounts/{id}/folders` |
| `emails.list` | 结构化过滤 / 游标 → Page + coverage | GET `/api/v1/emails` |
| `emails.search` | query / 过滤 / 游标 → Page + coverage | POST `/api/v1/search` |
| `emails.read` | emailId → 正文缓存状态、正文、附件元信息 | GET `/api/v1/emails/{id}` |
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
| `notFound` | 账号不存在 |
| `conflict` / `idempotencyConflict` | 加载最新配置或处理重复创建请求 |
| `unsupported` | 当前不支持该认证方式，例如 OAuth2 |
| `storageUnavailable` / `storageIncompatible` | 读写失败或 schema/校验不兼容；保留原数据 |
| `alreadyOpen` | 数据目录已被另一核心实例持有 |
| `secretStoreLocked` / `secretStoreUnavailable` | 解锁或恢复系统凭据库后重试，不降级明文 |
| `secretMissing` | 凭证条目确认不存在，需要重新输入 |
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
