# MCP 发信与草稿附件

状态：**Implemented**。MCP 发送与草稿附件工具随总开关统一授权，直接发信无需逐封 UI 确认；发送沿用应用发送队列，邮件删除不开放。平台、客户端和服务商证据见[验证记录](../development/validation.md#mcp-send-attachments)。

## 工具

| 工具 | 输入 | 结果 / 动作 |
| --- | --- | --- |
| `send_email` | `action: send, draftId, expectedRevision, clientMutationId` | 冻结草稿并入队，返回发送任务 |
| `send_email` | `action: retry, outboxId` | 唤醒已有 queued 任务，不生成第二项，不跳过撤销窗口 |
| `list_outbox` | `accountId?, states?[], limit?, cursor?` | `items[]` 与可选 `nextCursor`；默认 20、最大 100 |
| `get_outbox` | `outboxId` | 单个发送任务 |
| `cancel_outbox` | `outboxId` | 仅取消 queued，返回更新后的任务 |
| `add_draft_attachment` | `draftId, expectedRevision, filePath, fileName?` | 从本机绝对路径添加普通附件，返回新 revision 与附件元信息 |
| `remove_draft_attachment` | `draftId, expectedRevision, attachmentId` | 移除普通草稿附件，返回新 revision 与 `removed` |

对外名称带 `mailx_`。发送与重试共用一个工具，严格拒绝 action 不匹配的字段。发件箱结果只含 id、账号 / 草稿引用、`senderEmailAddress`、主题、最多三个收件人与总数、状态、尝试次数、时间、人工处理结果及稳定错误类别 / SMTP 代码；不返回服务器原文、路径、MIME 或凭证。

账号列表返回邮箱地址及 `isDefault`，标记当前授权范围内新草稿的实际默认账号。创建、回复、列出和读取草稿直接返回当前 `senderEmailAddress`；发送及发件箱结果从冻结的 SMTP 信封取地址，账号地址后续改变也不会改写历史。指定发件人时先按账号列表选择，向用户回报时使用返回的邮箱地址。

## 授权与发送

- 发送 / 重试需要 `mail:send`，发件箱读取需要 `outbox:read`，取消需要 `outbox:write`，附件增删需要 `draft:write`，添加本机文件另需 `files:read`。这些动作均加入本地 MCP 默认授权；内置 Agent 必须由宿主授予相应动作与账号范围。
- 不增加发送开关、申请批准工具或 `confirmed` 参数。开启确认用短句说明可直接发信和读取本机文件作为附件，已开启的 MCP 升级后也获得这些能力。
- 调用方身份来自宿主。MCP / 模型不伪装成写信窗口；核心允许它们走正常入队流程，MainWindow 仍不能入队。`unknown` 的人工处理接口仍只接受用户 UI。
- `send` 必须带 UUID 幂等键、草稿 ID 和当前 revision。重复键只重放同草稿同版本的任务，包括已发送后草稿已删除的情况；账号范围仍检查。内容不同不能复用键，并发重试也不能重复入队。
- 用户正打开的草稿拒绝发送与附件修改，避免发送尚未保存的内容。核心复查 revision 与锁定状态，并冻结发件身份、收件人、正文和附件。
- 桌面注入 `compose.undoSendSeconds`，读取失败则阻止入队；发送任务继续走账号路由、既有 SMTP 工作线程与事件通知。关闭 MCP 阻止新的工具调用，不撤销已入队任务。

## 状态与分页

`queued` 是等待发送，`sending` 是已开始尝试，`accepted` 表示 SMTP 服务器接受（或用户手动标为已发送），不能表述为已投递到收件箱。`failed` 是确定失败，`unknown` 是结果不确定，`canceled` 是已取消。人工处理通过 `resolution` 标识：`markedSent` 为用户标记已发送，`resent` 为用户选择重发，此时 `supersededBy` 指向新任务。`retry` 只适用于 queued，不重发 failed / unknown；`cancel` 与工作线程竞争时由存储事务裁决，已经开始发送返回 `alreadySending`。

列表默认只含 queued、sending、failed 和未处理的 unknown；显式 states 可查询其他状态。账号范围与状态在 SQL 分页前过滤，按创建时间与 ID 倒序；游标绑定查询条件，不能拿另一个账号或过滤条件的游标继续查询。

## 附件

附件工具读取 MailX 所在电脑上的普通文件，复制进现有 blob 存储；客户端自己的远程路径不可用。拒绝 URL、UNC、设备路径、目录与特殊文件；读取有界，沿用单封合计 20 MiB / 20 个附件限制。可选 fileName 只影响展示名，由核心清理。结果与错误不回传本机路径。

来源由宿主主体决定为 mcp / ai，不可由输入伪造；桌面文件和转发附件仍为 user / forwarded。`read_draft.attachments` 补充稳定 id 和 origin，供后续移除。修改增加 revision，已锁定或用户正打开的草稿拒绝修改；本批只管理普通附件，内嵌图片沿用现有写信窗口流程。

## 验证

验证需要覆盖发送授权、账号范围、参数组合、草稿版本与打开保护、成功后 / 并发幂等重放、取消与领取竞争、重试不越过撤销窗口、unknown 不自动重发、分页范围与游标，以及附件文件类型、大小、来源和路径脱敏。协议测试经过本地 MCP 与 stdio bridge；自动测试使用合成数据，不替代真实客户端和邮箱手测。执行证据记入[验证记录](../development/validation.md)。
