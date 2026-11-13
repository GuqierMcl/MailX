# Changelog

This file records user-facing changes to MailX, in English and Chinese on the same line. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

- For a user-facing change, add one line under `## [Unreleased]` in the PR, in one of the five categories below (leave out a category with nothing in it). Every entry starts with `- `. Keep the category headings exactly as written, emoji included: `### 🚀 Feature`, `### ✨ Optimization`, `### 🩹 Fixed`, `### 🔐 Security`, `### 🔄 Changed`.
- Write entries for users, one sentence per line. Do not list tests, CI, refactors or documentation. End with the PR link, for example `- 修复远程图片加载失败后无法重试 ([#25](https://github.com/GuqierMcl/MailX/pull/25))`.
- Write every entry in both languages on one line, separated by ` / `: English first, then Chinese, then the link or reference. Match the wording the app itself uses. Keep the two halves saying the same thing.
- `Changed` holds only incompatible behaviour, data formats, configuration changes and removed features (including data migrations). Improvements that keep behaviour the same go under `Optimization`.
- Prereleases (alpha / beta / rc) never get a section of their own; their release notes come from `[Unreleased]`. A stable release folds all of it into one dated section via `bun run version:bump`.
- Release steps are in [docs/development/release.md](docs/development/release.md).

## [Unreleased]

### 🚀 Feature

