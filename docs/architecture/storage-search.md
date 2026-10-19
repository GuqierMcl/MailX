# 存储、搜索与备份

状态：**账号、文件夹与邮件头存储和头部查询已有实现 / 正文、全文索引与备份为 Initial draft** · 2026-10-18。当前使用 `rusqlite`，编号 schema 位于 `core/src/storage/migrations/`：v1 为账号和凭证恢复，v2 为文件夹，v3 为邮件头与同步游标。真实服务商和平台运行证据见[验证策略](../development/validation.md)。

## 1. 所有权与目录

桌面直接使用 Tauri `app_data_dir()` 对应的 user-data-dir；路径由宿主注入，不在仓库或当前工作目录存生产数据。未来 Web 使用容器 `/data`。每个目录加实例锁，桌面、server 和 MCP bridge 不得各自打开独立写入核心。SQLite 位于本地磁盘，不能把运行中的数据库放在 NFS / SMB 共享目录。

```text
user-data-dir/
  mailx.sqlite3             # 当前：账号、连接、认证、路由、文件夹、邮件头与游标
  mailx.sqlite3-wal / -shm  # SQLite 管理的运行文件
  mailx.lock                # 核心实例持有的文件锁
```

`blobs/`、`tmp/`、`logs/` 随邮件、附件与诊断功能引入，当前尚未建立这些存储流程。

密钥不在该 SQLite 中明文保存。桌面存系统凭据库，服务端用外部注入主密钥保护独立密钥库，详见[网络与安全](network-security.md)。正文和附件首版默认不做应用层整体加密，磁盘与备份的加密由部署者承担。

