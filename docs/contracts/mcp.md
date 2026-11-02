# MCP 初始设计

状态：**Draft / 未冻结** · 尚未实现；v0.1 先验证本地 stdio，远程接口后续设计。工具与内置 Agent 共用工具层，见 [ADR 0004](../adr/0004-shared-tool-layer.md) 与[工具层与调用入口](../architecture/agent-tools.md)。

## 1. 角色与协议

MailX 提供 MCP Server，外部 Agent 是 Client。MCP 只是工具层的一种入口：工具定义和处理函数与内置 Agent 共用，MCP 只列出显式声明对外可见的工具。内置 Agent 在进程内直接调用工具层，不经 MCP；MailX 也没有连接第三方 MCP Server 的能力，因此暂不把产品称作“双向 MCP”。

MCP 服务实现驻留在核心所在进程：桌面版由桌面核心托管，服务端版由服务进程托管同一份实现，两者只在传输与认证上不同。

传输以 stdio 与后续 Streamable HTTP 为方向，后者替代早期独立 HTTP+SSE 传输。初次联调以官方 `2025-11-25` 规范作为可复现参考，实际支持版本由 SDK 与目标客户端共同验证并在 initialize 中协商，不把该参考版本称为永远最新。[MCP 传输规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)。

## 2. 桌面连接与授权

### 启用与生命周期

MCP 默认关闭，在设置的“MCP”页用“开启 MCP 服务器”开关启用；开启时弹出确认，说明外部 Agent 将能读取全部账号的本地邮件并创建草稿。开关状态保存在本机设置中，下次启动 MailX 时按该状态自动启动。

MCP 服务随 MailX 进程运行：关窗行为为“在后台运行”时，关闭窗口不影响 MCP；从托盘或菜单真正退出 MailX，或关闭开关时，服务停止并断开所有连接。之后 bridge 返回 `CORE_UNAVAILABLE` 和启动应用的提示。

### 桥接、发现与认证

建议独立桥接程序由客户端以 stdio 启动，`mailx-mcp` 仅为暂用名称，正式可执行程序名称待定。它连接已运行的桌面核心，不打开 SQLite，也不另起同步器。

核心在 macOS / Linux 监听 Unix domain socket（socket 文件 `0600`，所在目录仅当前用户可访问），在 Windows 监听 named pipe（ACL 限定当前用户 SID），并写出发现文件：协议版本、连接端点、随机 token、进程 id 与核心实例 id。Unix socket 路径受系统长度限制，数据目录过长时放在当前用户专属的临时目录。发现文件固定位于数据目录内，原子写入，权限与 socket 相同；退出或关闭开关时只删除属于本实例的文件。token 每次启动服务时重新生成，关闭开关即作废。

bridge 读取发现文件后连接核心，先发送一行握手（bridge 协议版本与 token），核心以常量时间比较校验，通过后 bridge 只在 stdio 与 IPC 之间转发 MCP 消息；initialize、版本协商和工具调用都由核心中的 MCP 服务实现处理。握手版本与 MCP 协议版本分开，bridge 因此基本不随工具变化更新。单条消息大小、并发连接数与空闲超时都有上限，具体数值实现时确定并写入本文。

### 首版授权（有意简化）

首版不做逐客户端配对：开启 MCP 后，同一操作系统用户下能读取发现文件的进程都可以连接，所有连接获得同一档授权“只读加新建草稿”（`accounts:read`、`mail:read`、`draft:create`），范围为全部账号。客户端在 initialize 中自报的名称与版本只用于状态展示和审计，不作为身份依据。核心为每个连接派生 RequestContext，bridge 输入不能自报额外权限。

这意味着开启期间，本机同一用户的任何程序都能读取全部本地邮件。对应的缓解是：默认关闭、开启前确认、主界面常驻运行状态图标、关闭开关立即断开连接并作废 token。逐客户端配对、账号范围与单独撤销留作后续，届时扩展授权记录即可，工具层与 RequestContext 的检查位置不变。

### 状态

