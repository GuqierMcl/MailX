# 应用服务 API 草案

状态：**Draft / 未实现、未冻结**。以下接口用于指导首批实现，可随新设计修改。

## 1. 通用结构

请求中的 JSON 字段使用 `camelCase`，ID 为 UUID 字符串，时间为 UTC RFC 3339。UID 等远端标识留在核心内部，前端只使用稳定的本地 ID。权限主体由宿主注入，客户端不能提交 `actor=admin`。

应用响应统一为互斥结果，传输状态另外保留：

```ts
type Result<T> =
  | { ok: true; data: T; requestId: string }
  | { ok: false; error: AppError; requestId: string };

type AppError = {
  code: string;
  message: string;
  retryable: boolean;
  details?: Record<string, unknown>; // 仅允许各错误定义的脱敏字段
};

type Page<T> = { items: T[]; nextCursor: string | null };
type BodyState = "missing" | "fetching" | "ready" | "failed";
```

成功 HTTP 返回对应 2xx，失败使用合理的 4xx / 5xx，不能始终返回 200。Tauri adapter 和 HTTP adapter 将响应转换为同一前端结果类型。MCP 使用自己的 JSON-RPC / tool result 封装，复用 `AppError.code` 而非套一层 HTTP 状态。

## 2. 分页与查询

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

## 3. 首批应用操作

下表中的 HTTP 映射属于后续 Web 设计，当前没有运行中的端点。Tauri 命令按领域命名，经同一服务执行。

| 应用操作 | 主要输入 → 输出 | 拟定 HTTP 映射 |
| --- | --- | --- |
| `accounts.list` | 无 → 脱敏账号、能力、状态 | GET `/api/v1/accounts` |
| `accounts.configure` | 配置 + 写入型凭证字段 → accountId / revision | POST `/api/v1/accounts` |
| `accounts.test` | accountId → 收 / 发测试结果 | POST `/api/v1/accounts/{id}/test` |
| `accounts.reauthorize` | accountId → 受控授权流程 | POST `/api/v1/accounts/{id}/reauthorize` |
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

## 4. 写操作一致性

`expectedRevision` 不匹配报 `REVISION_CONFLICT`，客户端加载最新草稿供用户处理，不静默覆盖。`clientMutationId` 用于本地创建 / 入队幂等，作用域为账号及操作类型；重复键带不同请求内容报冲突。MCP 创建草稿也使用该机制。

`drafts.queueSend` 只允许交互 UI 主体，后端核对账号权限、草稿版本和确认过的快照；MCP 或模型主体即使直接调用该方法也不能通过。发送结果按[发信状态机](../architecture/mail-sync.md)解释；`retryable` 从不授权对 `unknown` SMTP 结果自动重发。

## 5. 事件草案

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

## 6. 错误分类初案

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

未知内部异常不原样泄漏路径、SQL、凭证或正文。`details` 按错误类型白名单输出，例如 `retryAfterMs`、`currentRevision`，禁止直接序列化底层异常对象。
