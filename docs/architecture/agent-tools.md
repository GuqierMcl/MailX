# 工具层与调用入口

状态：**本地 MCP 工具层已实现 / 内置 Agent 与远程入口为 Planned**。`core/src/tools/` 提供邮件查询与整理、草稿、附件和发送工具；`mcp-server/` 在 `mcp.enabled` 打开时由桌面宿主启动，宿主为每个连接构造 `RequestContext` 并授予 `Grant::mcp_default()`。桌面邮件操作复用 core 应用服务，内置 Agent 尚未接入工具层。工具目录见第 4 节，平台与客户端证据见[验证记录](../development/validation.md)，设计依据见 [ADR 0004](../adr/0004-shared-tool-layer.md)。

## 1. 分层

```text
内置 Agent / 固定工作流 ──────────────── 进程内调用 ─┐
本地外部 Agent → stdio bridge → 本机受控 IPC → MCP 适配层 ─┤→ 工具层 → 应用服务 → 存储 / 协议
远程外部 Agent → HTTPS Streamable HTTP ──→ MCP 适配层 ─┘
```

- **工具层**（core，`core/src/tools/`，已实现）：工具定义、schema 校验、按调用方的可见性与策略、授权检查、结果整形和错误码。与传输无关，不依赖 Tauri、Axum 或 UI。
- **MCP 适配层**：把工具层中对 MCP 可见的工具映射为 MCP 工具，处理 initialize、版本协商、工具列表、取消与 JSON-RPC 错误。桌面核心与服务端进程托管同一份实现，只换底层传输。M0 后确定放在独立 crate：core 不依赖 `rmcp`，适配层、本地 IPC 监听与发现文件在 MCP 适配 crate 中，由桌面宿主链接，以后服务端也复用它；bridge 是另一个只依赖异步运行时与 JSON 解析的小可执行 crate。依据见 [MCP 草案 M0 结论](../contracts/mcp.md#8-m0-原型结论2026-11-02)。
- **传输**：本地 IPC 监听由桌面宿主创建；远程 HTTP 路由由服务端宿主挂载。stdio bridge 只完成 token 握手后转发字节，不解析工具。
- **内置 Agent**：AI Runtime 直接调用工具层，工具调用结果作为不可信材料回填给模型，不经 MCP 序列化。

## 2. 工具定义

每个工具包含：

| 字段 | 说明 |
| --- | --- |
| 标识 | 工具层内部稳定 ID，例如 `search_emails`；MCP 适配层对外加 `mailx_` 前缀 |
| 输入 / 输出 schema | 拒绝未知字段，ID 与字符串有长度上限，字段采用 camelCase，与应用 DTO 一致 |
| 可见入口 | `builtinAgent`、`mcp` 的组合；未声明即不可见，MCP 必须显式声明 |
| 所需动作 | 例如 `accounts:read`、`mail:read`、`draft:create`、`draft:write`，授权检查依据 |
| 按入口策略 | 输出上限、账号范围来源、是否需要用户确认等，见第 4 节 |
| 行为注解 | 只读 / 会写本地数据 / 需用户确认；只帮助调用方理解，不作为安全控制 |
| 处理函数 | 接收 `ToolCall`（含 `RequestContext`、本入口策略）与已反序列化的类型化输入，经 `MailxCore` 执行，返回类型化结构；转 JSON 在注册表边界完成 |

处理函数不感知入口；入口差异全部体现在 `RequestContext` 与策略上，不为不同入口复制实现。

已实现的 API（`mailx_core::tools`）：

```rust
let tools = Tools::new(core, || default_account_id()) // 宿主读取 compose.defaultAccountId
    .with_audit(Arc::new(my_sink));                    // 可选，见第 3 节
tools.list(Audience::Mcp)            // -> Vec<ToolDescriptor>，只含该入口可见的工具
tools.list_for(&ctx)                 // 同上，并隐藏授权不含所需动作的工具
tools.call(&ctx, "read_email", json) // -> Result<serde_json::Value, ToolError>
tools.register(spec, handler)        // 注册 / 替换工具，handler: Fn(&ToolCall, I) -> Result<O, ToolError>
```

- `ToolSpec::new::<Input, Output>(id, title, description, annotations)` 由 `schemars` 1.x 从类型生成 JSON Schema；`.requires(&[Action])` 设所需动作；`.visible_to(Audience, AudiencePolicy)` 声明可见入口与该入口策略，未声明即不可见。输入类型使用 `rename_all = "camelCase"` 与 `deny_unknown_fields`；长度、数量、范围由校验代码执行（schema 不表达）。
- 调用是同步、会阻塞存储的，与 `MailxCore` 一致；异步宿主必须用 `spawn_blocking`。
- 每次 `call` 依次：工具对该入口不可见或不存在 → `unknownTool`（不泄露隐藏工具的存在）；授权缺少所需动作 → `notPermitted`；输入不符 schema → `invalidInput`；处理函数内再做字段校验与账号范围检查。授权每次重新评估，不缓存。
- `null` 输入视为 `{}`。
- 可选值用“省略”表示，不用 `null`：输入的可选字段不在 `required` 中且只有单一类型；输出的可选字段（如 `nextCursor`、`nextOffset`、`from.name`、`lastSyncedAt`）无值时不出现。工具 schema 统一在 `core/src/tools/spec.rs` 的 `schema_of` 生成，会去掉 schemars 为 `Option<T>` 加的 `null` 类型，因为部分 MCP 客户端不支持 `type` 数组。

## 3. RequestContext

宿主为每次调用构造 `RequestContext`，调用方输入不能自报额外权限：

| 调用方 | 主体 | 账号范围来源 |
| --- | --- | --- |
| 内置 Agent | runId、模型提供商与模型 | 本次 Run 的选定内容与该提供商的披露授权 |
| 本地 MCP | 连接 id（经发现文件中的 token 认证）与客户端自报名称 | 统一授权：全部账号、读取与整理邮件、管理草稿及本机文件附件、直接发信与发件箱查询 / 取消（无删除邮件）；后续可改为逐客户端授权记录 |
| 远程 MCP | 远程授权主体与 clientId | 远程授权范围（待设计） |

本地 MCP 首版的简化及其风险说明见 [MCP 草案](../contracts/mcp.md#首版授权有意简化)。

`RequestContext { principal, grant, request_id }` 由宿主用公有构造函数创建：`Principal::{BuiltinAgent { run_id }, LocalMcp { connection_id, client_name }}`（`client_name` 自报，只用于显示和审计），`Grant { accounts: AccountScope::{All, Only(ids)}, actions }`，`Action::{AccountsRead, MailRead, DraftCreate, DraftWrite, MailOrganize, MailArchive, MailSync, MailSend, OutboxRead, OutboxWrite, FilesRead}`（序列化为 `accounts:read`、`mail:read`、`draft:create`、`draft:write`、`mail:organize`、`mail:archive`、`mail:sync`、`mail:send`、`outbox:read`、`outbox:write`、`files:read`）。`Grant::mcp_default()` 是本地 MCP 首版统一授权（全部账号、读取邮件与草稿、创建 / 修改 / 删除草稿，包含整理、归档、刷新、发送与发件箱动作，没有删除邮件动作）。界面状态由宿主提供：`Tools::with_draft_open_check`（草稿是否正开在写信窗口，默认从不）、`Tools::with_ui_opener`（把窗口带到前台并打开邮件，默认 `unavailable`）与 `Tools::with_draft_opener`（打开已有草稿，默认 `unavailable`），均在调用时读取。

发送路径中的 `send::Caller` 由宿主提供：`LocalMcp` → `Caller::Mcp`，`BuiltinAgent` → `Caller::Model`。两者在工具层检查 `mail:send` 与账号范围后走正常队列，不能通过输入伪造为 `Composer`。`MainWindow` 仍不能入队，处理 `unknown` 仍限用户 UI。宿主通过 `with_undo_send_seconds` 注入撤销窗口，读取失败会阻止发送。

账号范围：`list_accounts` 只返回范围内账号；`search_emails` 把请求的 `accountIds` 与范围取交集（范围外 id 被忽略，与不存在的账号表现相同）；`read_email` 先读出邮件所属账号，范围外与不存在的 `emailId` 返回同样的 `notFound`；`create_draft` 的账号不在范围内同样返回 `notFound`。

账号识别使用本地元数据，不读取凭据。`list_accounts` 的 `isDefault` 与当前范围内新草稿省略账号时的选择一致；草稿结果给出当前 `senderEmailAddress`，发送任务给出冻结的 SMTP 发件地址。Agent 通过邮箱选择账号并回报发件人，无需再猜测 UUID 对应的身份。

每次调用都重新检查授权状态，关闭 MCP 或撤销后后续调用立即拒绝。已知 `emailId` 也要先确认其账号在范围内，不能绕过账号授权。调用记录主体、工具、资源 ID、结果与 requestId，不记录完整工具结果。

core 目前没有日志后端，存储中也没有审计表；工具层因此把每次调用（含被拒绝的）交给宿主提供的 `ToolAudit` sink（默认丢弃）：`ToolCallRecord { request_id, principal_kind, principal_id, client_name, tool_id（截断到 64 字符）, resource_ids（账号与邮件 / 草稿 id）, error: Option<ToolErrorCode> }`，不含参数、邮件内容或结果。宿主（MCP 适配层 / 桌面）接入时把记录写入自己的日志；是否落审计表待定。

## 4. 工具集（草案）

工具池按“相交”组织：两侧共享领域工具，各自另有专属工具。

| 工具 | 内置 Agent | MCP | 说明 |
| --- | --- | --- | --- |
| `list_accounts` | ✓ | ✓ | 范围内账号 id、显示名、邮箱地址、实际默认标记 `isDefault`、同步状态 `syncStatus`（`ok` / `syncing` / `authError` / `connectionError` / `unknown`，取自同步管理器，不含服务器原文）与 `lastSyncedAt?` |
| `search_emails` | ✓ | ✓ | 本地检索，限量摘要与游标；默认排除 trash / junk 文件夹，可选 `folderRole` 只搜某一角色（含 trash / junk）；可选 `folderId`（取自 `list_folders`）只搜某个文件夹且不再默认排除 trash / junk |
| `read_email` | ✓ | ✓ | 纯文本片段、发件方、附件元信息，按字符分页；可选 `download` 在正文未缓存时按需下载（不标已读、60 秒上限） |
| `list_folders` | ✓ | ✓ | 范围内账号的已同步文件夹：`id`、`accountId`、`name`、`role`，计数已知时带未读 / 总数；只读本地，需要 `mail:read` |
| `create_reply_draft` | ✓ | ✓ | 回复 / 全部回复 / 转发草稿，调用方文字放在引用之上，来源标记随主体；原邮件正文须已缓存，永不发送 |
| `create_draft` | ✓ | ✓ | 只建本地草稿，不代表发送；UI 标出创建来源：MCP 主体写 `mcp`，内置 Agent 主体写 `ai`（`Draft.origin` 与界面标记已有） |
| `list_drafts` | ✓ | ✓ | 范围内全部草稿（不限来源）的摘要，最多 100 条，含 `revision`、`origin`、`locked`、`open`，无正文；`mail:read`；已实现 |
| `read_draft` | ✓ | ✓ | 单个草稿的收件人、`bodyHtml`、`bodyText`、附件元信息与 `revision`；`mail:read`；已实现 |
| `update_draft` | ✓ | ✓ | 按 `expectedRevision` 修改给出的字段，不改 `origin` 与附件；`conflict` / `draftLocked` / `draftOpen`；`draft:write`；已实现 |
| `delete_draft` | ✓ | ✓ | 按 `expectedRevision` 删除草稿（破坏性注解）；`draft:write`；已实现 |
| `get_ui_context` | ✓ | | 当前选中的邮件或正在编辑的草稿引用 |
| `propose_draft_edit` | ✓ | | 对编辑器中的草稿提出修改，带 revision，由用户接受 |
| `present_suggestions` | ✓ | | 摘要、待办等结构化建议展示到界面 |
| `request_user_confirmation` | ✓ | | 请求用户在界面中确认高风险动作；只能请求，不能批准 |
| `show_email_in_app` | | ✓ | 先检查范围，再让宿主把 MailX 带到前台并打开某封邮件（只读，但会移动用户焦点）；`mail:read`；已实现 |
| `set_email_flags` | ✓ | ✓ | 1–50 封、已读与星标分别报告 queued / unchanged / localOnly / failed，允许部分成功 |
| `archive_emails` / `unarchive_emails` | ✓ | ✓ | 1–50 个目标，本机归档 / 恢复，整批原子；真实计数与 UI 精确撤销 |
| `refresh_sync` | ✓ | ✓ | 按账号唤醒正常同步任务，不自动恢复服务商策略停止状态 |
| `show_draft_in_app` | | ✓ | 检查账号范围后打开已有草稿窗口，不发送 |
| `send_email` | ✓ | ✓ | action 区分直接发送草稿 / 唤醒 queued 任务；版本与幂等保护 |
| `list_outbox` / `get_outbox` | ✓ | ✓ | 有界查询范围内发送任务，返回稳定状态与错误类别 |
| `cancel_outbox` | ✓ | ✓ | 仅取消 queued，发送开始后拒绝 |
| `add_draft_attachment` / `remove_draft_attachment` | ✓ | ✓ | 按 revision 添加本机普通文件 / 移除普通附件，保留草稿打开保护 |

二十二个 MCP 工具（二十个也对内置 Agent 可见）已实现，输入输出与限制见 [MCP 草案](../contracts/mcp.md#3-工具草案)（schema 以 `ToolDescriptor` 为准）。其余为候选，进入实现前单独补齐 schema。MCP 的 resources / prompts 等协议特有能力若后续提供，只存在于 MCP 适配层。

发送工具随本地 MCP 总开关授权，直接发信无需逐封确认。首次发送核对草稿版本和幂等键，复用现有快照、发送队列与事件；规则见[发送与附件工具](mail-send-tools.md)。

## 5. 按入口策略

同一工具对不同入口可取不同策略，示例：

| 策略 | 内置 Agent | MCP |
| --- | --- | --- |
| 账号范围 | 本次 Run 的选定内容；需要检索更大范围时须先取得该提供商对相应账号的披露授权 | 首版为全部账号；引入逐客户端授权后可再收窄 |
| `read_email` 输出上限 | 默认 8,000、单次上限 32,000 字符（`AudiencePolicy`，与 MCP 分开存放；Run 输入预算约束待内置 Agent 接入时再定） | 默认 8,000、单次上限 32,000 字符 |
| 正文未缓存 | 返回 `bodyNotCached`，界面可提示用户获取 | 默认返回 `bodyNotCached`，不隐式下载；调用方显式传 `download: true` 才下载 |
| 结果去向 | 发送给所选模型提供商，属于内容披露 | 返回给外部客户端，MailX 无法约束其后续处理 |

当前 `AudiencePolicy::DEFAULT` 两个入口取相同数值：搜索默认 20、上限 100 条；正文默认 8,000、上限 32,000 字符。`ToolDescriptor.limits` 返回列出该入口时适用的值。

内置 Agent 的工具结果会进入模型请求，因此每次工具调用都按 [AI Runtime](../ai-runtime/README.md) 的披露规则计入范围；MCP 侧的风险提示在授权界面说明。

## 6. 结果与错误

- 邮件正文、主题和发件显示信息作为不可信材料返回，不能被当作系统指令拼接。
- 结果不包括 API Key、邮箱密码、refresh token、本地路径或无关账号信息。
- 业务失败返回 `ToolError { code, message, field? }`。错误码（camelCase，稳定）：`unknownTool`、`invalidInput`、`notPermitted`、`notFound`、`bodyNotCached`、`downloadTimeout`、`downloadFailed`、`bodyTooLarge`、`unavailable`、`internal`。`message` 是固定的英文短句，不含路径、凭证或服务器文本；`AppError` 只保留错误码映射（`Validation` / `IdempotencyConflict` / `Conflict` → `invalidInput`，`NotFound` → `notFound`，`BodyNotCached` → `bodyNotCached`，`Forbidden` → `notPermitted`，存储 / 核心未就绪 / 密钥库不可用 → `unavailable`，其余 → `internal`）。
- MCP 适配层把参数或协议错误映射为 JSON-RPC 错误，把业务失败映射为工具错误结果。
- 纯文本分页按 Unicode 字符计数，不切断编码；读取不自动标记已读。

## 7. 实施顺序

本地 MCP 是工具层的首个使用方，内置 AI 的前端另行设计。为避免工具层只为 MCP 服务，工具层测试在进程内分别以 MCP 与内置 Agent 主体调用。

1. 共享工具的应用服务支撑：纯文本读取接口、草稿来源标记。已具备（`DraftOrigin::{User, Mcp, Ai}`、`create_draft_with_origin`、界面标记、HTML-only 邮件的分行纯文本）。本地 MCP 已创建来源为 `mcp` 的草稿，内置 Agent 尚未接入。
2. 工具层骨架、`RequestContext` 与二十二个工具：**已完成**，core 内以进程内测试验证两个主体，本地 MCP 已由桌面宿主接入；平台和客户端证据范围见上方状态及验证记录。
3. M0 原型验证 `rmcp` 与本地 IPC，随后实现 MCP 适配层、桌面 IPC 监听与 stdio bridge。
4. 设置中的 MCP 页、标题栏运行状态图标、复制客户端配置与 bridge 随应用分发。
5. 至少两个目标 MCP 客户端联调。
6. 内置 AI 固定工作流接入工具层（另行设计前端）。
7. 服务端：同一 MCP 适配层挂到 Streamable HTTP，远程授权另行设计。

工具按批次开放：每个工具完成 schema、授权、测试与客户端联调后才对 MCP 可见；不预先实现缺少使用方或设计未定的工具。

各阶段仍按[路线图](../roadmap/README.md)的完成标准验收。

## 8. 整理工具扩展（2026-11-03）

本批不增加整理开关，新增动作随 MCP 总开关统一开放；内部仍保留权限与账号范围检查。搜索增加 starred 和三态 archive 筛选，结果及阅读补邮件状态，搜索暴露 coverage。标记结果按字段报告，归档事务返回真实计数，core 事件驱动列表 / 详情 / 目录计数刷新，可信界面使用短期凭据精确撤销。工具注解新增服务器写入、幂等性与外部访问说明，审计上限覆盖一批目标及其账号。完整接口和边界见[整理邮件与 MCP 工具扩展](mail-organize-tools.md)。

## 9. 发送与附件工具（2026-11-04）

本批新增六个工具，不增加额外设置页说明或逐封确认。发送 / 重试统一为 `send_email`；发件箱查询和取消复用核心，分页在存储层按授权范围与状态过滤。附件工具只接收本机普通文件，来源按主体记录，结果不带路径。设计与验证边界见[发送与附件工具](mail-send-tools.md)。
