# MCP 初始设计

状态：**Draft / 未冻结** · 尚未实现；v0.1 先验证本地 stdio，远程接口后续设计。工具与内置 Agent 共用工具层，见 [ADR 0004](../adr/0004-shared-tool-layer.md) 与[工具层与调用入口](../architecture/agent-tools.md)。

## 1. 角色与协议

MailX 提供 MCP Server，外部 Agent 是 Client。MCP 只是工具层的一种入口：工具定义和处理函数与内置 Agent 共用，MCP 只列出显式声明对外可见的工具。内置 Agent 在进程内直接调用工具层，不经 MCP；MailX 也没有连接第三方 MCP Server 的能力，因此暂不把产品称作“双向 MCP”。

MCP 服务实现驻留在核心所在进程：桌面版由桌面核心托管，服务端版由服务进程托管同一份实现，两者只在传输与认证上不同。

传输以 stdio 与后续 Streamable HTTP 为方向，后者替代早期独立 HTTP+SSE 传输。初次联调以官方 `2025-11-25` 规范作为可复现参考，实际支持版本由 SDK 与目标客户端共同验证并在 initialize 中协商，不把该参考版本称为永远最新。[MCP 传输规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)。

## 2. 桌面连接与配对

建议独立桥接程序由客户端以 stdio 启动，`mailx-mcp` 仅为暂用名称，正式可执行程序名称待定。它连接已运行的桌面核心，不打开 SQLite，也不另起同步器；桌面未运行时返回 `CORE_UNAVAILABLE` 和启动应用的提示。

桥接程序不实现 MCP 工具：它与核心完成版本与配对握手后，在 stdio 与本机 IPC 之间转发 MCP 消息，initialize、版本协商和工具调用都由核心中的 MCP 服务实现处理。桥接与核心之间的握手单独带版本号，桥接程序因此基本不随工具变化更新。

桌面设置中显式启用 MCP，为一个客户端创建 `clientId` 与随机配对凭据，绑定账号集合和动作集合。桥接配置以用户专属 ACL 的文件保存凭据引用，不在命令行参数暴露明文。用户可查看、撤销或重建配对。

本地 bridge 使用 Windows named pipe / Unix domain socket，校验同用户权限与配对凭据；Unix socket 所在目录仅用户可访问，Windows 管道限制当前用户 SID。操作系统用户相同不代表可以跳过客户端授权。核心派生 RequestContext，桥接输入不能自报额外权限。

stdio 的 stdout 只输出协议消息，诊断写 stderr 且脱敏。取消按请求 ID 传递；客户端断开只取消其未完成读取 / AI 请求，不取消已保存草稿或其他客户端任务。

## 3. 工具草案

下表是首批对 MCP 可见的共享工具，对应工具层的 `list_accounts`、`search_emails`、`read_email`、`create_draft`。MCP 专属候选（如 `show_email_in_app`、`get_grant_scope`）与只对内置 Agent 可见的工具见[工具层工具集](../architecture/agent-tools.md#4-工具集草案)，进入实现前再补齐 schema。

工具 JSON Schema 拒绝未知字段；ID 与字符串长度有上限。表内字段采用 camelCase，与应用 DTO 一致。

| 工具 | 输入 | 输出 | 权限 |
| --- | --- | --- | --- |
| `mailx_list_accounts` | `{}` | 获授权账号 id、显示名、状态 | `accounts:read` |
| `mailx_search_emails` | `query?, accountId?, limit?, cursor?, receivedAfter?, receivedBefore?` | 限量摘要、游标、缓存覆盖说明 | `mail:read` |
| `mailx_read_email` | `emailId, offset?, maxChars?` | 纯文本片段、发送方、附件元信息、nextOffset | `mail:read` |
| `mailx_create_draft` | `accountId, to[], cc?, bcc?, subject, body, clientMutationId` | draftId、revision、需用户检查的提示 | `draft:create` |

建议限制：搜索默认 20 条、上限 100；正文默认 8,000 字符、单次上限 32,000；query 最长 256 字符，正文分页按 Unicode 字符计数，不能切断编码。读取不自动标记已读。正文未缓存报 `BODY_NOT_CACHED` 并引导在 UI 获取，首版不让 MCP 隐式触发大范围下载。

创建草稿必须显式指定账号，初版不接受宿主附件路径、URL 附件或可执行 HTML；草稿正文提交 `bodyHtml`，核心按写信白名单净化并生成只读 `bodyText`，客户端不能写入派生纯文本。返回不代表发送。UI 展示创建来源，供用户检查后发送。

不发布蓝图中语义混合的 `mailx_send_draft`。未来若提供发送工具，另行设计与完整内容哈希 / 草稿版本绑定的一次性确认，而不是由 Agent 提交 `confirmed=true` 自行批准。

`mailx_get_latest_otp` 暂缓。若后续增加，单独定义账号、可信发件域、短时间窗、一次性授权与审计；验证码不写日志。现有 `mail:read` 仍可能读到验证码邮件，因此授权界面应说明正文读取的实际敏感范围。

## 4. 调用与授权

每次执行重新检查授权状态，不能只在 initialize 检查；撤销后后续读取立即拒绝。只列出已获授权工具，并仍在执行入口检查权限。资源账号范围在查询与读取时检查，已知 emailId 不能绕过账号授权。

工具注解中的只读 / 破坏性标志仅帮助客户端理解行为，不作为安全控制。邮件正文返回为不可信数据，不能追加为 Agent 系统指令。结果不包括 API Key、邮箱密码、refresh token、本地路径或无关账号信息。

参数或协议错误使用 JSON-RPC 错误；业务失败使用工具错误结果并带稳定错误码和简短建议。记录 clientId、动作、资源 ID、结果和 requestId，不记录完整邮件工具结果。

## 5. 远程扩展边界

后续 Web 可提供 HTTPS `/mcp` Streamable HTTP，并验证 Origin、请求大小、速率和会话。服务端在进程内托管与桌面相同的 MCP 服务实现与工具层，不以网关包装 stdio bridge 进程。远程授权单独设计为兼容 MCP 授权规范的资源服务器流程；按规范校验资源 audience、有效期和权限，不接受邮箱或模型提供商的 token。[MCP 授权规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)。

静态 Bearer token 可作为受控开发实验，但不能据此宣称完整标准授权兼容；实际公开部署前再确定授权服务器集成、发现与配对 UX。Session ID 只用于协议会话，不是认证凭据。远程 MCP 未完成时，Web 版本仍可独立发布但保持该端点关闭。

## 6. 分发（草案）

首版建议把 stdio bridge 作为桌面应用的附带可执行程序随安装包分发，设置页生成带绝对路径的客户端配置片段，版本随应用一致。

后续可提供 npm 薄启动器，供客户端配置 `npx` 一类命令：启动器只读取桌面应用写出的发现信息，定位并执行应用自带的 bridge，不打包原生程序，也不实现 MCP 逻辑。包名、npm scope 和是否发布均未决定。不安装桌面应用即可使用的场景属于远程 MCP，不由该启动器解决。