核心向宿主报告 MCP 状态：已关闭、运行中（当前连接数与最近连接的客户端自报名称）、启动失败（稳定错误码，例如端点被占用或无法写出发现文件）。设置页与标题栏状态图标都读取这一状态，界面规则见 [UI 设计](../architecture/ui-design.md)。

stdio 的 stdout 只输出协议消息，诊断写 stderr 且脱敏。取消按请求 ID 传递；客户端断开只取消其未完成读取 / AI 请求，不取消已保存草稿或其他客户端任务。

## 3. 工具草案

下表是首批对 MCP 可见的共享工具，对应工具层的 `list_accounts`、`search_emails`、`read_email`、`create_draft`。MCP 专属候选（如 `show_email_in_app`、`get_grant_scope`）与只对内置 Agent 可见的工具见[工具层工具集](../architecture/agent-tools.md#4-工具集草案)，进入实现前再补齐 schema。

工具 JSON Schema 拒绝未知字段；ID 与字符串长度有上限。表内字段采用 camelCase，与应用 DTO 一致。

| 工具 | 输入 | 输出 | 权限 |
| --- | --- | --- | --- |
| `mailx_list_accounts` | `{}` | `accounts[]`：`id, displayName, emailAddress, status`（`status` 目前只有 `unverified`），范围内账号（首版为全部账号） | `accounts:read` |
| `mailx_search_emails` | `query?, accountIds?, receivedAfter?, receivedBefore?, unread?, hasAttachment?, limit?, cursor?` | `emails[]`：`id, accountId, subject, from{name?,address}, receivedAt, preview, isRead, hasAttachment, bodyCached`；`nextCursor` | `mail:read` |
| `mailx_read_email` | `emailId, offset?, maxChars?` | `id, accountId, subject, from, to[], cc[], receivedAt, attachments[]{name,mimeType,size,isInline}, text, nextOffset（末尾为 null）, totalChars, bodyTruncated` | `mail:read` |
| `mailx_create_draft` | `accountId?, to[], cc?, bcc?, subject, bodyHtml, clientMutationId?` | `draftId, accountId, subject` | `draft:create` |

状态：四个工具已在 core 工具层实现（`core/src/tools/`，进程内测试），MCP 适配层尚未实现，以上是适配层要暴露的形状；输入输出 JSON Schema 由工具层的 `ToolDescriptor` 提供，如有出入以其为准。适配层在工具层 id 前加 `mailx_` 前缀。`accountIds` 与授权范围取交集，范围外账号被忽略。`accountId` 缺省时依次用设置中的默认发件账号（须在授权范围内）、授权范围内的账号；`clientMutationId` 须为 UUID，缺省由工具层生成，重复提交同一 id 返回同一草稿。时间参数为 RFC 3339。

限制（已实现于工具层，两个入口各自一份策略值，目前相同）：搜索默认 20 条、上限 100；`accountIds` 最多 20 个；正文默认 8,000 字符、单次上限 32,000；query 最长 256 字符；`cursor` 最长 1,024 字符。正文分页按 Unicode 字符（标量值）计数，`offset` 超过总长度报 `invalidInput`，不会切断编码。读取不自动标记已读。正文未缓存报 `bodyNotCached` 并引导在 UI 获取，首版不让 MCP 隐式触发下载。

草稿限制：`to` / `cc` / `bcc` 各最多 50 个收件人，地址最长 320、显示名最长 200 字符，`subject` 最长 998 字符，`bodyHtml` 最长 200,000 字节，`clientMutationId` 最长 64 字符。

创建草稿可显式指定账号，缺省使用默认发件账号（须在授权范围内），初版不接受宿主附件路径、URL 附件或可执行 HTML；草稿正文提交 `bodyHtml`，核心按写信白名单净化并生成只读 `bodyText`，客户端不能写入派生纯文本。返回不代表发送。UI 展示创建来源，供用户检查后发送：应用服务已提供不可由请求指定的 `Draft.origin`（MCP 主体写 `mcp`，内置 AI 主体经同一工具写 `ai`）及草稿列表 / 写信窗口的“由 MCP 创建”“由 AI 创建”标记；工具层已按主体写入来源，但没有宿主调用它，目前没有任何草稿会带 `mcp` 或 `ai`。

不发布蓝图中语义混合的 `mailx_send_draft`。未来若提供发送工具，另行设计与完整内容哈希 / 草稿版本绑定的一次性确认，而不是由 Agent 提交 `confirmed=true` 自行批准。

`mailx_get_latest_otp` 暂缓。若后续增加，单独定义账号、可信发件域、短时间窗、一次性授权与审计；验证码不写日志。现有 `mail:read` 仍可能读到验证码邮件，因此授权界面应说明正文读取的实际敏感范围。

## 4. 调用与授权

每次执行重新检查授权状态，不能只在 initialize 检查；首版即检查 MCP 开关与连接认证仍然有效，关闭开关后后续调用立即拒绝，后续引入逐客户端授权时同样适用于撤销。只列出已获授权工具，并仍在执行入口检查权限。资源账号范围在查询与读取时检查，已知 emailId 不能绕过账号授权。

工具注解中的只读 / 破坏性标志仅帮助客户端理解行为，不作为安全控制。邮件正文返回为不可信数据，不能追加为 Agent 系统指令。结果不包括 API Key、邮箱密码、refresh token、本地路径或无关账号信息。

参数或协议错误使用 JSON-RPC 错误；业务失败使用工具错误结果并带稳定错误码和简短建议；工具层错误码为 `unknownTool`、`invalidInput`、`notPermitted`、`notFound`、`bodyNotCached`、`unavailable`、`internal`，范围外账号或邮件与不存在的资源同样报 `notFound`，未对该入口开放的工具报 `unknownTool`。记录连接 id、客户端自报名称、动作、资源 ID、结果和 requestId，不记录完整邮件工具结果。

## 5. 远程扩展边界

后续 Web 可提供 HTTPS `/mcp` Streamable HTTP，并验证 Origin、请求大小、速率和会话。服务端在进程内托管与桌面相同的 MCP 服务实现与工具层，不以网关包装 stdio bridge 进程。远程授权单独设计为兼容 MCP 授权规范的资源服务器流程；按规范校验资源 audience、有效期和权限，不接受邮箱或模型提供商的 token。[MCP 授权规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)。

静态 Bearer token 可作为受控开发实验，但不能据此宣称完整标准授权兼容；实际公开部署前再确定授权服务器集成、发现与配对 UX。Session ID 只用于协议会话，不是认证凭据。远程 MCP 未完成时，Web 版本仍可独立发布但保持该端点关闭。

## 6. 分发（草案）

首版建议把 stdio bridge 作为桌面应用的附带可执行程序随安装包分发，MCP 设置页提供“复制客户端配置”，生成带 bridge 绝对路径的配置片段，版本随应用一致。

后续可提供 npm 薄启动器，供客户端配置 `npx` 一类命令：启动器只读取桌面应用写出的发现信息，定位并执行应用自带的 bridge，不打包原生程序，也不实现 MCP 逻辑。包名、npm scope 和是否发布均未决定。不安装桌面应用即可使用的场景属于远程 MCP，不由该启动器解决。

## 7. 参考做法（2026-11-02 调研）

- **Orca**（Electron）：应用运行时监听 `0600` 的 Unix socket / Windows named pipe，消息为按行 JSON 并限制大小、连接数与空闲时间；把端点、随机 token、进程 id 与运行实例 id 写入用户数据目录中的元数据文件，CLI 读取后连接，退出时只清除属于本实例的文件。MailX 的本地 IPC 与发现文件参照这一模式。
- **JetBrains IDE**：在设置中以一个开关启用内置 MCP Server，启用时确认访问范围，提供复制配置与自动写入客户端配置；文档未描述额外认证，等同信任同一用户的本机进程。MailX 首版授权与设置页参照这一取舍。
- **dbx**（Tauri + Rust）：独立 MCP 可执行程序与桌面共用核心 crate，直接读取本地数据，界面类动作经桌面端桥接；内置 AI 工具与 MCP 工具分别实现。MailX 不采用直接读库（核心是单一写入方），也不采用两套工具实现。

以上为调研时对公开仓库与文档的阅读结论，可能随上游变化。
