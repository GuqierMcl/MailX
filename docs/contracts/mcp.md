# MCP 初始设计

状态：**Draft / 未冻结** · 本地 MCP 后端（服务端库、stdio bridge、桌面生命周期与状态接口）已有实现，实现细节见[第 9 节](#9-后端实现状态2026-11-02)；设置页、标题栏状态图标、复制配置和 bridge 随应用分发（Tauri sidecar）已有实现，尚未在任何平台手测，也未与真实 MCP 客户端联调。远程接口后续设计。工具与内置 Agent 共用工具层，见 [ADR 0004](../adr/0004-shared-tool-layer.md) 与[工具层与调用入口](../architecture/agent-tools.md)。

## 1. 角色与协议

MailX 提供 MCP Server，外部 Agent 是 Client。MCP 只是工具层的一种入口：工具定义和处理函数与内置 Agent 共用，MCP 只列出显式声明对外可见的工具。内置 Agent 在进程内直接调用工具层，不经 MCP；MailX 也没有连接第三方 MCP Server 的能力，因此暂不把产品称作“双向 MCP”。

MCP 服务实现驻留在核心所在进程：桌面版由桌面核心托管，服务端版由服务进程托管同一份实现，两者只在传输与认证上不同。

传输以 stdio 与后续 Streamable HTTP 为方向，后者替代早期独立 HTTP+SSE 传输。初次联调以官方 `2025-11-25` 规范作为可复现参考，实际支持版本由 SDK 与目标客户端共同验证并在 initialize 中协商，不把该参考版本称为永远最新。M0 原型中 `rmcp` 3.8 对 `2025-11-25`、`2025-06-18`、`2024-11-05` 按请求版本应答，对无状态的 `2026-07-28` 或未知版本回落为 `2025-11-25`；Claude Code 2.1.316 实测使用无状态的 `2026-07-28`（见第 9 节“协议版本与客户端联调”）。[MCP 传输规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)。

## 2. 桌面连接与授权

### 启用与生命周期

MCP 默认关闭，在设置的“MCP”页用“开启 MCP 服务器”开关启用；开启时弹出确认，说明外部 Agent 将能读取全部账号的本地邮件并创建草稿。开关状态保存在本机设置中，下次启动 MailX 时按该状态自动启动。

MCP 服务随 MailX 进程运行：关窗行为为“在后台运行”时，关闭窗口不影响 MCP；从托盘或菜单真正退出 MailX，或关闭开关时，服务停止并断开所有连接。之后 bridge 返回 `CORE_UNAVAILABLE` 和启动应用的提示。

### 桥接、发现与认证

建议独立桥接程序由客户端以 stdio 启动，`mailx-mcp` 仅为暂用名称，正式可执行程序名称待定。它连接已运行的桌面核心，不打开 SQLite，也不另起同步器；bridge 只依赖异步运行时与 JSON 解析，不链接 core。

核心在 macOS / Linux 监听 Unix domain socket（socket 文件 `0600`，所在目录仅当前用户可访问），在 Windows 监听 named pipe（ACL 限定当前用户 SID），并写出发现文件：协议版本、连接端点、随机 token、进程 id 与核心实例 id。socket 优先放在数据目录内；macOS `sun_path` 可用 103 字节，默认路径约为 65 字节加用户名长度，超过约 100 字节时改用当前用户专属的临时目录（macOS `$TMPDIR/mailx-<uid>/`，Linux `$XDG_RUNTIME_DIR`）。socket 所在目录由宿主显式创建为 `0700`，core 不会替宿主设置目录权限。绑定前必须已持有单实例锁，否则第二个实例会删除第一个实例的 socket。发现文件固定位于数据目录内，端点写绝对路径，经临时文件加重命名原子写入，权限 `0600`；退出或关闭开关时只删除 instanceId 相同的文件。进程崩溃会留下过期发现文件，bridge 连接被拒时按核心不可用处理。token 由密码学安全随机数生成，每次启动服务时重新生成，关闭开关即作废。Windows 版需要 `unsafe` 构造安全描述符，而工作区禁止 `unsafe`，因此 IPC 代码放在单独配置 lint 的 crate 中（见第 8 节）。

bridge 读取发现文件后连接核心，先发送一行握手（bridge 协议版本与 token），核心以常量时间比较校验，通过后 bridge 只在 stdio 与 IPC 之间转发 MCP 消息；initialize、版本协商和工具调用都由核心中的 MCP 服务实现处理。握手版本与 MCP 协议版本分开，bridge 因此基本不随工具变化更新。

唯一例外是核心不可用：发现文件缺失、连接被拒或握手失败时，bridge 对收到的第一个请求回 JSON-RPC 错误（携带 `CORE_UNAVAILABLE` 与“请先启动 MailX 并开启 MCP”的提示），对通知不回应，同时向 stderr 写说明并以非零状态退出。M0 中只退出不回应时，客户端只能看到连接关闭，看不到原因。bridge 为此只解析到能回填请求 `id` 的程度。

限制（M0 实测值，作为首版初值）：握手行不超过 512 字节、5 秒内完成，否则直接关闭；单条 MCP 消息不超过 1 MiB。`rmcp` 对超长消息只丢弃、不回错误，客户端会一直等待。已实现的处理见第 9 节：超限即关闭该连接。并发连接上限为 8（第 9 节）。

客户端在调用中途断开时，正在执行的工具调用会跑完（工具层是阻塞调用），不会写一半草稿；如需可取消的工具，需在工具层另行设计。

### 首版授权（有意简化）

首版不做逐客户端配对：开启 MCP 后，同一操作系统用户下能读取发现文件的进程都可以连接，所有连接获得同一档授权“只读加新建草稿”（`accounts:read`、`mail:read`、`draft:create`），范围为全部账号。客户端在 initialize 中自报的名称与版本只用于状态展示和审计，不作为身份依据。核心为每个连接派生 RequestContext，bridge 输入不能自报额外权限。

这意味着开启期间，本机同一用户的任何程序都能读取全部本地邮件。对应的缓解是：默认关闭、开启前确认、主界面常驻运行状态图标、关闭开关立即断开连接并作废 token。逐客户端配对、账号范围与单独撤销留作后续，届时扩展授权记录即可，工具层与 RequestContext 的检查位置不变。

### 状态

核心向宿主报告 MCP 状态：已关闭、运行中（当前已连接的各会话及其客户端自报标签，按连接先后排列）、启动失败（稳定错误码，例如端点被占用或无法写出发现文件）。设置页与标题栏状态图标都读取这一状态，界面规则见 [UI 设计](../architecture/ui-design.md)。

工具映射：MCP 工具名为工具层 ID 加 `mailx_` 前缀；不带前缀的名称返回 JSON-RPC `-32602`。成功结果同时给 `structuredContent` 与 JSON 文本兜底；`ToolError` 返回 `isError: true`，错误 JSON 只放文本内容、不放 `structuredContent`，以免严格客户端按 `outputSchema` 校验失败。`readOnly` 映射为 `readOnlyHint`；所需动作与限制在 MCP 中没有对应字段，放入 `_meta`。客户端自报名称取自 initialize 的 `clientInfo`。

stdio 的 stdout 只输出协议消息，诊断写 stderr 且脱敏。取消按请求 ID 传递；客户端断开只取消其未完成读取 / AI 请求，不取消已保存草稿或其他客户端任务。

## 3. 工具草案

下表是首批对 MCP 可见的共享工具，对应工具层的 `list_accounts`、`search_emails`、`read_email`、`create_draft`。MCP 专属候选（如 `show_email_in_app`、`get_grant_scope`）与只对内置 Agent 可见的工具见[工具层工具集](../architecture/agent-tools.md#4-工具集草案)，进入实现前再补齐 schema。

工具 JSON Schema 拒绝未知字段；ID 与字符串长度有上限。表内字段采用 camelCase，与应用 DTO 一致。

| 工具 | 输入 | 输出 | 权限 |
| --- | --- | --- | --- |
| `mailx_list_accounts` | `{}` | `accounts[]`：`id, displayName, emailAddress, syncStatus, lastSyncedAt?`，范围内账号（首版为全部账号）。`syncStatus` 取自同步状态：`ok`（最近一轮同步成功）、`syncing`、`authError`（认证、凭据缺失 / 不可信、服务商策略；需用户在 MailX 中处理）、`connectionError`（网络、代理、TLS、超时、协议或本地存储等其他失败）、`unknown`（本次启动尚无同步结果，不表示异常）；不含服务器原文。`lastSyncedAt` 为 RFC 3339，本次启动后尚无成功同步时省略 | `accounts:read` |
| `mailx_search_emails` | `query?, accountIds?, folderRole?, receivedAfter?, receivedBefore?, unread?, hasAttachment?, limit?, cursor?` | `emails[]`：`id, accountId, subject, from{name?,address}, receivedAt, preview, isRead, hasAttachment, bodyCached`；`nextCursor` | `mail:read` |
| `mailx_read_email` | `emailId, offset?, maxChars?` | `id, accountId, subject, from, to[], cc[], receivedAt, attachments[]{name,mimeType,size,isInline}, text, nextOffset（末尾省略）, totalChars, bodyTruncated` | `mail:read` |
| `mailx_create_draft` | `accountId?, to[], cc?, bcc?, subject, bodyHtml, clientMutationId?` | `draftId, accountId, subject` | `draft:create` |

状态：四个工具已在 core 工具层实现（`core/src/tools/`，进程内测试），MCP 适配层尚未实现，以上是适配层要暴露的形状；输入输出 JSON Schema 由工具层的 `ToolDescriptor` 提供，如有出入以其为准。适配层在工具层 id 前加 `mailx_` 前缀。`accountIds` 与授权范围取交集，范围外账号被忽略。`search_emails` 默认不含“已删除”（trash）与“垃圾邮件”（junk）文件夹；`folderRole`（`inbox`、`sent`、`drafts`、`archive`、`junk`、`trash`、`other`）给出时只搜该角色的文件夹，可借此查看已删除或垃圾邮件，无效取值报 `invalidInput`。按文件夹 id 过滤要等将来的 `list_folders` 工具。本机归档（仅本机、不动服务器）的规则不变：不带 query 时隐藏，带 query 的文本搜索会找到它们。`accountId` 缺省时依次用设置中的默认发件账号（须在授权范围内）、授权范围内的账号；`clientMutationId` 须为 UUID，缺省由工具层生成，重复提交同一 id 返回同一草稿。时间参数为 RFC 3339。

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

## 6. 分发

stdio bridge 作为 Tauri sidecar（`bundle.externalBin = ["binaries/mailx-mcp"]`）随安装包分发，版本与应用一致。

- **构建**：`scripts/mcp-sidecar.mjs` 构建 `mailx-mcp-bridge`（cargo 二进制名同为 `mailx-mcp-bridge`）并复制为 `desktop/binaries/mailx-mcp-<target triple>[.exe]`（该目录已加入 `.gitignore`）。`beforeDevCommand` 以 debug、`beforeBuildCommand` 以 release 调用它，并遵循 Tauri 提供的 `TAURI_ENV_TARGET_TRIPLE` / `TAURI_ENV_DEBUG`；`universal-apple-darwin` 会构建两个 macOS 架构后用 `lipo` 合并。Tauri 的 build script 在文件缺失时就会失败，因此 `bun run rust:check` / `rust:clippy` / `rust:test` 先放置一个空占位文件（`--placeholder`，只用于编译检查，不会进入安装包）。发布工作流在各自原生三元组的 runner 上运行 `tauri build`，由 `beforeBuildCommand` 取得正确的 bridge，无需另设步骤；当前不构建 universal 包。
- **命名**：cargo 二进制叫 `mailx-mcp-bridge`，随包分发的 sidecar / 客户端命令叫 `mailx-mcp`。两者故意不同：`tauri-build` 会把 sidecar 去掉三元组后复制到 `target/<profile>/mailx-mcp`，若 cargo 二进制同名，空占位文件会覆盖真实 bridge（Windows CI 因此出现 `%1 is not a valid Win32 application`）。后端发现 bridge 为空文件时按“找不到 bridge”处理。
- **运行时位置**：Tauri 把外部程序放在主程序旁（macOS `MailX.app/Contents/MacOS/mailx-mcp`，Windows 安装目录，deb / rpm 为 `/usr/bin`），`tauri dev` 同样复制到调试程序旁。后端按“主程序所在目录下的 `mailx-mcp`”解析。
- **AppImage**：挂载路径每次启动都变，不能写进客户端配置。检测到环境变量 `APPIMAGE` 时，后端把 bridge 复制到 `<数据目录>/mcp/bin/mailx-mcp`（目录 `0700`，文件 `0755`，先写临时文件再改名；内容与打包版不同才覆盖），并返回该稳定路径。
- **复制客户端配置**：命令 `mcp_client_config`（仅主窗口）返回 `{ command, args }`，`args` 恒为 `["--discovery", "<发现文件绝对路径>"]`，因为数据目录可能被用户改过。前端复制的 JSON 为 `{"mcpServers":{"mailx":{"command":…,"args":[…]}}}`（缩进两格）。该命令不读取 token。

后续可提供 npm 薄启动器，供客户端配置 `npx` 一类命令：启动器只读取桌面应用写出的发现信息，定位并执行应用自带的 bridge，不打包原生程序，也不实现 MCP 逻辑。包名、npm scope 和是否发布均未决定。不安装桌面应用即可使用的场景属于远程 MCP，不由该启动器解决。

## 7. 参考做法（2026-11-02 调研）

- **Orca**（Electron）：应用运行时监听 `0600` 的 Unix socket / Windows named pipe，消息为按行 JSON 并限制大小、连接数与空闲时间；把端点、随机 token、进程 id 与运行实例 id 写入用户数据目录中的元数据文件，CLI 读取后连接，退出时只清除属于本实例的文件。MailX 的本地 IPC 与发现文件参照这一模式。
- **JetBrains IDE**：在设置中以一个开关启用内置 MCP Server，启用时确认访问范围，提供复制配置与自动写入客户端配置；文档未描述额外认证，等同信任同一用户的本机进程。MailX 首版授权与设置页参照这一取舍。
- **dbx**（Tauri + Rust）：独立 MCP 可执行程序与桌面共用核心 crate，直接读取本地数据，界面类动作经桌面端桥接；内置 AI 工具与 MCP 工具分别实现。MailX 不采用直接读库（核心是单一写入方），也不采用两套工具实现。

以上为调研时对公开仓库与文档的阅读结论，可能随上游变化。

## 8. M0 原型结论（2026-11-02）

在仓库外的临时 crate 中验证：真实 `MailxCore`（临时数据目录）→ 工具层 → 手写 `rmcp` `ServerHandler` → Unix socket + token 握手 → bridge → `rmcp` 客户端。macOS 上 initialize、工具列表、`mailx_list_accounts`、`mailx_search_emails`、输入错误与未知工具均按预期返回；两个客户端并发会话互不影响；`lsof` 显示 bridge 进程只持有标准输入输出与 socket，没有打开数据目录或 SQLite。

- **SDK**：`rmcp` ~3.8（固定次版本），`default-features = false, features = ["server"]`。在 core 之上只新增约 7 个包，不引入 axum、hyper、reqwest、tower；类型多为 `#[non_exhaustive]`，需用构造器，跨小版本时有 API 变化，应锁定小版本。
- **crate 布局**：core 不依赖 `rmcp`，只保留工具层。新增 MCP 适配 crate（适配层、IPC 监听、发现文件、握手，自带 lint 配置以容纳 Windows 的 `unsafe`），由桌面宿主链接；新增 bridge 可执行 crate，只依赖异步运行时与 JSON 解析。crate 名称尚未决定。
- **Windows**：tokio named pipe 支持 `first_pipe_instance`、`reject_remote_clients` 与自定义安全属性，配合 `windows-sys` 可把 DACL 限定为当前用户 SID。只查了 API，未编译、未运行。
- **未验证**：Windows 与 Linux 运行、崩溃后的过期发现文件清理、并发连接上限、其他 MCP 客户端（Cursor、Claude Desktop 等）联调（M0 时的结论；macOS 上 Inspector CLI 与 Claude Code 的联调见第 9 节）。

## 9. 后端实现状态（2026-11-02）

以下为已实现部分。crate 与可执行程序名称仍是候选，不是冻结的命名决定。

**crate 与二进制**

- `mcp-server/`（crate `mailx-mcp-server`，库）：手写 `rmcp` `ServerHandler`、IPC 监听、握手、发现文件、状态。自带 `[lints.rust] unsafe_code = "deny"`，仅 Windows 命名管道模块以 `#![allow(unsafe_code)]` 放行，每个 `unsafe` 块都有 SAFETY 注释。公开接口：`McpServer::start(McpConfig { data_dir, tools }) -> Result<McpHandle, McpStartError>`、`McpHandle::{stop, status, subscribe}`、`McpStatus`、`McpFailureCode`、`StderrAudit`。可重复启停。
- `mcp-bridge/`（crate `mailx-mcp-bridge`，cargo 二进制 `mailx-mcp-bridge`，随包名称 `mailx-mcp`）：只依赖 tokio 与 `serde_json`，不链接 core，只读取发现文件。
- 桌面：`desktop/src/mcp.rs`；设置项 `mcp.enabled`（`AppSettings.mcp`，默认 `false`，只能经 `SettingsStore::set_mcp_enabled` 修改，通用设置保存不会改变它）；命令 `mcp_get_status`、`mcp_set_enabled`（沿用现有 `<模块>_<动词>` 命名）；事件 `mailx://mcp-status-changed`。

**端点与权限**

- Unix：首选 socket 放在数据目录下专用子目录 `<数据目录>/mcp/mcp.sock`；该子目录显式创建为 `0700` 并校验是真实目录、属主为当前用户，socket `chmod 0600`；数据目录本身只在缺失时创建，权限从不修改。残留 socket 若无人应答则删除，有人应答则报 `endpointInUse`，非 socket 文件不删除。完整 socket 路径超过 100 字节时改用 `$TMPDIR/mailx-<uid>/mcp-<数据目录哈希>.sock`（macOS）或 `$XDG_RUNTIME_DIR/mailx/`（Linux，缺失时 `/tmp/mailx-<uid>/`）；回退目录必须属于当前用户，否则报 `permissionDenied`。发现文件记录最终绝对路径。
- Windows：管道名 `\\.\pipe\mailx-mcp-<instanceId>`，`first_pipe_instance(true)`、`reject_remote_clients(true)`，DACL 为 `D:P(A;;GA;;;<当前用户 SID>)`，在接受连接前先创建下一个实例。仅编译层面核对，未在 Windows 上运行。
- 发现文件 `mcp-discovery.json`：`{version, endpoint, token, pid, instanceId}`，临时文件加重命名，Unix `0600`；停止时仅当 `instanceId` 相同才删除。token 为 `getrandom` 取 32 字节、十六进制，每次启动重新生成。

**握手与限制**

- 握手行 `{"bridge":1,"token":"…"}`，逐字节读取（不消耗 MCP 字节）、≤512 字节、5 秒；比较用 `subtle` 常量时间；失败回 `{"ok":false}` 并关闭。
- 单条 MCP 消息 ≤1 MiB。超限的请求**得不到 JSON-RPC 错误**：未解析就不知道 `id`，而 `rmcp` 3.8 只丢弃不回错。服务端在 `rmcp` 之前加了读取包装，超限即结束该会话，客户端看到连接关闭（而不是无限等待）。
- 并发会话上限 8（`MAX_CONNECTIONS`）：握手通过但已满时回 `{"ok":false}` 并关闭；握手前同时在途的连接上限 32。
- 每次工具列出与调用都重新检查服务器是否仍启用；停止时先使 token 作废（删除发现文件），再断开全部会话、移除 socket，`stop()` 等待完成。
- 工具映射按第 2 节：`mailx_` 前缀、`outputSchema` 透传、`readOnlyHint`、`_meta` 中的 `mailx/requiredActions`、`mailx/limits`、`mailx/writesLocalData`；`call_tool` 经 `spawn_blocking(Tools::call)`；`Principal::LocalMcp` 的连接 id 每连接新生成，客户端名取自 `RequestContext::client_info()`：会话式客户端为 `initialize` 的 `clientInfo.name`，无状态客户端为每个请求 `params._meta["io.modelcontextprotocol/clientInfo"].name`；审计取 `name` 而不是展示用的 `title`（稳定标识，审计更易检索）；状态里的展示标签则优先取非空 `title`、否则取 `name`（二者都去控制字符、截断 64 字符），二者都是客户端自报、不可信，只用于展示与审计、绝不作身份依据；界面直接显示名称，不加“自行报告”之类的限定语（面向非技术用户的措辞决定，安全规则不变）；`initialize` 完成、`list_tools` 与 `call_tool` 都会把本会话的标签写入状态；会话结束即从状态中移除，未自报身份的会话标签为 `null`（界面显示“未知客户端”）。界面显示当前已连接的客户端，不再保留“最近连接”。授权为 `Grant::mcp_read_and_draft()`。
- 审计：`StderrAudit` 每次调用向 stderr 写一行（工具 id、主体、客户端名、请求 id、资源个数、结果），不含参数与内容。桌面宿主目前只有 stderr 这一种日志通道。

**协议版本与客户端联调（2026-11-02）**

- Claude Code 2.1.316 使用无状态协议 `2026-07-28`：先发 `server/discover`，**不发** `initialize`，并在每个请求的 `params._meta` 带 `io.modelcontextprotocol/protocolVersion` 与 `io.modelcontextprotocol/clientInfo`（`name`、`title`、`version` 等）。经 bridge 可用，bridge 只转发字节，无需改动。
- `server/discover`：`rmcp` 3.8 的 `ServerHandler` 有默认的 `discover` 实现，`_meta` 带 `protocolVersion` 与 `clientCapabilities` 时正常应答（`supportedVersions` 含 `2026-07-28`，服务端信息在 `result._meta["io.modelcontextprotocol/serverInfo"]`）。缺这些 `_meta` 的 `server/discover` 由 `rmcp` 内部的统一入口（blanket `Service` 实现，先于任何 handler 方法）校验并回 `-32602`；`ServerHandler` 无法覆盖这一步，要改只能自己包一层 `Service`，我们没有这么做。实测 Claude Code 容忍这个错误，并继续用 `_meta` 调用 `tools/list` 与 `tools/call`。
- 测试 `stateless_client_without_initialize_gets_its_name_from_request_meta` 覆盖：带 `_meta` 的 discover 成功、不带的回 `-32602`、不 initialize 直接 `tools/list` / `tools/call` 成功且状态里出现客户端名、无 `clientInfo` 时匿名调用仍成功。
- 联调结果（仅 macOS）：MCP Inspector CLI 与 Claude Code 2.1.316 各自成功调用了四个工具。Windows、Linux 与其他客户端（Cursor、Claude Desktop 等）未测试。
- 重启与断线（Claude Code 实测）：MailX 重启或关闭 MCP 开关时，bridge 按设计退出；Claude Code 随即把该服务器标为断开，旧连接上进行中的调用以“Connection closed”失败，用户需用 `/mcp` 重新连接。
- 并发（Claude Code 实测）：在同一条存活连接上，经 bridge 并行发起 4 个 `tools/call` 均成功。

**状态码**（`McpFailureCode`，camelCase）：`endpointInUse`、`endpointCreateFailed`、`permissionDenied`、`discoveryWriteFailed`、`tokenUnavailable`、`acceptFailed`。`McpStatus` 序列化为 `{"state":"off"}`、`{"state":"running","clients":[{"label":"Claude Code"},{"label":null}]}`、`{"state":"failed","code":…}`。

**桌面生命周期**：单实例插件最先注册，第二个实例在到达 `setup` 前就退出，因此只有持锁进程会启动服务。服务在 core 就绪后（就绪回调）按 `mcp.enabled` 启动；开关切换后即时启停；`RunEvent::Exit` 时停止（先于更新器的收尾），后台模式下关窗不影响服务。崩溃遗留的 socket 与发现文件由下一次启动清理，bridge 连接被拒时按核心不可用处理。`Tools` 的默认账号来自写信设置里的默认账号。

**bridge**：`mailx-mcp [--discovery <路径>]`；发现文件优先级为 `--discovery`、环境变量 `MAILX_MCP_DISCOVERY`、`MAILX_DATA_DIR/mcp-discovery.json`、再按平台默认数据目录（macOS `~/Library/Application Support/MailX`，Windows `%APPDATA%\MailX`，其他 `$XDG_DATA_HOME` 或 `~/.local/share` 下的 `MailX`）。这一默认值**复制自** `desktop/src/storage_location.rs`，两处需同步修改。核心不可用时对第一个带 `id` 的请求回 JSON-RPC 错误：`code = -32001`，`message` 以 `CORE_UNAVAILABLE:` 开头并附中英文提示，`data = {"code":"CORE_UNAVAILABLE","reason":…}`，`reason` 为 `discoveryMissing`、`discoveryInvalid`、`connectFailed`、`handshakeRejected` 之一；通知不回应；等待至多 30 秒；stderr 写一行说明。stdin 到达 EOF 即视为客户端结束（命名管道没有半关闭）：bridge 刷新写端，给服务端最多 2 秒（`DRAIN_GRACE`）送完剩余回复，然后丢弃 IPC 连接并以 `0` 退出，服务端随连接断开释放会话；服务端先关闭时（例如用户关闭 MCP）立即结束并以 `0` 退出，即使客户端仍保持 stdin 打开；bridge 以 `shutdown_background` 结束运行时，不等待无法取消的 stdin 阻塞读取（核心不可用路径同样适用）。退出码：`0` 正常结束，`64` 参数错误，`69` 核心不可用。

**设置页与分发**：设置 → MCP 页含开关（开启前确认）、状态行、说明与复制配置；标题栏状态图标见 [UI 设计](../architecture/ui-design.md)。命令 `mcp_client_config` 的实现与 AppImage 复制逻辑在 `desktop/src/mcp_sidecar.rs`，有自动测试。

**仍未验证**：各平台实机手测（含设置页、标题栏图标、复制配置和 sidecar 路径）、AppImage 实机复制、Windows 实机运行（仅 CI 编译与测试）、Linux 手测、睡眠唤醒、真实 MCP 客户端联调、进程崩溃后的清理路径手测。