WAL 允许并发读取但仍只有一个写入者，且依赖本机共享内存。当前为单连接同步服务，宿主在阻塞线程串行执行；未来同步与搜索负载出现后再评估读取连接池。[SQLite WAL 文档](https://www.sqlite.org/wal.html)。

## 2. 当前表与后续邮件模型

账号相关 ID 使用 UUID 字符串，时间当前存 UTC RFC 3339 文本。`accounts`、`mail_connections`、`auth_profiles`、`network_routes` 的关系与字段见[账号配置](account-configuration.md)；`create_requests` 记录不含秘密的创建幂等信息，`credential_journal` 记录可恢复的配置快照与秘密引用，`schema_migrations` 记录版本及 checksum。当前没有旧草案中的账号级单一 `auth_kind` 或混合接收/发送配置字段。

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

### 后续模型（尚未建表）

| 表 | 核心字段 | 关键约束 |
| --- | --- | --- |
| message_bodies | message_id, plain_text, sanitized_html, sanitizer_version, raw_blob_id | 1:1 messages；HTML 净化版本可重建 |
| attachments | id, message_id, mime_part, file_name, media_type, size, blob_id, content_id | 文件名仅用于展示；下载用 ID |
| blobs | id, digest, relative_path, size, created_at | 应用生成路径；不可变，引用统计后回收 |
| pending_mutations | id, account_id, mailbox_source_id, target_identity, kind, desired_value, client_mutation_id, state | unique(account_id, client_mutation_id) |
| drafts | id, account_id, revision, client_mutation_id, create_payload_hash, recipients, subject, plain_text, reply_headers, updated_at | unique(account_id, client_mutation_id) 保证创建幂等；更新使用乐观锁 |
| draft_attachments | draft_id, attachment_id, blob_id, file_name, media_type | 发送前校验完整性 |
| outbox | id, draft_id, revision, account_id, client_mutation_id, payload_hash, mime_blob_id, state, attempt_stage, sent_copy_state | unique(account_id, client_mutation_id) |
| ai_runs | id, kind, provider_id, model, state, context_refs, context_hash, input_revision, result, error_code | 原始完整提示词默认不另存 |
| mcp_grants | client_id, account_scope, action_scope, expires_at, revoked_at | 随时可撤销；不存邮箱凭证 |
| audit_events | id, time, actor_id, account_id, action, resource_id, outcome, request_id | 不记录正文、地址列表、OTP、密钥 |

Web 在 M5 增加单管理员认证、Session 与 TOTP 恢复码表；不为未来多租户预建租户系统。模型提供商配置由 `AiService` 管理，配置与密钥引用保存，凭证本身交给 `SecretStore`。

## 3. 事务与文件一致性

邮件批次、flags、删除与既有游标推进事务核对账号当前来源、文件夹归属和 UIDVALIDITY。文件夹计数更新核对来源与归属，代际初始化事务负责建立或切换 UIDVALIDITY。批次邮件与最高已处理 UID 原子提交，数据库失败不推进游标；账号切换服务器、文件夹删除或代际切换后的迟到写入被拒绝。来源切换后读取立即排除旧来源，目录清理或删除文件夹时通过外键级联删除对应邮件和游标。

- 当前连接设置外键校验和 `busy_timeout`，启用 WAL、`synchronous=FULL`，为配置提交提供持久化基础。未来草稿与发送状态沿用该边界。性能调整需基准与 ADR。
- 网络、MIME 大文件解析和模型请求不能占用数据库事务；当前 `rusqlite` 调用隔离到宿主阻塞执行器，未来连接池与事务调度必须继续避免阻塞异步任务。
- 后续 blob 先写受配额限制的临时文件、校验和持久化，再在同卷原子重命名；之后事务写引用。文件写成功而事务失败产生的孤儿由延迟 GC 清理，不能先提交数据库再补文件。
- 后续正文与对应全文索引同事务更新，避免搜索结果指向不存在的内容；索引是派生数据，可以重建。
- 后续 GC 不删除被草稿、outbox 或备份引用的 blob。磁盘满时不清理用户未发送草稿来给邮件缓存腾空间。

## 4. 缓存边界

当前每轮从固定窗口内选 UID 最大的 1,000 封补齐头部，每批最多 200 封；之前已缓存的头部不会因为退出最新 1,000 封集合而淘汰，因此总缓存可能超过 1,000。删除对账检查窗口内完整 SEARCH 结果，flags 对账只检查当轮选定集合；窗口外旧缓存保留，不参与删除或 flags 对账，可能保留服务端后来删除的旧邮件。更早历史回填、显式清理与缓存设置尚未实现。

后续正文和接收附件拟使用 LRU 配额，初始合计 2 GiB，可配置。草稿、待发送 MIME 与用户固定的离线内容不参与自动淘汰，单独显示占用。缓存被淘汰后 `bodyState=missing`，同步状态不能仍宣称可离线阅读。

后续默认单次正文解析上限 20 MiB，附件单件上限 50 MiB，完整消息上限 100 MiB；提供商限制可能更低。超限返回明确错误或原始文件手动保存入口，不截断后假装完整。MIME 深度默认上限 30，附件不自动解压或执行。

## 5. 当前头部查询与后续全文搜索

`emails_list` 搜索**已缓存且当前可见的头部**，查询由 `AccountService` 校验后交给 Storage；账号与文件夹作用域在 SQL 中限定，不先跨账号搜索再在界面隐藏。当前支持 `accountIds`、`folderId` 或 `folderRole`、`unread`、`hasAttachment`、`text`、`receivedAfter` / `receivedBefore`。统一收件箱使用 `folderRole=inbox`，详情用 `emails_get`；完整参数和错误见[应用 API](../contracts/app-api.md#2-当前邮件分页与查询)。

`text` 去除首尾空白，对主题、发件人显示名和邮箱地址做参数化 LIKE 子串匹配；转义 `%`、`_` 和反斜杠，用户文本不作为 SQL 或搜索表达式。当前没有 FTS 索引，不搜索正文，也没有三字符最短限制。时间范围左闭右开，RFC 3339 参数先转为 UTC 毫秒格式。查询默认每页 50 条，上限 200；前端请求 100 条，按 `(receivedAt DESC, id DESC)` 游标分页并防抖搜索，丢弃过期响应。

返回 `coverage { headersComplete, windowCapped, bodiesComplete, syncedAfter, syncedBefore }`：对账号 / 文件夹作用域内的全部当前可选文件夹，存在游标、`backfill_done=1` 且 `window_capped=0` 才认为头部窗口完整；无匹配文件夹或任一文件夹未同步时为 false。`syncedAfter` 为涉及游标中最晚的固定窗口起点（UTC 日期），无游标时为 null；`syncedBefore=null`，`bodiesComplete=false`。`windowCapped=true` 表示至少一个匹配目录的窗口超过本轮选取上限；coverage 不表达当前任务是否正在运行。

完整性声明的是这些文件夹的**同步窗口**，不声明整箱历史已缓存；文本、未读、附件或时间过滤不缩小计算完整性的文件夹范围。即使查询请求窗口之前的时间，`headersComplete=true` 也不能证明那段历史完整，应结合 `syncedAfter` 理解结果。邮件摘要固定 `preview=""`、`bodyState=missing`、`pendingChanges=false`，详情 body 为 null、attachments 为空数组。

FTS5 trigram 对主题、地址显示文本和已缓存纯文本的检索仍是后续候选；其三字符限制与中文短词体验需要在接入时重新评审，不套用到当前 LIKE 查询。[SQLite FTS5 文档](https://www.sqlite.org/fts5.html)。向量索引、云端 embedding 和自然语言转 SQL 不在本阶段范围内。

## 6. 迁移与恢复

当前由宿主启动任务取得目录锁，再检查 `user_version` 与 `schema_migrations` 中的 SHA-256 checksum；编号 SQL 按序以逐步事务执行，当前登记至 v3，可从已有 v1 / v2 顺序升级，重复启动不会重跑。较新 schema、校验不符或非空无版本数据库均拒绝继续，保留原数据。核心打开与写入前还恢复凭证更新 journal，具体步骤见[账号配置的一致性协议](account-configuration.md#4-sqlite-与凭据库的一致性)。

[数据库迁移与启动方案](database-migrations.md)记录本轮已实现的迁移核心和启动准入；升级前备份、恢复入口和完整故障矩阵仍是后续要求。

以下为后续升级和邮件数据恢复要求：迁移在任何同步任务启动前执行，每个迁移在支持的事务边界内完成。升级前创建一致性备份；不支持直接降级打开更高版本数据库，提示恢复兼容备份。迁移失败保持原数据可恢复，禁止删除数据库重建作为通用修复手段。

健康检查覆盖 `integrity_check`、外键校验、FTS 一致性和 blob 引用缺失。修复优先重建派生索引或重新获取可下载缓存，不损坏草稿、发送历史和凭证引用。

## 7. 备份协议（后续）

v0.1 以停机备份为正式路径：暂停并等待任务退出 → 关闭所有数据库连接 → 复制完整数据目录 → 输出版本、文件清单与校验和。不能只复制运行中的 `mailx.sqlite3`，也不能把备份目录直接放回当前运行目录覆盖。

后续在线备份采用 SQLite Backup API 生成数据库快照，同时冻结 blob 删除并复制快照引用的不可变文件；两者都完成后才能标记备份成功。[SQLite Backup API](https://www.sqlite.org/backup.html)。

恢复到新目录，校验清单和 schema 兼容性后再切换。桌面系统凭据库不随数据目录备份，恢复后需重新授权；Web 主密钥须独立保管，丢失时只能重新配置凭证。每个发布版本至少演练一次“新目录恢复并读取草稿”的流程。
