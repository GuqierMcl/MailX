# 日程与待办识别及系统添加

状态：**Proposed** · 覆盖从邮件中识别日程与待办（L0 结构化日历、L1 本地规则与正文高亮、L2 内置 AI 结构化提取）、阅读区交互、统一编辑确认与本机状态持久化，以及分阶段写入系统日历与待办的架构边界。界面规则以[界面设计语言](ui-design.md)和[AI 交互界面](ai-chat-ui.md)为准，安全与外发边界遵循[网络与安全](network-security.md)与 [AI Runtime](../ai-runtime/README.md)。

> 落地同步约束：本方案将阅读区「AI 摘要」入口升格为「AI」并补齐待办提取能力。实现合入时，以本文为新设计依据，同步更新[AI 交互界面](ai-chat-ui.md)（5.2 与第 9 节）、[数据库迁移与启动](database-migrations.md)、[应用 API](../contracts/app-api.md)，以及中英文用户指南与隐私说明（说明自动 AI 识别的触发条件，以及添加到系统日历的备注默认含发件人且可能随用户日历同步到云端）。

## 1. 目标与分期边界

本能力解决邮件阅读中的高频场景：从会议邀请、时间约定与行动请求中提取日程和待办，在用户确认和微调后添加到系统日历或待办工具。

| 阶段 | 范围 | 交付终点 |
| --- | --- | --- |
| **第一阶段（三平台闭环）** | L0 `.ics` 解析、L1 本地规则与正文可点击高亮、L2 内置 AI 提取、阅读区「AI」入口升级、首次引导横幅、顶部合并卡片、统一编辑确认层与本机状态持久化 | **日程**：生成标准 `.ics` (`VEVENT`) 并交由系统默认日历程序打开（打开失败时回退保存 `.ics`）；**待办**：主动作为**复制结构化待办文本**，次动作为导出 `.ics` (`VTODO`)（因 Apple 日历 / 提醒事项与 Outlook 普遍不导入 `VTODO`） |
| **第二阶段（macOS 原生直写）** | `desktop` 在 macOS 上接入 `objc2-event-kit`（前提：已签名且已公证的安装包，避免未签名构建下 TCC 授权不稳定） | 应用内一键静默直写 Apple 日历（macOS 14+ 优先 `requestWriteOnlyAccessToEvents` 只写权限）与 Apple 提醒事项（需完整提醒权限），支持选择日历 / 清单与权限引导，拒绝或失败时自动回退第一阶段路径 |
| **后续可选** | 对话岛工具联动、第三方待办 URL Scheme（如 Things / OmniFocus / TickTick）、`.ics` RSVP 应答、「此刻」页聚合 | 独立评估，不阻塞前两阶段 |

第一阶段不承诺跨设备同步、不修改服务器邮件内容、不引入云端日历 OAuth，也不向邮件正文沙箱 iframe 开放任何脚本或 IPC 权限。

## 2. 三层识别引擎（L0 / L1 / L2）

三层识别按确定性与延迟分层，职责互补、互不阻塞；**草稿与本机已发送副本不运行 L1 / L2 识别**。

```text
打开已缓存正文的收件邮件
  ├── L0：MIME text/calendar / .ics（render() 时一次提取并带版本号保存）──→ 顶部「日程与待办」条
  ├── L1：本地有界规则扫描（body_protocol 请求时，原文视图且在 bind 前运行）→ 正文内淡虚线可点击 <a>
  │        └─ 首次命中且满足 L2 全部可用条件 ──→ 顶部引导横幅（是否自动 AI 识别）
  │        └─ 已开启自动 AI 识别且 L1 命中 ────→ 停留去抖（1s）后异步触发 L2
  └── L2：内置 AI 固定任务（手动触发或 L1 命中自动触发、本机缓存）──────────→ 顶部「日程与待办」条（AI 视觉区分）
```

### 2.1 L0：结构化日程（`core::mail::calendar`）

