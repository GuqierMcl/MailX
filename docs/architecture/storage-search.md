# 存储、搜索与备份

状态：**账号、文件夹与邮件头、正文缓存和头部查询已有实现 / 全文索引与备份为 Initial draft** · 2026-10-20。当前使用 `rusqlite`，编号 schema 位于 `core/src/storage/migrations/`：v1 为账号和凭证恢复，v2 为文件夹，v3 为邮件头与同步游标，v4 为正文与 MIME 部分。真实服务商和平台运行证据见[验证策略](../development/validation.md)。

## 1. 所有权与目录

桌面使用以产品名命名的 `MailX` user-data-dir（Tauri `app_data_dir()` 的同级目录，不含 identifier）；路径由宿主注入，不在仓库或当前工作目录存生产数据。未来 Web 使用容器 `/data`。每个目录加实例锁，桌面、server 和 MCP bridge 不得各自打开独立写入核心。SQLite 位于本地磁盘，不能把运行中的数据库放在 NFS / SMB 共享目录。

```text
user-data-dir/
  mailx.sqlite3             # 当前：账号、连接、认证、路由、文件夹、邮件头、正文与游标
  mailx.sqlite3-wal / -shm  # SQLite 管理的运行文件
  mailx.lock                # 核心实例持有的文件锁
  blobs/<前两位>/<SHA-256>  # 不可变原始 MIME
```

正文 blob 按内容哈希保存，临时文件与最终文件在同一文件系统；不保存主题或文件名作为路径。

密钥不在该 SQLite 中明文保存。桌面存系统凭据库，服务端用外部注入主密钥保护独立密钥库，详见[网络与安全](network-security.md)。正文和附件首版默认不做应用层整体加密，磁盘与备份的加密由部署者承担。