- AI settings: connect model providers (models.dev catalog, OpenAI-compatible and Anthropic-compatible APIs) and choose a default model. / AI 设置：连接模型服务商（models.dev 目录，兼容 OpenAI 与 Anthropic 接口）并选择默认模型。 ([#126](https://github.com/GuqierMcl/MailX/pull/126))
- Translation can use an AI model from AI settings, with its own model choice. / 翻译可以使用 AI 设置里的模型，并可单独选择模型。 ([#127](https://github.com/GuqierMcl/MailX/pull/127))

### ✨ Optimization

- The message list is tidier: a narrow strip holds only the unread dot, and star, read, archive and delete now appear together in the upper right on hover, for every preview setting. / 邮件列表更紧凑：左侧窄条只放未读蓝点，星标、已读、归档和删除在悬停时统一出现在右上角，对所有预览设置都一样。 ([#128](https://github.com/GuqierMcl/MailX/pull/128))
- New appearance setting to show a message's mailbox as a name or a small icon on the sender's avatar. / 新增外观设置：可把邮件所属邮箱显示为名称，或显示为发件人头像上的小图标。 ([#128](https://github.com/GuqierMcl/MailX/pull/128))

### 🩹 Fixed

- On macOS 26 and later, the window's close, minimize and zoom buttons now use the current system style and sit in the right place. / 在 macOS 26 及更新版本上，窗口的关闭、最小化和缩放按钮改用当前系统样式，并回到正确位置。 ([#125](https://github.com/GuqierMcl/MailX/pull/125))
- In the desktop app, an account dragged in Email accounts stays under the pointer instead of jumping to the window's top-left corner. / 在桌面应用的邮箱账号中拖动账号时，账号会跟随指针，不再跳到窗口左上角。

## [0.2.1] - 2026-11-12

### 🚀 Feature

- Translate the message you are reading in place, keeping its layout, with no setup (Microsoft Translate by default and Google Translate, which use the channels browsers' built-in translators use and are not official services for other apps, so they may stop working) or with a DeepL, Microsoft Translator or Google Cloud Translation key of your own; nothing is sent until you agree and translate. / 在阅读区原位翻译当前邮件并保留版式：无需设置（默认 Microsoft 翻译，另有 Google 翻译，借用浏览器内置翻译的通道，并非正式开放给其他应用的服务，可能失效），也可用你自己的 DeepL、Microsoft Translator 或 Google Cloud Translation 密钥；你同意并点击翻译之前不会发送任何内容。 ([#118](https://github.com/GuqierMcl/MailX/pull/118))

### ✨ Optimization

- In the Unread view, a message you open stays in the list after it is marked read, until you switch mailbox, filter or search. / 在“未读”视图中打开的邮件标为已读后仍留在列表中，直到切换邮箱、筛选或搜索。 ([#122](https://github.com/GuqierMcl/MailX/pull/122))
- Account names in the sidebar get more room: the unread count, refresh button and refresh spinner share one spot at the end of the row again. / 侧栏账号名称可显示得更完整：未读数、刷新按钮和刷新进度重新共用行尾的同一位置。 ([#122](https://github.com/GuqierMcl/MailX/pull/122))

### 🔄 Changed

- The local database upgrades to v29 to keep saved translations; older versions cannot open the upgraded database. / 本地数据库升级到 v29 以保存译文，旧版本无法打开升级后的数据库。 ([#118](https://github.com/GuqierMcl/MailX/pull/118))

## [0.2.0] - 2026-11-12

### 🚀 Feature

- Quoted earlier messages at the end of a reply are folded behind Show quoted text when MailX recognizes the quoting style; anything else is shown in full as before. / 回复末尾引用的原邮件在 MailX 能识别其引用格式时会收进“显示引用内容”，其余情况仍完整显示。 ([#114](https://github.com/GuqierMcl/MailX/pull/114))
- Messages that are part of a back-and-forth open as a conversation, with your replies included and a switch back to single messages. / 属于多次往来的邮件会以对话形式打开，包含你自己的回复，并可切换回逐封阅读。 ([#113](https://github.com/GuqierMcl/MailX/pull/113))
- Right-click a message and choose Show mail from this contact to list that contact's mail, then go back with the back button, Esc or the mouse's back button. / 右键邮件并选择“查看此联系人的邮件”可列出该联系人的邮件，再用返回按钮、Esc 或鼠标后退键返回。 ([#111](https://github.com/GuqierMcl/MailX/pull/111))
- Group a contact's mail into a single row in your inboxes from the right-click menu, and manage grouped contacts under Settings → Contacts. / 可通过右键菜单将联系人的邮件在收件箱中聚合为一行，并在“设置 → 联系人”中管理已聚合的联系人。 ([#111](https://github.com/GuqierMcl/MailX/pull/111))
- Add Outlook.com, Hotmail, Live and Microsoft 365 mailboxes by signing in in your browser, without a password. / 通过浏览器登录即可添加 Outlook.com、Hotmail、Live 和 Microsoft 365 邮箱，无需密码。 ([#107](https://github.com/GuqierMcl/MailX/pull/107))
- Right-clicking a message in the list opens a menu with open, reply, reply all, forward, read and star, archive and delete actions, without opening the message. / 在邮件列表中右键点击邮件可打开菜单，提供打开、回复、全部回复、转发、已读与星标、归档和删除操作，且不会打开该邮件。 ([#105](https://github.com/GuqierMcl/MailX/pull/105))

### ✨ Optimization

- The 139 Mail provider now uses the clean official blue mail icon without the promotional “AI+” badge. / 139 邮箱服务商现使用不含“AI+”推广信息的官方蓝色邮件图标。
- The About page now shows the MailX logo and version, with GitHub, problem-report and feature-request templates, and release history together below the update controls. / “关于”页现展示 MailX Logo 与版本，并在更新设置下方集中提供 GitHub、问题反馈和功能建议模板及发布记录入口。
- Manual retry starts a fresh delivery retry window, and waiting for mailbox authorization preserves queued messages without spending delivery attempts. / 手动立即重试会重新开始投递重试窗口，等待邮箱授权时保留排队邮件且不消耗投递次数。

### 🔄 Changed

- The local database upgrades from v24 to v28 for browser sign-in, grouped contacts and conversations; after the upgrade MailX fetches reply information for cached mail and groups it into conversations in the background, and older versions cannot open the upgraded database, so keep a full backup from before the upgrade if you may downgrade. / 本地数据库从 v24 升级到 v28，以支持浏览器登录、联系人聚合和对话；升级后 MailX 会在后台为已缓存的邮件补取回复信息并整理成对话，旧版无法直接打开升级后的数据库，降级前需保留升级前的完整数据备份。
- Update checks now stay on, with default-enabled Automatic updates downloading in the background and prompting for a restart or installing on quit; turning it off keeps manual downloads, and previous disabled preferences are preserved. / 更新检查现固定开启，默认打开的“自动更新”会在后台下载并提醒重启或在退出时安装；关闭后保留手动下载，升级时保留原有关闭偏好。
- Account summaries now distinguish password and OAuth credentials while preserving existing password accounts. / 账号摘要现明确区分密码与 OAuth 凭证，并兼容已有的密码账号。

## [0.1.2] - 2026-11-08

### 🚀 Feature

- A Gmail entry, listed first with iCloud second, fills in encrypted IMAP / SMTP settings for @gmail.com and @googlemail.com, with the official bundled Gmail icon and step-by-step app password guidance. / 新增 Gmail 添加入口并排在服务商列表首位、iCloud 第二，为 @gmail.com 和 @googlemail.com 预填加密收发服务器，采用本地打包的官方 Gmail 图标，并提供应用专用密码的分步指引。 ([#100](https://github.com/GuqierMcl/MailX/pull/100))
- Select several messages from the message list to mark them read or unread, star, archive or delete them together, with undo for delete and archive and failed messages kept selected. / 邮件列表支持多选，可一次标为已读或未读、加星标、归档或删除，删除与归档可撤销，没能处理的邮件保持选中。 ([#101](https://github.com/GuqierMcl/MailX/pull/101))

### ✨ Optimization

- Draft previews show saved rich-text formatting and inline images, follow the message theme and update after saving, with file attachments listed separately. / 草稿预览保留已保存的富文本格式和内嵌图片，跟随正文主题并在保存后更新，普通文件附件单独列出。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))
- A refresh button beside the title-bar sidebar toggle refreshes all accounts, with progress shown only for a manual refresh while automatic sync stays quiet. / 标题栏侧栏折叠按钮右侧新增刷新按钮，可刷新全部邮箱；仅手动刷新显示进度，自动同步保持安静。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))
- Message cards show star, read and archive controls in a left column when previews are enabled, with deletion at the upper right, compact controls when previews are off and a subtle hover or focus fade. / 邮件卡片开启正文预览时，星标、已读与归档操作在左侧竖排，删除保留在右上角；关闭预览时保留紧凑布局，悬停或聚焦时以轻微淡入淡出显示操作。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))
- Message cards bring the read-status dot closer to the avatar, and the reading toolbar uses the same empty ring for read messages and blue-dot ring for unread messages. / 邮件卡片收紧状态点与头像的间距，阅读区工具栏使用相同的空环表示已读、蓝点加环表示未读。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))
- The open message, draft or outbox item keeps a clearer selection background in blue or a custom accent across light and dark themes, falling back to blue for colors that make selection hard to distinguish. / 当前打开的邮件、草稿或发件箱项目持续显示更清晰的蓝色或自定义强调色淡底，适配浅深主题，难以区分选中状态的颜色会回退为蓝色。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))

### 🩹 Fixed

- Message-card quick actions fade out after a mouse click when the pointer leaves, while keyboard focus keeps them available. / 修复鼠标点击已读等快捷操作后按钮不再隐藏的问题，移开鼠标即可淡出，键盘聚焦时仍保持可用。 ([#98](https://github.com/GuqierMcl/MailX/pull/98))

## [0.1.1] - 2026-11-06

### 🚀 Feature

- An iCloud Mail entry fills in encrypted IMAP / SMTP settings for @icloud.com, @me.com and @mac.com, with the official bundled Mail icon, app-specific password guidance and separate username settings. / 新增 iCloud 邮箱添加入口，为 @icloud.com、@me.com 和 @mac.com 预填加密收发服务器，采用本地打包的官方 Mail 图标，提供应用专用密码指引与收发独立用户名设置。

### ✨ Optimization

- The macOS MailX menu bar icon is now enabled by default, and iCloud is the first provider in the account setup list. / macOS 默认开启 MailX 菜单栏图标，iCloud 调整为添加账号服务商列表的第一项。
- Collapsing the sidebar now leaves a column of icons with unread counts, tooltips, and menus for More and each account, and the choice is remembered; a narrow window uses the icon column on its own. / 收起侧栏后保留一列图标，带未读数、悬停提示，以及“更多”和各账号的弹出菜单，选择会被记住；窗口较窄时自动使用图标栏。 ([#95](https://github.com/GuqierMcl/MailX/pull/95))
- The sidebar now groups Inbox, Starred, Drafts and Sent across all accounts at the top, with Archive, Junk and Trash under More and each account listed below a row on demand; accounts below start collapsed and still hold all their folders. / 侧栏顶部按类型汇总所有账号的收件箱、星标、草稿和已发送，归档、垃圾邮件和已删除收在“更多”里，并可按账号展开；下方各账号默认折叠，展开后仍是完整的文件夹。 ([#94](https://github.com/GuqierMcl/MailX/pull/94))
- Automatic sync keeps sidebar account rows quiet and unread counts visible; manual refresh shows progress only on the refresh button you use, newly added accounts get a subdued first-fetch hint, and sync problems keep static warnings. / 自动同步时侧栏账号行保持安静并保留未读数；手动刷新只在使用的刷新按钮上显示进度，新接入账号首次获取内容时显示弱化提示，同步问题继续保留静态警示。

### 🩹 Fixed

- Fixed duplicate local and server copies in Sent when QQ Mail rewrites message identifiers or the server copy syncs before sending finishes; existing duplicates are reconciled during sync without sending again. / 修复 QQ 邮箱重写邮件标识或服务器副本先于发送完成同步时，“已发送”中同时出现本机与服务器副本的问题；已有重复项会在同步时重新对账，无需再次发送。

### 🔄 Changed

- The local database upgrades to v24 to track sent-copy identities; older versions cannot open the upgraded database. / 本地数据库升级至 v24，以记录已发送副本的身份；旧版无法直接打开升级后的数据库。

## [0.1.0] - 2026-11-04

### 🚀 Feature

- Settings have a new, off-by-default launch at login and start silently option; start silently only applies when MailX launches at login, and the main window shows when no background entry is available. / 设置中新增默认关闭的开机自启动与静默启动；静默启动仅在登录时自动启动生效，后台入口不可用时显示主窗口。 ([#87](https://github.com/GuqierMcl/MailX/pull/87))
- Local MCP: the AI assistant can send drafts, query, retry and cancel queued sends, and add or remove local file attachments without confirming each message. / 本地 MCP：AI 助手可直接发送草稿、查询 / 重试 / 取消待发送任务，并添加或移除本机文件附件，无需逐封确认。 ([#86](https://github.com/GuqierMcl/MailX/pull/86))
- Local MCP: the AI assistant can set read and starred in bulk, archive or unarchive locally, request a sync, and open a draft for you to check; search accepts starred and local-archive filters, the UI refreshes, and the archive notice can be undone. The existing MCP master switch grants all of this. / 本地 MCP：AI 助手可批量设置已读和星标、本机归档或取消归档、请求刷新邮件，并打开草稿供你检查；搜索支持星标与本机归档筛选，界面同步刷新，归档提示可撤销。现有 MCP 总开关统一开放这些能力。 ([#85](https://github.com/GuqierMcl/MailX/pull/85))
- The MCP settings page moves the program path and connection-info file path into a collapsed Manual setup section, shows paths under your home directory with a `~` prefix, and offers a button to copy the full path; Copy client configuration still copies full paths. / MCP 设置页的程序路径与连接信息文件路径放入默认折叠的“手动配置”，位于主目录内时显示为 `~` 开头的短路径，并提供复制完整路径的按钮；“复制客户端配置”仍复制完整路径。 ([#83](https://github.com/GuqierMcl/MailX/pull/83))
- Local MCP: the AI assistant can now list, read, edit and delete your drafts, including ones you wrote yourself; edits and deletions check the draft version and are refused while you are editing that draft in a compose window. It can also bring MailX to the front and open a message when you ask. / 本地 MCP：AI 助手现在可以列出、阅读、修改和删除你的草稿（包括你自己写的；修改和删除需核对版本，你正在写信窗口中编辑的草稿会被拒绝），并可在你要求时让 MailX 回到前台并打开某封邮件。 ([#81](https://github.com/GuqierMcl/MailX/pull/81))
- Local MCP: when you explicitly ask, the AI assistant can download a message body that is not saved locally yet (without marking it read), create reply, reply-all and forward drafts (drafts only), list folders and search within a folder such as Trash. / 本地 MCP：AI 助手现在可以在你明确要求时下载尚未保存到本机的邮件正文（不会把邮件标为已读）、为邮件创建回复 / 全部回复 / 转发草稿（只创建草稿），并列出文件夹、按文件夹搜索邮件（如“已删除”）。 ([#80](https://github.com/GuqierMcl/MailX/pull/80))
- A new local MCP server, off by default: after you enable it in Settings → MCP and confirm, programs on this computer such as AI assistants can read local mail from all accounts and work with drafts through the bundled `mailx-mcp`. The settings page shows the running state and connected clients and copies the client configuration in one click, and a status icon appears in the title bar while it runs. Tested with two clients on macOS; other platforms and clients are unverified. / 新增本地 MCP 服务器（默认关闭）：在“设置 → MCP”开启并确认后，本机的 AI 助手等程序可通过随应用附带的 `mailx-mcp` 读取所有账号的本地邮件并处理草稿；设置页显示运行状态、当前已连接的客户端，并可一键复制客户端配置，开启后标题栏出现状态图标。已在 macOS 上与两个客户端联调，其他平台与客户端未验证。 ([#78](https://github.com/GuqierMcl/MailX/pull/78))
- Settings navigation is grouped into App, Mail and Integrations, with About pinned at the bottom. / 设置导航按“应用 / 邮件 / 集成”分组，“关于”固定在底部。 ([#78](https://github.com/GuqierMcl/MailX/pull/78))
- The account list has a drag handle on the left, so you can reorder accounts with a mouse, touch or keyboard; the order is stored locally and applied to the sidebar, the composer and related settings lists. / 账号管理列表左侧新增拖动手柄，可用鼠标、触控或键盘调整账号显示顺序；顺序保存在本机，并同步到侧栏、写信和相关设置列表。 ([#72](https://github.com/GuqierMcl/MailX/pull/72))
- You can archive while reading: the message leaves the inbox on this computer only and the server is not touched. Find it under All archived in the sidebar or Archived under an account, move it back any time, or undo from the notice for a few seconds. Archived mail no longer counts towards the inbox unread count. / 阅读邮件时可以归档：邮件只在本机移出收件箱，不改动服务器；在侧栏“全部已归档”或账号下的“已归档”里查看，可随时移回收件箱，归档后几秒内也可在提示中撤销。收件箱未读数不再计入已归档的邮件。 ([#71](https://github.com/GuqierMcl/MailX/pull/71))
- New mail reaches the inbox almost instantly: mailboxes that support IMAP IDLE keep one long-lived connection and sync and notify as soon as mail arrives; mailboxes that do not support it, or where the provider refuses, keep refreshing at the previous interval. / 收件箱新邮件几乎即时到达：支持 IMAP IDLE 的邮箱保持一条常驻连接，有新邮件时立即同步并通知；不支持或被服务商拒绝时仍按原来的间隔刷新。 ([#70](https://github.com/GuqierMcl/MailX/pull/70))
- You can delete while reading: the message moves to Trash, and is permanently deleted after a confirmation when it is already in Trash or the account has no such folder. The delete can be undone from the notice for a few seconds. When the server does not support safe deletion, the message reappears with the reason. / 阅读邮件时可以删除：邮件会移到“已删除”，已在“已删除”中或账号没有该文件夹时会先确认再永久删除；删除后几秒内可在提示中撤销。服务器不支持安全删除时邮件会恢复显示并提示原因。 ([#69](https://github.com/GuqierMcl/MailX/pull/69))
- Search now covers message bodies and attachment names, not just the subject and sender, and highlights the matching text in results. Search runs locally, and bodies you have opened are added to the local index. / 搜索现在会查找邮件正文和附件名，不只是主题和发件人；结果里会高亮命中的那段文字。搜索在本机完成，已打开过的邮件正文会被收进本地索引。 ([#67](https://github.com/GuqierMcl/MailX/pull/67))
- Mail that was just sent and has no copy on the server yet also appears in Sent, where it can be read, replied to, forwarded or removed locally; the Sent list now shows recipients. / 刚发出、服务器上还没有副本的邮件也会出现在“已发送”里，可以阅读、回复、转发或从本机移除；已发送列表改为显示收件人。 ([#65](https://github.com/GuqierMcl/MailX/pull/65))
- You can insert pictures into a message body: pick one from the toolbar, or paste a screenshot. Pictures travel inside the message and nothing is loaded from outside when the recipient opens it. / 写信时可以在正文中插入图片：点工具栏按钮选择图片，或直接粘贴截图；图片随邮件一起发出，收件人打开时不会向外部加载任何内容。 ([#62](https://github.com/GuqierMcl/MailX/pull/62))
- Message list previews load automatically as rows scroll near the visible area, showing (Loading…) meanwhile; previews are fetched in part and do not mark mail as read. / 邮件列表的预览随行滚动到可视区域附近时自动加载，加载期间显示（加载中）；预览使用部分获取，不会把邮件标为已读。 ([#59](https://github.com/GuqierMcl/MailX/pull/59))
- A 139 Mail entry with the provider logo, encrypted server settings filled in, and guidance on enabling an authorization code, its 90-day validity and account eligibility. / 新增 139 邮箱添加入口，显示服务商图标、预填加密收发服务器，并提供授权码开通、90 天有效期与账号资格说明。 ([#55](https://github.com/GuqierMcl/MailX/pull/55))
- The compose window adds and removes attachments with the paperclip or by dropping files onto it; the attachment bar shows names and sizes and warns when the total size or count limit is exceeded, a draft already in the send queue is read-only, forwarding carries the original attachments, and an oversized message rejected by the server gets a clear message. / 写信窗口可添加和移除附件：用回形针按钮选择文件，或把文件拖入窗口；附件栏显示文件名与大小，超出总大小或数量上限时给出提示，已进入发送队列的草稿只读；转发邮件时自动带上原邮件的附件；服务器因邮件过大拒收时给出明确提示。 ([#49](https://github.com/GuqierMcl/MailX/pull/49))
- Closing the main window can run in the background or quit, asks each time by default, and remembers the choice you can change in General settings; supported by the Windows and Linux tray and an optional macOS menu bar item. / 关闭主窗口时可选择后台运行或退出，默认每次询问，可记住选择并在通用设置修改；支持 Windows/Linux 系统托盘与可选的 macOS 菜单栏入口。 ([#48](https://github.com/GuqierMcl/MailX/pull/48))
- When adding an account you can test the incoming and outgoing sign-in before saving, cancel, change settings and test again, or skip the test and continue. / 添加邮箱时可在保存前测试当前收信与发信登录，支持取消、修改后重测，也可跳过测试继续添加。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- NetEase 163 / 126 / yeah.net entries with their provider logos, encrypted server settings filled in, and guidance on client authorization codes. / 新增网易 163 / 126 / yeah.net 邮箱添加入口，显示各自的服务商图标，预填加密收发服务器并提供客户端授权码指引。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- The macOS main and compose windows use system vibrancy in their sidebar and title bar, following the app theme. / macOS 主窗口与写信窗口的侧栏与标题栏使用系统毛玻璃材质（跟随应用主题）。 ([#43](https://github.com/GuqierMcl/MailX/pull/43))
- Mailbox list content that overflows fades out at the top and bottom edges to show there is more. / 邮箱列表内容超出时，上下边缘渐隐，提示还有更多内容。 ([#43](https://github.com/GuqierMcl/MailX/pull/43))
- Accounts can be removed from this computer, clearing their local mail, drafts and send records; in-flight send protection, credential cleanup retries and compose-window coordination prevent accidental sending or restoring a removed account. / 账号管理支持移除本机账号，清理本地邮件、草稿和发件记录；发送中保护、凭证清理重试和写信窗口联动避免误发或恢复已移除账号。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- A QQ Mail / Foxmail entry covering @qq.com, @vip.qq.com and @foxmail.com, with the QQ Mail logo, server settings filled in and authorization-code guidance; account settings verify incoming and outgoing sign-in separately. / QQ 邮箱 / Foxmail 添加入口：支持 @qq.com、@vip.qq.com 和 @foxmail.com，显示 QQ 邮箱图标，预填收发服务器并提供授权码指引；账号设置支持分别验证收信与发信登录。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- A local-first desktop mail workspace preview: signs in to IMAP through the account's routing, syncs folders and message headers, and offers a unified inbox with local header search and paging. / 本地优先的桌面邮件工作台预览：经账号路由登录 IMAP，同步文件夹与邮件头，提供统一收件箱、本地头部搜索与分页。
- Message bodies download on demand and are cached and shown in an isolated reading view; read and starred states write back, attachments save, and new mail raises a notification. / 按需下载并缓存邮件正文，在隔离的阅读视图中显示；支持已读 / 星标写回、附件保存与新邮件通知。
- Rich-text composing (Tiptap, local drafts, replies and forwards, a separate compose window), sending over SMTP through the send queue. / 富文本写信（Tiptap、本地草稿、回复与转发、独立写信窗口），经发送队列通过 SMTP 发信。
- English and Chinese UI with a language setting that can follow the system; the main and compose windows switch together, and new notifications and quoted text in new replies and forwards use the chosen language. / 中英文界面与语言设置：支持跟随系统，主窗口与写信窗口同步切换，新通知及新建回复 / 转发的引用使用所选语言。 ([#29](https://github.com/GuqierMcl/MailX/pull/29))
- In-app updates: checked about 30 seconds after launch and every 6 hours afterwards, and automatic checks can be turned off in settings. When a new version exists a title-bar notice appears, and the dialog shows the release notes with options to download and restart now or install on next quit. Links always open in the system browser. / 应用内更新：启动约 30 秒后和之后每 6 小时检查一次新版本（可在设置中关闭自动检查）；发现新版本时标题栏出现提示，弹窗里可以阅读更新说明、下载并立即重启安装，或选择下次退出时安装。链接一律用系统浏览器打开。

### ✨ Optimization

- MCP draft and send results return the sending address directly, and account lists mark the default sending account, so the AI assistant chooses and describes the right mailbox. / MCP 草稿和发送结果直接返回发件邮箱地址，账号列表标明默认发件账号，方便 AI 助手准确选择和说明使用的邮箱。 ([#86](https://github.com/GuqierMcl/MailX/pull/86))
- The MCP settings page is trimmed to the switch, connection state and client configuration; the About page drops the static privacy text and keeps version and update controls. / 精简 MCP 设置页，只保留开关、连接状态与客户端配置；关于页移除静态隐私说明，保留版本和更新功能。 ([#85](https://github.com/GuqierMcl/MailX/pull/85))
- Accounts added through the QQ, NetEase and 139 entries no longer get an identity colour by default; manual IMAP/SMTP accounts still get one automatically, and every entry lets you pick a colour. / 通过 QQ、网易和 139 等服务商入口添加邮箱时默认不设置标识颜色，手动 IMAP/SMTP 仍自动分配颜色，各入口均可自行选色。 ([#66](https://github.com/GuqierMcl/MailX/pull/66))
- Provider icons now use the official original artwork, keeping the full mark and its colours, and QQ Mail uses its mailbox-specific logo. / 邮箱服务商图标改用官方原始素材，保留完整图形与原有配色，QQ 邮箱使用专属邮箱图标。 ([#64](https://github.com/GuqierMcl/MailX/pull/64))
- Avatars use the local cache first after a restart instead of looking online first, and senders confirmed to have no avatar are not looked up again for 24 hours. / 重启后头像优先使用本地缓存，不再先联网查询；确认没有头像的发件人 24 小时内不再重复查找。 ([#60](https://github.com/GuqierMcl/MailX/pull/60))
- The message list uses virtual scrolling and scrolls more smoothly with many messages; reaching the end of the list loads more and Load more still works after a failure. The preview area reserves a fixed height from the message preview lines setting, so every row has the same height and the list no longer jumps while scrolling. / 邮件列表改用虚拟滚动，邮件很多时滚动更流畅；滚动到接近列表末尾时自动加载更多，加载失败仍可点击“加载更多”重试；列表按“预览行数”设置为预览区预留固定高度，每行等高，滚动时不再跳动。 ([#58](https://github.com/GuqierMcl/MailX/pull/58))
- The settings dialog shows MailX and the current version in its bottom-left corner, visible on every settings page. / 设置弹窗左下角显示 MailX 与当前版本号，切换设置页时始终可见。 ([#47](https://github.com/GuqierMcl/MailX/pull/47))
- The identity colour picker has a None option with a cross that hides the colour ring and restores the provider icon to full size. / 标识颜色提供叉号“不标识”选项，选择后隐藏颜色圆环，服务商图标恢复完整大小。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- The accent Add account button in account settings is labelled Add account instead of appearing as a hint whenever the dialog opens; accounts using a provider icon keep a transparent avatar background with the colour as a thin ring and more space between icon and ring. / 账号管理的强调色添加按钮显示“添加账号”，不再因打开弹窗自动出现提示；使用服务商图标的账号头像保留透明背景，账号颜色仅显示为细圆环，并增加图标与圆环之间的留白。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 163 / 126 / yeah.net use their own provider icons. / 163 / 126 / yeah.net 使用各自的服务商图标。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- The Add account button sits at the top right of the account list in the accent colour, and accounts with a chosen provider use its icon by default, which each account's profile can turn off. / 账号管理的添加按钮移至列表右上角并使用强调色；已选择服务商的账号默认使用服务商图标，可在每个账号的资料中单独关闭。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- When adding an account or editing its connection, an empty IMAP / SMTP username falls back to the email address. / 添加账号和修改连接时，IMAP / SMTP 登录用户名留空会使用邮箱地址。 ([#40](https://github.com/GuqierMcl/MailX/pull/40))
- The provider picker when adding an account is a single-line list of logo, name and short description, searchable by name, domain and keyword. / 添加账号的服务商选择改为 Logo、名称与短描述的单行列表，支持按名称、域名和关键词搜索。 ([#40](https://github.com/GuqierMcl/MailX/pull/40))
- The settings window opens on General first so the language is easy to change. / 设置窗口默认打开第一项“通用”，便于调整语言。 ([#29](https://github.com/GuqierMcl/MailX/pull/29))
- On macOS all account passwords are stored in one keychain entry, asking for authorization at most once per launch instead of once per account; old entries migrate on first launch and are cleaned up on the next. / macOS 上所有账号密码合并存放在一个钥匙串条目中，启动时最多只需授权一次，不再每个账号一次；旧条目会在首次启动时迁入、下次启动时清理。

### 🩹 Fixed

- Fixed the top shadow and outline of the main content island being clipped. / 修复主界面内容岛顶部阴影与描边被裁切的问题。 ([#84](https://github.com/GuqierMcl/MailX/pull/84))
- Fixed copied MCP client configuration on Windows carrying a `\\?\` prefix that stopped some clients from launching `mailx-mcp`. / 修复 Windows 上复制的 MCP 客户端配置带有 `\\?\` 前缀，导致部分客户端无法启动 `mailx-mcp` 的问题。 ([#83](https://github.com/GuqierMcl/MailX/pull/83))
- Fixed remote image loading state and Retry loading not working while reading a message. / 修复阅读邮件时远程图片的加载状态与“重试加载”无法使用的问题。 ([#78](https://github.com/GuqierMcl/MailX/pull/78))
- Fixed the Linux AppImage quitting at launch because tray libraries were missing; when tray libraries are unavailable the main window is kept and hiding to the background is disabled. / 修复 Linux AppImage 缺少托盘运行库导致启动退出的问题；托盘运行库不可用时保留主窗口并禁用后台隐藏。 ([#74](https://github.com/GuqierMcl/MailX/pull/74))
- Stopped building the background search index during quit so it no longer keeps using the local database after exit. / 退出时停止后台搜索索引建立，避免退出后继续占用本地数据库。 ([258e825](https://github.com/GuqierMcl/MailX/commit/258e825af4ea4b36128709ea6dfa6c5b759aa034))
- Fixed Linux packages depending on system libraries too new for Ubuntu 22.04 to launch. / 修复 Linux 安装包依赖过新的系统库，导致 Ubuntu 22.04 上无法启动的问题。 ([#73](https://github.com/GuqierMcl/MailX/pull/73))
- Fixed italics not slanting in the compose window: the UI font has no italic glyphs and synthesis was previously disabled, and it is now allowed for message bodies only. / 修复写信窗口中斜体不显示倾斜的问题：界面字体本身没有斜体字形，之前被禁止合成，现在只在正文里允许合成斜体。 ([#53](https://github.com/GuqierMcl/MailX/issues/53))
- Quitting now waits for draft saves and sends in progress, and cancels the quit if a save fails or the wait times out, so force-closing no longer loses unsaved content. / 退出时等待草稿保存和正在进行的发送，保存失败或等待超时会取消退出，避免强制关窗丢失未保存内容。 ([#48](https://github.com/GuqierMcl/MailX/pull/48))
- Fixed release notes being replaced with This version has no release notes while an update downloads; they now stay visible during download and retry after a failure. / 修复下载更新时更新说明被错误替换为“没有提供更新说明”的问题，下载完成和失败重试时也持续显示。 ([#46](https://github.com/GuqierMcl/MailX/pull/46))
- Providers that refuse access with an explicit security policy now show a distinct reason and stop automatic retries; after fixing it you can sync manually, keeping the local cache and pending flag changes. / 邮箱服务商明确以安全策略拒绝访问时显示独立原因，停止自动重试；处理后可手动同步恢复，保留本地缓存与待写标记。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- Attachments without a file name show a placeholder in the selected language instead of always Chinese. / 没有文件名的附件在英文界面下也按所选语言显示占位名称，不再固定显示中文。 ([#32](https://github.com/GuqierMcl/MailX/pull/32))

### 🔄 Changed

- The local database upgrades to v23, recording the origin of MCP drafts and attachments; older versions cannot open the upgraded database, so keep a full backup from before the upgrade if you may downgrade. / 本地数据库升级至 v23，记录 MCP 草稿与附件的来源；旧版无法直接打开升级后的数据库，降级前需保留升级前的完整数据备份。 ([#86](https://github.com/GuqierMcl/MailX/pull/86))
- Upgrading from 0.1.0-rc.3 moves the local database from v19 to v21 for delete undo and local archive; older versions cannot open the upgraded database. / 从 0.1.0-rc.3 升级时，本地数据库自动从 v19 升级到 v21，以支持删除撤销和本地归档；旧版本无法打开升级后的数据库。 ([#69](https://github.com/GuqierMcl/MailX/pull/69), [#71](https://github.com/GuqierMcl/MailX/pull/71))
- The local database upgrades to v19 with a full-text search index: on first launch existing mail is indexed in the background, search keeps working meanwhile, and the index grows on disk with the cached bodies. Older versions cannot open the upgraded database. / 本地数据库升级到 v19，新增全文搜索索引：首次启动后会在后台为已有邮件建立索引，期间搜索照常可用，索引会随已缓存的正文增长而占用更多磁盘空间；旧版本无法打开升级后的数据库。 ([#67](https://github.com/GuqierMcl/MailX/pull/67))
- The local database upgrades to a new version: queued and sent messages move to separate files, migrated on first launch, after which older versions cannot open the database; a sent message releases its local copy once the same message appears in the server's Sent folder. / 本地数据库升级到新版本：待发送与已发送邮件改存为独立文件，首次启动时自动迁移，迁移后旧版本无法再打开该数据库；已发送邮件在服务器的已发送文件夹出现同一封后会释放本地副本。 ([#49](https://github.com/GuqierMcl/MailX/pull/49))
- Upgrading from 0.1.0-alpha.4 moves the local database from v10 to v12 for account removal and provider icon preferences; older versions cannot open the upgraded data, so downgrade only from a backup taken before the upgrade. / 从 0.1.0-alpha.4 升级时，本地数据库自动从 v10 升级到 v12，以支持移除账号和服务商图标偏好；旧版无法直接打开升级后的数据，如需降级，请使用升级前的数据备份。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- Installer names follow the system (`MailX_<版本>_macos-arm64.dmg`, `MailX_<版本>_windows-x64-setup.exe`) so you can pick the right download by operating system. / 下载页的安装包文件名改为按系统命名（如 `MailX_<版本>_macos-arm64.dmg`、`MailX_<版本>_windows-x64-setup.exe`），便于直接按操作系统选择。
- 0.1.0-alpha.1 and 0.1.0-alpha.2 have no in-app updater, so you need to download and install a build with the updater once; in-app updates work from 0.1.0-alpha.3 onwards. / 0.1.0-alpha.1、0.1.0-alpha.2 不含应用内更新，需要手动下载安装带更新器的版本一次；自 0.1.0-alpha.3 起支持应用内更新。
- From 0.1.0-alpha.2 the app identifier is `com.mailxapp.desktop`, the user data directory is `MailX` (macOS: `~/Library/Application Support/MailX`, Windows: `%APPDATA%\MailX`, Linux: `~/.local/share/MailX`) and the system credential entry uses the service name `MailX`. Upgrading from alpha.1 does not move the old directory or credentials automatically; move the data and enter each account password or authorization code again. Anyone on alpha.2 or later does not need to migrate again. / 自 0.1.0-alpha.2 起，应用标识为 `com.mailxapp.desktop`，用户数据目录为 `MailX`（macOS：`~/Library/Application Support/MailX`，Windows：`%APPDATA%\MailX`，Linux：`~/.local/share/MailX`），系统凭据条目使用服务名 `MailX`。从 alpha.1 升级时，旧目录中的数据和旧凭据不会自动迁移，需要迁移数据并重新输入各账号的密码或授权码；已使用 alpha.2 或更新版本的用户无需重复迁移。

[Unreleased]: https://github.com/GuqierMcl/MailX/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/GuqierMcl/MailX/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/GuqierMcl/MailX/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/GuqierMcl/MailX/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/GuqierMcl/MailX/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/GuqierMcl/MailX/releases/tag/v0.1.0