- **提取时机**：在正文 `render()` 阶段从原始 MIME 中一次性提取 `text/calendar` part 及 `.ics` 附件并随渲染版本号持久化，避免每次打开邮件重复解析原始 MIME。若原始邮件处于 `truncated` 截断状态，不因缺失尾部报错，并在 UI 中标注邀请信息可能不完整。
- **解析字段**：`UID`、`SUMMARY`、`DTSTART`、`DTEND` / `DURATION`、全天标记（`VALUE=DATE`）、`TZID` / UTC 偏移、`RRULE`、`EXDATE`、`RECURRENCE-ID`、`LOCATION`、`DESCRIPTION`、`URL` / 常见会议链接、`METHOD`。
- **兼容与去重**：
  - 同一封邮件常同时包含 `text/calendar` 正文部件和 `.ics` 附件，按 `UID`（及 `RECURRENCE-ID`）去重。
  - 兼容 Outlook 等服务商的 Windows 时区名（如 `China Standard Time`），映射为对应 IANA 时区或规范化为 UTC。
  - `METHOD:CANCEL` 明确展示为“已取消”状态，不提供“添加到日历”动作。
- **有界约束**：单个 `.ics` 部件大小上限 256 KiB、单封最多解析 16 条事件，畸形或超限部件静默跳过，不影响正文渲染。
- **隐私**：完全离线本地处理，不依赖 AI 配置或外发同意。

### 2.2 L1：本地规则识别与正文高亮（`core::mail::action_detect`）

参考[验证码识别](verification-codes.md)的纯函数与有界预算设计，在 `core` 提供宿主无关的规则识别模块，默认开启，可在设置中关闭。

- **在正文渲染流水线中的精确位置**：
  - `desktop/src/body_protocol.rs` 的处理顺序为：`fold_quotes` → `translate::apply` → **L1 正文标注（原文与译文收件视图均支持）** → `bind_resources`。
  - **绝不把 L1 锚点写入 `message_bodies.sanitized_html`**：避免破坏 `translate::apply` 的片段数量一致性检查（`plans.len() != translations.len()` 会导致译文 404），避免规则微调触发全库重渲染，也避免将依赖当前时区的推算结果固化进静态缓存。
  - 跳过 `fold_quotes` 生成的 `<details>` 折叠引用块；译文视图同样绑定 `/action/` 路由并响应点击。
- **扫描目标**：
  1. **日期与时间短语（中英文）**：绝对日期时间（`2026年12月3日 14:30`、`12月3日下午3点`、`2026-12-03 14:00`、`Dec 3, 2026 at 2:30 PM`）、相对日期与时间（`明天上午10点`、`下周三 15:00`、`tomorrow at 2pm`、`next Friday`）、附带明确截止语境的日期（`截止到 12月5日`、`周五前`、`due by Friday`）。
  2. **强行动信号的无日期待办短语（中英文）**：仅匹配具备明确动作对象或强祈使结构的紧凑短语（例如“需签署 / 审批 / 提交 / 付款 / 确认合同”、“Action item: …”、“TODO: …”、“Please sign / submit / approve / review …”）。
- **严格降噪策略**：
  - **跳过非正文节点**：仅遍历安全文本节点，跳过 `<a>`、`<button>`、`<code>`、`<pre>`、`<style>`、`blockquote` 与 `<details>` 引用容器。
  - **排除客套话、验证码与弱匹配**：显式排除“请查收”“请知悉”“请见附件”“麻烦看一下”“Thank you for your reply”“Please let me know if you have any questions”等客套话；排除验证码/有效期语境（如“10 分钟内有效”“expires in 10 minutes”）；排除孤立年份、纯数字编号、订单号、版本号与无日期语境的裸时间点。
  - **预算上限**：单封正文扫描上限 64 KiB，最多标注 15 处，超出截断或跳过。
- **时间基准与时区**：
  - 相对日期推算优先以邮件 `Date` 头及其发件人时区偏移为基准（`Date` 缺失或非法时退回 `receivedAt` + 用户本地时区），随后统一换算为**宿主注入的当前用户本地 IANA 时区**展示。
  - 若推算时间早于当前时刻，在确认弹层中温和提示“该时间已过”。
  - 当系统时区变更时，宿主使 30 分钟有效期内的旧正文文档缓存失效并重新准备。

### 2.3 L2：内置 AI 结构化提取（`core::assistant::action_items`）

复用 `core/src/assistant/email_summary.rs` 的固定任务架构与 `mail_read` 工具层读取边界：