WAL 允许并发读取但仍只有一个写入者，且依赖本机共享内存。当前为单连接同步服务，宿主在阻塞线程串行执行；未来同步与搜索负载出现后再评估读取连接池。[SQLite WAL 文档](https://www.sqlite.org/wal.html)。

## 2. 当前表与后续邮件模型

账号相关 ID 使用 UUID 字符串，时间当前存 UTC RFC 3339 文本。`accounts`、`mail_connections`、`auth_profiles`、`network_routes` 的关系与字段见[账号配置](account-configuration.md)；`create_requests` 记录不含秘密的创建幂等信息，`credential_journal` 记录可恢复的配置快照与秘密引用，`schema_migrations` 记录版本及 checksum。当前没有旧草案中的账号级单一 `auth_kind` 或混合接收/发送配置字段。

`accounts.use_provider_icon`（v12）以 0 / 1 保存是否使用可用服务商图标的资料偏好，默认 1。已有账号升级后和缺少此字段的旧账号快照 / journal 均默认启用。更新此偏好不推进连接 `revision`、不修改凭证或重启同步；连接更新与 journal 恢复不覆盖已保存的偏好。

### 已建邮件表

| 表 | 实际字段与结构 | 关键约束 |
| --- | --- | --- |
| `folders`（v2） | id、account_id、mailbox_source_id、remote_name、parent_id、name、delimiter、role、selectable、attributes、unread_count、total_count、uid_next、uid_validity、synced_at | `UNIQUE(account_id, mailbox_source_id, remote_name)`；v3 增加 `folders_identity(id, account_id, mailbox_source_id)` 唯一索引 |
| `folder_sync_cursors`（v3） | folder_id、uid_validity、window_start、window_capped、backfill_done、highest_uid、updated_at | folder_id 是主键，外键引用 folders 并级联删除；邮件批次和 highest_uid 在同一事务推进 |
| `messages`（v3） | id、account_id、mailbox_source_id、folder_id、uid_validity、uid、message_id_header、subject、from_name、from_address、to_addresses、cc_addresses、reply_to_addresses、received_at、sent_at、size、is_read、is_flagged、flags、has_attachment、remote_state、updated_at | `UNIQUE(folder_id, uid_validity, uid)`；复合外键引用 folders 的 id / account_id / mailbox_source_id 并级联删除 |

`folders.synced_at` 记录最后一次成功 STATUS 计数观测的时间；新发现且尚无计数的目录使用空串。LIST-only 更新保留旧计数和此时间，`Folder.countsKnown` 由可选目录且该时间非空派生，未知计数不能当成服务器真实零值。此调整不改写已应用迁移；既有版本存储可选目录前已完成 STATUS。

以上为 `STRICT` 表；布尔字段使用 0 / 1 CHECK，`remote_state` 限定为三种值：

- `present`：远端位置仍有效；只有所属账号的当前来源、可选文件夹及其游标的当前 UIDVALIDITY 才能查询到。
- `deleted`：窗口内完整 UID SEARCH 成功后确认位置已不存在；保留本地行但不展示，不等同于只设置了服务端 `\Deleted` 标记。
- `stale`：UIDVALIDITY 改变后失效的旧代际；保留本地行但不展示，不把旧 UID 套用到新邮件。

本地邮件 id 为 UUID，重复写入同一 `(folder_id, uid_validity, uid)` 使用 upsert 且保留 id。收件人、抄送、回复地址保存为 `{ name: string | null, address: string }` 的 JSON 数组，flags 保存为字符串 JSON 数组。邮件时间为固定 UTC 毫秒格式 `YYYY-MM-DDTHH:MM:SS.sssZ`，字典序与时间序一致。账号与文件夹的查询索引都包含 `remote_state, received_at DESC, id DESC`。

游标的 `window_start` 是首次同步或 UIDVALIDITY 切换时的 UTC 当日减 30 天，代际内保持不变。`window_capped` 表示完整搜索匹配超过每轮 1,000 封上限；`backfill_done` 表示当轮选中的 UID 头部均已抓取，且删除与 flags 对账成功，不能用 `highest_uid` 推断所有历史已完成。文件夹 STATUS 更新的 UIDNEXT / UIDVALIDITY 与已提交的邮件同步游标分开存储，避免目录刷新覆盖旧代际判断依据。

### 正文缓存（v4）

`messages` 追加 body_state（missing / fetching / ready / failed）、body_failure 和 preview。
`message_bodies` 以 message_id 为主键，保存 raw_sha256、raw_size、plain_text、sanitized_html、sanitizer_version、links、remote_images_blocked、truncated、fetched_at；`message_parts` 以 (message_id, part_index) 为主键，保存 content_id、filename、mime_type、size、is_inline，另有 CID 索引。两表随 messages 级联删除。当前净化版本为 2，旧结果可从本地 blob 按需重建；重建同样验证当前身份，不触发网络下载。

### 远程图片的阅读期内存缓存

远程图片不建表、不写入 blob，也不复用跨阅读缓存。验证后的图片只保存在当前文档绑定的内存中，按 URL 复用，保留字节总预算 64 MiB；超过预算的图片本次仍可显示，但不额外保留。切换邮件、关闭阅读、隐藏或文档意外重载时，UI 释放对应令牌与缓存；收紧图片策略同样释放缓存，在途请求不能重新写回已释放的绑定。再次打开会建立新的空缓存，继续按 ask / automatic / never 策略决定是否加载。

当前净化版本为 2，旧正文缓存从本地 MIME 重建，不重新下载邮件。远程图片功能没有新增数据库迁移，schema 仍为 v4；原始 MIME 与 CID 的持久化行为不变。

### 后续模型（尚未建表）

| 表 | 核心字段 | 关键约束 |
| --- | --- | --- |
| attachments | id, message_id, mime_part, file_name, media_type, size, blob_id, content_id | 文件名仅用于展示；下载用 ID |
| blobs | id, digest, relative_path, size, created_at | 应用生成路径；不可变，引用统计后回收 |
| pending_mutations | id, account_id, mailbox_source_id, target_identity, kind, desired_value, client_mutation_id, state | unique(account_id, client_mutation_id) |
| drafts | id, account_id, revision, client_mutation_id, create_payload_hash, recipients, subject, plain_text, reply_headers, updated_at | unique(account_id, client_mutation_id) 保证创建幂等；更新使用乐观锁 |
| draft_attachments | draft_id, attachment_id, blob_id, file_name, media_type, size, inline, content_id, origin, created_at | 添加时复制进 blob；草稿删除时级联删除；入队前校验哈希（见[发信设计](sending.md)第 16 节；普通附件已实现）。内嵌图片（已实现，macOS 用户手测通过）用 `inline = 1` 加 `content_id` 表示，草稿 HTML 以 `cid:` 引用；迁移 v17 已加 `(draft_id, content_id) WHERE inline = 1` 部分唯一索引。保存草稿不删记录，未被正文引用的内嵌记录在新增附件时或草稿发送、删除时清理（sending.md 16.11.2、16.11.6） |
| outbox | id, draft_id, account_id, client_mutation_id, message_id, snapshot_hash, snapshot_size, snapshot_released_at, subject, recipients_preview, recipient_count, state, accepted_at, server_copy_message_id, deleted_locally_at, resolution 等；v18 加 preview, attachment_count | unique(account_id, client_mutation_id)；快照存为 blob（发信设计第 16.6 节），服务器副本对上后释放（第 16.7 节）。尚未对上的 `accepted` 行在已发送角色文件夹的列表中作为本机副本出现（已实现，见发信设计第 9 节） |
| ai_runs | id, kind, provider_id, model, state, context_refs, context_hash, input_revision, result, error_code | 原始完整提示词默认不另存 |
| mcp_grants | client_id, account_scope, action_scope, expires_at, revoked_at | 随时可撤销；不存邮箱凭证 |
| audit_events | id, time, actor_id, account_id, action, resource_id, outcome, request_id | 不记录正文、地址列表、OTP、密钥 |

Web 在 M5 增加单管理员认证、Session 与 TOTP 恢复码表；不为未来多租户预建租户系统。模型提供商配置由 `AiService` 管理，配置与密钥引用保存，凭证本身交给 `SecretStore`。

## 3. 事务与文件一致性

邮件批次、flags、删除与既有游标推进事务核对账号当前来源、文件夹归属和 UIDVALIDITY。文件夹计数更新核对来源与归属，代际初始化事务负责建立或切换 UIDVALIDITY。批次邮件与最高已处理 UID 原子提交，数据库失败不推进游标；账号切换服务器、文件夹删除或代际切换后的迟到写入被拒绝。来源切换后读取立即排除旧来源，目录清理或删除文件夹时通过外键级联删除对应邮件和游标。

- 当前连接设置外键校验和 `busy_timeout`，启用 WAL、`synchronous=FULL`，为配置提交提供持久化基础。未来草稿与发送状态沿用该边界。性能调整需基准与 ADR。
- 网络、MIME 大文件解析和模型请求不能占用数据库事务；当前 `rusqlite` 调用隔离到宿主阻塞执行器，未来连接池与事务调度必须继续避免阻塞异步任务。
- 当前 blob 先写临时文件、同步并原子重命名，文件持久化后短事务写入正文、parts、preview 与 ready；已存在哈希复用。启动持目录独占锁、后台任务开始前恢复 fetching 并清理未引用的严格哈希文件 / 本程序临时文件，不跟随符号链接。运行期 GC、配额与 LRU 尚未实现。
- 后续正文与对应全文索引同事务更新，避免搜索结果指向不存在的内容；索引是派生数据，可以重建。
- 后续 GC 不删除被草稿、outbox 或备份引用的 blob；写信附件落地后，启动 GC 把 `draft_attachments` 与尚未释放的 outbox 快照算作存活引用（内嵌图片的记录同样算，包括暂未被正文引用、尚待清理的）。已发送快照在服务器对上后只清空引用，文件由下次启动 GC 回收。磁盘满时不清理用户未发送草稿来给邮件缓存腾空间。

## 4. 缓存边界

当前每轮从固定窗口内选 UID 最大的 1,000 封补齐头部，每批最多 200 封；之前已缓存的头部不会因为退出最新 1,000 封集合而淘汰，因此总缓存可能超过 1,000。删除对账检查窗口内完整 SEARCH 结果，flags 对账只检查当轮选定集合；窗口外旧缓存保留，不参与删除或 flags 对账，可能保留服务端后来删除的旧邮件。更早历史回填、显式清理与缓存设置尚未实现。

后续正文和接收附件拟使用 LRU 配额，初始合计 2 GiB，可配置。草稿（含写信附件）、待发送与尚未对上服务器的已发送 MIME 快照、用户固定的离线内容不参与自动淘汰，单独显示占用。缓存被淘汰后 `bodyState=missing`，同步状态不能仍宣称可离线阅读。

当前按需正文实现：完整原始 MIME 上限 100 MiB，既检查 RFC822.SIZE，也在接收层按超限 literal 预分配之前拒绝；单个解码正文保留最多 20 MiB，超限按 UTF-8 边界截断并明确标记 truncated，不假装完整。20 MiB 是保留正文上限，不是解析器总内存上限。MIME 深度上限 30，内嵌资源单件上限 50 MiB；附件仅列元数据，不解压、执行或提供独立保存。原始 MIME 按内容哈希写入 blobs，迁移 v4 为正文 / MIME 部分建表，派生 HTML 带净化版本；文件同步与原子重命名先于数据库引用，启动恢复遗留 fetching 并清理孤立 blob。原始 MIME 的运行期配额与 LRU 尚不实现；实际验收与服务商覆盖见验证策略。

## 5. 当前头部查询与后续全文搜索

`emails_list` 搜索**已缓存且当前可见的头部**，查询由 `AccountService` 校验后交给 Storage；账号与文件夹作用域在 SQL 中限定，不先跨账号搜索再在界面隐藏。当前支持 `accountIds`、`folderId` 或 `folderRole`、`unread`、`hasAttachment`、`text`、`receivedAfter` / `receivedBefore`。统一收件箱使用 `folderRole=inbox`，详情用 `emails_get`；完整参数和错误见[应用 API](../contracts/app-api.md#2-当前邮件分页与查询)。

`text` 去除首尾空白，对主题、发件人显示名和邮箱地址做参数化 LIKE 子串匹配；转义 `%`、`_` 和反斜杠，用户文本不作为 SQL 或搜索表达式。当前没有 FTS 索引，不搜索正文，也没有三字符最短限制（全文搜索规划见 5.1）。时间范围左闭右开，RFC 3339 参数先转为 UTC 毫秒格式。查询默认每页 50 条，上限 200；前端请求 100 条，按 `(receivedAt DESC, id DESC)` 游标分页并防抖搜索，丢弃过期响应。

已实现（发信设计第 9.3 节）：查询范围包含已发送角色文件夹时，在同一查询里 `UNION ALL` 合并 outbox 中尚未对上服务器的 `accepted` 行，排序时间取 `accepted_at`，ID 为 `local-sent:<outbox id>`，沿用同一游标；文本搜索对其主题与 `recipients_preview` 做同样的参数化 LIKE。

返回 `coverage { headersComplete, windowCapped, bodiesComplete, syncedAfter, syncedBefore }`：对账号 / 文件夹作用域内的全部当前可选文件夹，存在游标、`backfill_done=1` 且 `window_capped=0` 才认为头部窗口完整；无匹配文件夹或任一文件夹未同步时为 false。`syncedAfter` 为涉及游标中最晚的固定窗口起点（UTC 日期），无游标时为 null；`syncedBefore=null`，`bodiesComplete=false`。`windowCapped=true` 表示至少一个匹配目录的窗口超过本轮选取上限；coverage 不表达当前任务是否正在运行。

完整性声明的是这些文件夹的**同步窗口**，不声明整箱历史已缓存；文本、未读、附件或时间过滤不缩小计算完整性的文件夹范围。即使查询请求窗口之前的时间，`headersComplete=true` 也不能证明那段历史完整，应结合 `syncedAfter` 理解结果。邮件摘要的 preview / bodyState 来自 v4 缓存列，pendingChanges=false。详情返回缓存纯文本、HTML 存在标志、截断和远程图片计数，以及非内联附件元数据；没有全文搜索。

全文搜索的方案见 5.1（2026-10-30 确认，未实现）。向量索引、云端 embedding 和自然语言转 SQL 不在本阶段范围内。

### 5.1 本地全文搜索（规划，未实现）

只做**本地**搜索：索引与查询都在本机 SQLite 里完成，不调用服务商的 IMAP `SEARCH`，搜不到本地没有的邮件。仓库锁定的 `libsqlite3-sys` 0.37 打包 SQLite 3.51.3 并开启 FTS5，trigram 分词器（3.34 起内置）可直接使用，不引入新依赖。参考 [SQLite FTS5 文档](https://www.sqlite.org/fts5.html)。

**分批**

| 分批 | 内容 |
| --- | --- |
| 第一批（F0–F4，本次） | 索引已有数据：头部、预览、已缓存正文、附件名、本机已发送副本；查询、短词退回、命中片段与覆盖说明 |
| 第二批（规划） | 同步窗口内后台下载正文文本部分，只写入索引，让正文搜索覆盖最近的邮件 |
| 不做 | 服务商端 `SEARCH`、相关度排序、`from:` 等字段语法、向量 / 语义搜索 |

**索引结构**

- 一张 FTS5 表，`tokenize = 'trigram case_sensitive 0 remove_diacritics 1'`，每封邮件一行，列为：`subject`、`people`（发件人、收件人、抄送的显示名与地址）、`preview`、`body`（已缓存纯文本，最多 64 KiB，按 UTF-8 字符边界截断）、`attachments`（附件文件名）。表保存内容，供 `snippet()` 与短词退回使用。
- FTS 行用整数 rowid，经映射表关联到邮件 ID（服务器邮件的 `messages.id`，本机已发送副本的 `local-sent:<outbox id>`），映射表带外键或由同一事务维护，邮件消失时随之删除。
- 索引是派生数据：版本号记在状态表里，分词器、列或截断规则变化时整表重建；健康检查发现不一致时也重建，不影响邮件数据本身。

**维护**

- 与数据写入**同一事务**更新索引：头部入库与更新（主题、地址）、预览写入、正文缓存写入、附件元数据写入、邮件删除与来源代际切换、账号删除；本机已发送副本在入队冻结时写入（正文取快照纯文本），对上服务器、从本机移除或快照释放时删除。
- 升级后的已有数据由启动后的后台任务分批补建（每批数百封），记录进度、可中断续做；补建期间照常可搜，结果的覆盖说明反映已索引的数量。

**查询**

- `text` 按 Unicode 空白拆成词，所有词都须命中（AND）；双引号括起来的部分作为一个整体匹配。最多 16 个词、总长 256 个字符，超出返回 `validation`。用户文本一律按字面量处理，不作为 FTS 查询语法执行。
- 长度不少于 3 个字符的词用 FTS `MATCH`（每个词包成转义后的字符串）；少于 3 个字符的词（如"发票""会议"）退回对索引内容列的参数化 LIKE 子串匹配。两种词可以混用，结果取交集。
- 与现有作用域、未读、附件、时间筛选及 `(receivedAt DESC, id DESC)` 游标组合，仍按时间排序；本机已发送副本同样参与。
- 正文或附件名命中时，摘要带命中片段：由 `snippet()` 取得，以纯文本分段返回（普通段与命中段），前端渲染高亮，不返回 HTML。

**覆盖说明**

- `coverage` 新增作用域内已收进索引的正文数，界面在结果上方说明"N 封邮件的正文已收进索引"，不暗示整箱正文都被搜索过；`bodiesComplete` 仍为 false。

**分步（第一批）**

| 步骤 | 内容 | 主要验证 |
| --- | --- | --- |
| F0 | 探针：1 万 / 5 万封合成数据下 trigram 查询、短词 LIKE 退回的耗时与索引体积，中英混排、全角半角、大小写、变音符号 | 结果记入验证策略；超出可接受范围时先回来调整方案 |
| F1 | 迁移、索引表与映射表、同事务维护、后台补建、版本与重建 | 各写入路径都更新索引、删除不留残行、补建可续做、重建幂等 |
| F2 | 查询：拆词与引号、转义、短词退回、筛选与游标组合、命中片段、覆盖计数 | 注入与特殊字符、两字中文词、混合词、分页连续 |
| F3 | 宿主与前端：命中片段高亮、覆盖提示、文案 | 高亮不经 HTML、覆盖提示随范围变化 |
| F4 | 测试与文档 | 自动检查；真实邮箱上的中文搜索手测 |

## 6. 迁移与恢复

当前由宿主启动任务取得目录锁，再检查 `user_version` 与 `schema_migrations` 中的 SHA-256 checksum；编号 SQL 按序以逐步事务执行，当前登记至 v12，可从已有版本顺序升级，重复启动不会重跑。较新 schema、校验不符或非空无版本数据库均拒绝继续，保留原数据。核心打开与账号写入前还恢复凭证更新 journal 和移除后的凭证清理意图，具体步骤见[账号配置的一致性协议](account-configuration.md#4-sqlite-与凭据库的一致性)。

账号移除使用一个事务核对连接 revision 与无 `sending` 项，再清除配置及其全部来源的缓存、草稿和发件记录。v11 的 `account_removals`、`removed_create_requests` 与 `secret_gc` 保存可重试收据、禁止创建请求重放的墓碑和不含秘密的清理引用；凭证删除失败不恢复已移除账号。共享 blob 只删除数据库引用，下次启动 GC 再回收孤立文件，不在运行中执行全局 GC；已保存到用户选择路径的附件不受账号移除影响。

[数据库迁移与启动方案](database-migrations.md)记录本轮已实现的迁移核心和启动准入；升级前备份、恢复入口和完整故障矩阵仍是后续要求。

以下为后续升级和邮件数据恢复要求：迁移在任何同步任务启动前执行，每个迁移在支持的事务边界内完成。升级前创建一致性备份；不支持直接降级打开更高版本数据库，提示恢复兼容备份。迁移失败保持原数据可恢复，禁止删除数据库重建作为通用修复手段。

健康检查覆盖 `integrity_check`、外键校验、FTS 一致性和 blob 引用缺失。修复优先重建派生索引或重新获取可下载缓存，不损坏草稿、发送历史和凭证引用。

## 7. 备份协议（后续）

v0.1 以停机备份为正式路径：暂停并等待任务退出 → 关闭所有数据库连接 → 复制完整数据目录 → 输出版本、文件清单与校验和。不能只复制运行中的 `mailx.sqlite3`，也不能把备份目录直接放回当前运行目录覆盖。

后续在线备份采用 SQLite Backup API 生成数据库快照，同时冻结 blob 删除并复制快照引用的不可变文件；两者都完成后才能标记备份成功。[SQLite Backup API](https://www.sqlite.org/backup.html)。

恢复到新目录，校验清单和 schema 兼容性后再切换。桌面系统凭据库不随数据目录备份，恢复后需重新授权；Web 主密钥须独立保管，丢失时只能重新配置凭证。每个发布版本至少演练一次“新目录恢复并读取草稿”的流程。