- **可用前提（与 [ai-chat-ui.md](ai-chat-ui.md#52-阅读区的邮件摘要) 5.2 完全对齐）**：必须同时满足 ① 助手已开启、② 已配置可用模型、③ 当前邮件所属账号未被助手排除、④ 邮件正文已缓存在本机。任一不满足时，不展示引导横幅且不触发自动提取。
- **输入**：由 `core` 按 `email_id` 经工具层读取，向模型提供主题、发件人、`Date` 头 / `receivedAt`、用户当前本地 **IANA 时区名**（由宿主传入）及截断后的纯文本正文（标记为不可信数据），不接受前端提交正文。
- **输出与安全校验**：要求模型输出严格 JSON Schema，区分 `event` 与 `task`；Rust 侧用 `serde` + `chrono` 校验时间合法性与字段长度，限制单封最多返回 10 条：
  - **URL 白名单**：AI 给出的 `url` 除须为 `http` / `https` 外，还**必须逐字出现在本封邮件净化后的 `links` 列表中**，否则直接丢弃（防止 Prompt Injection 伪造钓鱼会议链接）。
  - **文本清洗**：标题、地点与备注过滤控制字符及 Unicode Bidi 覆盖字符（如 RLO/LRO，防视觉伪装）。
- **触发、去抖、并发与取消**：
  - **默认手动触发**：在阅读区工具栏「AI → 日程与待办」点击提取；支持运行中「停止」与完成后「重新提取」（传 `fresh: true` 跳过缓存）。
  - **自动触发**：开启 `assistant.actionItems.autoExtract` 后，**仅对 L1 初筛命中的邮件**自动触发 L2（已有有效缓存时不重复请求）。自动触发须经过 **1 秒停留去抖**且全局并发上限为 1，**切换邮件时立即取消**进行中的自动/手动提取，避免 `j`/`k` 快速翻信产生风暴。
  - **与自动摘要协同**：同一封邮件同时开启“自动摘要”与“自动提取日程/待办”时，按顺序先执行摘要再执行提取（或摘要优先占有流光名额），严格满足“同一视图同时最多一个发光元素”。

## 3. 安全与隔离不变量

1. **正文空 sandbox 零降级与协议扩展**：
   - 正文高亮**只能使用 `<a>` 标签**（iframe 无 JS，`<span>` 无法触发导航），由 L1 标注阶段写入受控属性 `data-mailx-action="<index>"`、类名 `mailx-detected-action` 与 `title` 提示。
   - `core/src/mail/render/html.rs` 新增 `ACTION_SLOT` 与 `ResourceBinding::Action { index }`，在 `bind()` 中剥离全部 `data-mailx-*` 属性，并扩展 `valid_resource_url` 对锚点放行 `/action/` 路径；`numbered_links` 模式下不绑定 `href`（高亮不可点击）。
   - `desktop/src/body_protocol.rs` 在 `parse_route`、`navigation()` 与 `resource()` 的 204 兜底分支均新增 `Route::Action { id, index, token }`，拦截跳转并向主窗口发送独立事件 `mailx://action-item-request`（不复用 `LINK_EVENT`）。
   - **可信序号映射与查询**：宿主在服务端按 `Document`（即 `document_token`）持有 L1 `index → DetectedActionItem` 映射；`revoke_all` 或 `retry_images` 更换令牌时旧映射随之作废。点击事件携带 `emailId`、`documentToken`、`index`、`requestId`，前端按当前视图代次核对。宿主另提供与 `list_links` 同级的受控命令（如 `list_document_actions(emailId, documentToken)`）供前端读取 L1 命中数与条目，驱动引导横幅与自动 L2 触发。
   - **邮件自带 CSS 边界**：邮件 `<style>` 理论上可隐藏高亮或将普通链接伪装成虚线，但普通链接仍受 `canonical_link`（仅放行 `http/https/mailto`）约束并走外链流程，邮件无法构造或触发 `/action/` 路由。
2. **防范 Prompt Injection（绝不自动写入系统）**：
   - 任何层级（包括 L0 `.ics`）的条目都**必须经过用户点击并在统一编辑确认弹层中明确确认**后，才由可信 Tauri 命令进入 `core` 对编辑后的 DTO 重新校验并生成 `.ics` / 唤起系统日历；协议 GET 绝不触发写入或外部程序。
3. **RFC 5545 `.ics` 防外发与防注入生成规范**：
   - **固定 `METHOD:PUBLISH`**：即便源头为 L0 `METHOD:REQUEST`，导出给系统日历的 `.ics` 一律固定为 `METHOD:PUBLISH`，并**剥离 `ORGANIZER`、`ATTENDEE` 与 `VALARM`**，避免系统日历自动向外发送 RSVP 邮件。
   - **严格转义与折行**：所有属性名硬编码不取自输入；值中的 `\`、`;`、`,` 与换行严格转义（换行转为字面 `\n`，禁止裸 `\r` / `\n` 注入）；按 RFC 5545 要求在 UTF-8 字符边界按 75 字节折行；`TZID` 等参数值仅允许白名单字符并按规范加引号。
   - **稳定 `UID`**：基于 `email_id` 与条目稳定键派生确定性 `UID`（如 `<sha256_prefix>@mailx.local`），防止用户重复点击添加时在日历中产生重复事件。
4. **受控临时文件生命周期**：
   - 唤起系统日历时，将 `.ics` 写入应用私有临时目录下的随机文件名（不含未清洗的邮件主题），Unix 平台设权限 `0600`。
   - 因系统日历异步读取文件，**不做短时超时删除**，改为保留至**下次应用启动时**统一清理旧临时文件。
   - 若系统打开失败（例如 Linux 未关联 `.ics` 默认应用），自动回退弹出系统“保存 `.ics`”对话框。

## 4. 统一数据模型、融合与持久化

### 4.1 统一候选结构（经 `ts-rs` 导出前端）

```rust
pub enum ActionItemKind { Event, Task }
pub enum ActionItemSource { Ics, Rule, Ai }
pub enum ActionItemStatus { Pending, Added, Dismissed }

pub struct DetectedActionItem {
    pub id: String,                   // 稳定 item_key（见 4.3）
    pub kind: ActionItemKind,         // 确认弹层中可自由切换 Event <-> Task
    pub source: ActionItemSource,
    pub status: ActionItemStatus,
    pub title: String,                // L1 默认取邮件主题或所在短句；L0/L2 取精确标题
    pub start_at: Option<String>,     // ISO 8601（带时区）
    pub end_at: Option<String>,
    pub all_day: bool,
    pub rrule: Option<String>,        // L0 重复规则（受控校验后透传）
    pub cancelled: bool,              // L0 METHOD:CANCEL 标记
    pub location: Option<String>,
    pub url: Option<String>,          // L2 须通过净化后 links 白名单校验
    pub notes: Option<String>,
    pub matched_text: Option<String>, // 原文摘录，便于确认层核对
}
```

### 4.2 L1 与 L2 的融合策略

- **不重载正文 iframe**：L1 在正文准备阶段一次性注入；L2 异步返回后只更新正文上方的「日程与待办」条，绝不重载 iframe。
- **点击 L1 时的智能预填**：若点击原文中的 L1 高亮且本邮件已有有效 L2 缓存，按**解析时间（同一天或相近时段）**匹配对应 L2 条目，自动预填 AI 提炼的更精确标题、地点与时长。

### 4.3 本机 SQLite 存储与稳定 `item_key`

新增编号迁移（具体编号以 `core/src/storage/migrations.rs` 为准），外键均关联 `messages(id) ON DELETE CASCADE`，随邮件与账号清理自动删除，不修改已有表：

- **`message_action_extractions`（L2 结果缓存）**：按 `message_id` 存储 `source_sha256`（主题、发件人、`Date`/`receivedAt`、**用户 IANA 时区名**、正文纯文本哈希）、`prompt_version`、校验后的条目 JSON 与更新时间。`source_sha256` 或 `prompt_version` 不一致即自然失效。扩展现有 `summary_delete_all`（或同步接入设置页“清除 AI 缓存”命令）一并清理本表。
- **`message_action_states`（条目操作状态）**：按 `(message_id, item_key)` 记录 `added` / `dismissed` 与更新时间。
  - **稳定 `item_key` 规则**：L0 使用 `ics:<uid>[:<recurrence_id>]`；L1 使用 `rule:<index>:<time_or_hash>`；L2 使用基于归一化特征的稳定键 `ai:<kind>:<start_date_or_none>:<normalized_title_prefix>`，使“重新提取”后相同日程/待办仍可保留原有 `added` 标记。
  - **重新提取对状态的影响**：手动点击“重新提取”仅清除该邮件 `ai:*` 的 `dismissed` 记录（让用户能重新看到新结果），**不清除 `added` 标记**。

## 5. 界面与交互设计

遵循[界面设计语言](ui-design.md)与[AI 交互界面](ai-chat-ui.md)第 4.6 节的视觉规则。

### 5.1 阅读区工具栏：「AI 摘要」升格为「AI」

- 原 `SummaryToolbarButton` 升格为统一的阅读区 **「AI」** 按钮（保持 `AiMark` 星芒图标；摘要或提取任一任务运行中显示加载/流光态）。
- 点开 Popover 顶部提供轻量分段 Tab：**「摘要」** 与 **「日程与待办」**：
  - **摘要**：完整保留现有篇幅、语言、按发件人自动摘要和生成按钮；
  - **日程与待办**：展示当前提取状态、主按钮「提取日程与待办」（运行中为「停止」，已有结果为「重新提取」），以及「疑似含日程/待办时自动用 AI 识别」开关。

### 5.2 首次 L1 命中顶部引导横幅

- **出现条件**：浏览一封 L1 有命中的原文邮件，且同时满足：① 满足 2.3 节全部 L2 可用条件（助手开启、有可用模型、账号未被排除、正文已缓存）；② `assistant.actionItems.autoExtract` 为 `false`；③ `assistant.actionItems.promptDismissed` 为 `false`。
- **样式与动作**：沿用远程图片 / 翻译提示条的单行浅底样式（`bg-muted rounded-md h-9 text-xs`），左侧为静态 `AiMark` 与说明文案，右侧提供 **「启用」**（保存 `autoExtract = true` 并对当前邮件立即触发一次 L2，按需走外发同意）、**「不再提示」**（保存 `promptDismissed = true`）与 **「×」**（本次会话收起）。

### 5.3 正文上方合并「日程与待办」条（L0 与 L2 的视觉区分）

L0（`.ics`）与 L2（AI 提取）统一收口在阅读区正文上方同一个紧凑区域（独立于摘要卡），避免顶部堆叠多个卡片：

- **折叠策略**：单条直接一行展示（来源/类型图标 + 时间 + 标题 + “添加”与“忽略”轻量动作；已添加条目带淡标仍可重开）；多条默认展示前 1–2 条，行尾提供“展开全部（N 项）”。
- **L0 与 L2 的视觉区分（严格遵循 [ui-design.md](ui-design.md) 与 [ai-chat-ui.md](ai-chat-ui.md#46-ai-视觉标识霓虹渐变与发光)）**：
  - **L0（`.ics` 邀请）**：采用中性表面（`bg-muted/60`）与普通日历图标，不带 AI 视觉特效；`METHOD:CANCEL` 显示淡化删除线与“已取消”标签。
  - **L2（AI 提取的日程 / 待办）**：
    - **识别进行中**：容器外缘显示 1px–1.5px 的 AI 霓虹渐变描边缓慢流动并附极淡柔光（不越出 8px），状态文字“正在识别日程与待办…”带 AI 渐变光泽；遵守“同一视图同时最多一个发光元素”原则，且在 `prefers-reduced-motion` 下退化为静态淡显。
    - **识别完成后**：流光与柔光在约 300ms 内平滑淡出，不常亮，且**不保留卡片边框或静态描边**（符合岛内无边框约定），仅在条目行首保留小型静态 `AiMark` 星芒图标与 L0 区分；标题与时间始终使用常规前景色 token 保证可读性。
  - **L0 与 L2 同时存在时**：置于同一区域内，L0 置顶、L2 带 `AiMark` 紧随其后，不画通栏分割线。

### 5.4 正文内 L1 高亮样式与暗黑模式兼容

- 通过 `mail_body_css` 注入，跟随用户强调色（默认 `#2563eb`），与 `DARK_CSS` 的 `invert(1) hue-rotate(180deg)` 保持色相一致：
  - 默认：1.5px 强调色虚线下划线 + `color-mix(in srgb, <accent> 7%, transparent)` 极淡底色；
  - 悬停 / 键盘聚焦：切换为实线下划线并加深底色至 15%，光标呈手形。
- 锚点附带 `title` 与清晰的读屏标签，支持键盘 `Tab` 聚焦；在 `MailTheme::Follow`（浅/深）与 `MailTheme::Light` 下均保持清晰可读。

### 5.5 统一编辑确认弹层

L0、正文内 L1 高亮、L2 条目点击后**全部**进入同一个轻量编辑确认弹层（固定在阅读区可控位置，不依赖 iframe 坐标）：

- **类型切换**：顶部「日程」 / 「待办」分段切换。
- **可编辑字段**：标题、开始 / 截止时间、结束时间与全天开关（日程模式）、地点（日程模式）、备注（默认附“来自邮件：`<主题>`（`<发件人>`）”，可编辑），以及只读展示的“原文片段”。
- **底部动作（第一阶段）**：
  - 日程模式：主按钮 **「添加到系统日历」**（生成 `.ics` 并唤起系统默认日历，失败回退保存），次按钮「保存 .ics」；
  - 待办模式：主按钮 **「复制待办文本」**，次按钮「导出 .ics 待办」。
- **反馈**：成功后关闭弹层、持久化 `added` 状态并轻 Toast 提示；失败在弹层内通过 `ErrorNotice` 就近提示。

## 6. 设置项扩展

在 `core/src/settings.rs` 新增向后兼容字段（均带 `#[serde(default)]`，缺省与旧 `settings.json` 自动回填）：

| 结构与路径 | 类型与默认值 | 说明 |
| --- | --- | --- |
| `MailSettings.detect_action_items`（`mail.detectActionItems`） | `bool`，默认 `true` | 控制阅读区是否启用 L1 本地规则识别与正文虚线高亮（位于“设置 → 邮件 → 增强”） |
| `AssistantSettings.action_items.auto_extract`（`assistant.actionItems.autoExtract`） | `bool`，默认 `false` | 开启后对 L1 初筛命中的邮件自动运行 L2 提取（新增 `ActionItemSettings`，结构对齐 `EmailSummarySettings`） |
| `AssistantSettings.action_items.prompt_dismissed`（`assistant.actionItems.promptDismissed`） | `bool`，默认 `false` | 首次 L1 命中顶部引导横幅的“不再提示”持久化标记 |

前端修改 `assistant.actionItems` 字段时，沿用 `summary.collapsed` 的写法，以完整 `assistant` 段提交 `SettingsPatch.assistant`，避免覆盖同级配置。

## 7. 验证方式

具体执行结果与环境记录在 PR 描述与 CI 运行记录中，本节定义验证范围与通过标准：

1. **L0 `.ics` 解析与生成（`core` 单元测试）**：覆盖会议邀请、全天事件、重复规则（`RRULE`/`EXDATE`/`RECURRENCE-ID`）、Windows 时区映射、`UID` 去重、`METHOD:CANCEL`、超限/畸形拒绝；验证生成 `.ics` 强制 `METHOD:PUBLISH`、剥离 `ORGANIZER`/`ATTENDEE`/`VALARM`、75 字节 UTF-8 安全折行、CRLF 注入防线及稳定 `UID`。
2. **L1 本地规则与降噪（`core` 单元测试 + 合成样本）**：覆盖中英文绝对/相对日期（验证基于 `Date` 发件人时区推算及缺失回退）、强行动信号待办短语、客套话与验证码“10 分钟内有效”排除、跳过链接/代码/引用块/`<details>`、预算上限，以及草稿与已发送不运行。
3. **HTML 标注与协议隔离（`core` + `desktop` 测试）**：验证 L1 在 `translate::apply` 之后、`bind_resources` 之前执行且不影响译文片段计数；验证 `data-mailx-action` 仅注入安全文本节点、邮件原文无法伪造 `/action/` 路由（`canonical_link` 拦截）；验证 `Route::Action` 不跳页、切换代次与 `revoke_all` / `retry_images` 换令牌后旧映射失效，以及 `numbered_links` 模式下高亮不可点。
4. **L2 AI 提取与持久化（`core` 单元测试）**：验证全部 4 项可用条件、外发同意门禁、AI 返回 URL 必须在邮件净化后 `links` 白名单内（否则丢弃）、控制/Bidi 字符清洗、切换取消与停留去抖、IANA 时区哈希失效、重新提取保留 `added` 且清除 `dismissed`、删邮件级联清理与清除 AI 缓存同步清空。
5. **前端交互与视觉（Vitest + 浏览器预览）**：验证工具栏「AI」Popover 分段切换、引导横幅出现条件、L0/L2 合并条折叠与 AI 流光/300ms 淡出/无边框约束、统一编辑确认弹层及日程/待办互转、键盘 `Tab` 聚焦与读屏名称。
6. **三平台实机验收（第一阶段收口）**：在 macOS / Windows / Linux 实机验证浅/深主题（`MailTheme::Follow` 与 `Light`）下的正文高亮显示与点击（不跳页、不丢滚动位置）、`.ics` 唤起系统默认日历及打开失败回退保存，以及待办复制与 `.ics (VTODO)` 导出表现；第二阶段另行补充已签名公证 macOS 包的 EventKit 权限与写入验收。
