# 存储、搜索与备份

状态：**Initial draft** · 表结构、缓存与搜索方案可随实现验证调整。

## 1. 所有权与目录

桌面使用平台应用数据目录，Web 使用容器 `/data`；路径由宿主注入，不能硬编码开发者路径。每个目录加实例锁，桌面、server 和 MCP bridge 不得各自打开独立写入核心。SQLite 位于本地磁盘，不能把运行中的数据库放在 NFS / SMB 共享目录。

```text
data/
  mailx.sqlite              # 结构化配置、缓存内容、草稿与任务
  mailx.sqlite-wal / -shm   # SQLite 管理的运行文件
  blobs/                    # 不可变附件 / 原始 MIME / 发送快照
  tmp/                      # 带配额、启动时可回收的临时文件
  logs/                     # 脱敏且轮转的日志
```

密钥不在该 SQLite 中明文保存。桌面存系统凭据库，服务端用外部注入主密钥保护独立密钥库，详见[网络与安全](network-security.md)。正文和附件首版默认不做应用层整体加密，磁盘与备份的加密由部署者承担。

WAL 允许并发读取但仍只有一个写入者，且依赖本机共享内存。采用一个数据库执行器串行写入，读取连接池初始上限 2。[SQLite WAL 文档](https://www.sqlite.org/wal.html)。

## 2. 数据模型草案

所有业务 ID 使用 UUID 字符串；SQL 内部可用整型 rowid 服务 FTS。时间持久化为 UTC 毫秒整数，对外转 RFC 3339。下表定义必要列与约束，SQL DDL 在 M1 写成迁移；它不是已存在数据库的描述。

| 表 | 核心字段 | 关键约束 |
| --- | --- | --- |
| accounts | id, provider, address, receive_config, send_config, auth_kind, secret_refs, route_id, enabled | 密钥仅存引用；账号为隔离单元 |
| network_routes | id, mode, proxy_endpoint, proxy_secret_ref, dns_mode | endpoint 不含 userinfo 凭证 |
| folders | id, account_id, remote_name, special_use, uid_validity | unique(account_id, remote_name) |
| messages | id, rowid, account_id, folder_id, uid_validity, uid, message_id_header, received_at, subject, addresses, flags, body_state, remote_state | unique(account_id, folder_id, uid_validity, uid)；复合外键保证账号与文件夹一致 |
| message_bodies | message_id, plain_text, sanitized_html, sanitizer_version, raw_blob_id | 1:1 messages；HTML 净化版本可重建 |
| attachments | id, message_id, mime_part, file_name, media_type, size, blob_id, content_id | 文件名仅用于展示；下载用 ID |
| blobs | id, digest, relative_path, size, created_at | 应用生成路径；不可变，引用统计后回收 |
| sync_cursors | folder_id, uid_validity, highest_observed_uid, modseq, coverage, backfill_cursor | 与当前批次同事务推进 |
| pending_mutations | id, account_id, target_identity, kind, desired_value, client_mutation_id, state | unique(account_id, client_mutation_id) |
| drafts | id, account_id, revision, client_mutation_id, create_payload_hash, recipients, subject, plain_text, reply_headers, updated_at | unique(account_id, client_mutation_id) 保证创建幂等；更新使用乐观锁 |
| draft_attachments | draft_id, attachment_id, blob_id, file_name, media_type | 发送前校验完整性 |
| outbox | id, draft_id, revision, account_id, client_mutation_id, payload_hash, mime_blob_id, state, attempt_stage, sent_copy_state | unique(account_id, client_mutation_id) |
| ai_runs | id, kind, provider_id, model, state, context_refs, context_hash, input_revision, result, error_code | 原始完整提示词默认不另存 |
| mcp_grants | client_id, account_scope, action_scope, expires_at, revoked_at | 随时可撤销；不存邮箱凭证 |
| audit_events | id, time, actor_id, account_id, action, resource_id, outcome, request_id | 不记录正文、地址列表、OTP、密钥 |
| schema_migrations | version, checksum, applied_at | 已发布版本不可改写 |

Web 在 M5 增加单管理员认证、Session 与 TOTP 恢复码表；不为未来多租户预建租户系统。模型提供商配置由 `AiService` 管理，配置与密钥引用保存，凭证本身交给 `SecretStore`。

## 3. 事务与文件一致性

- 每条连接设置外键校验和 `busy_timeout`；数据库启用 WAL，初始采用 `synchronous=FULL`，尤其保护本地草稿与发送状态。性能调整需基准与 ADR。
- 网络、MIME 大文件解析和模型请求不能占用数据库事务；`rusqlite` 调用位于专用阻塞执行器，不阻塞 Tokio 调度线程。
- blob 先写受配额限制的临时文件、校验和持久化，再在同卷原子重命名；之后事务写引用。文件写成功而事务失败产生的孤儿由延迟 GC 清理，不能先提交数据库再补文件。
- 正文与对应全文索引同事务更新，避免搜索结果指向不存在的内容；索引是派生数据，可以重建。
- GC 不删除被草稿、outbox 或备份引用的 blob。磁盘满时不清理用户未发送草稿来给邮件缓存腾空间。

## 4. 缓存边界

头部持续保留到账号删除或显式清理；正文和接收附件使用 LRU 配额，初始合计 2 GiB，可配置。草稿、待发送 MIME 与用户固定的离线内容不参与自动淘汰，单独显示占用。缓存被淘汰后 `bodyState=missing`，同步状态不能仍宣称可离线阅读。

默认单次正文解析上限 20 MiB，附件单件上限 50 MiB，完整消息上限 100 MiB；提供商限制可能更低。超限返回明确错误或原始文件手动保存入口，不截断后假装完整。MIME 深度默认上限 30，附件不自动解压或执行。

## 5. 搜索语义

搜索对象是**已经缓存的内容**。仅同步了头部时，返回 `coverage.bodyComplete=false` 并在界面提示正文覆盖不足。过滤条件统一包含账号、文件夹、时间、未读、星标、附件标志；账号过滤必须在 SQL 查询中执行，不能先跨账号搜索再隐藏结果。

v0.1 采用 FTS5 trigram 对主题、地址显示文本和已缓存纯文本做字面子串检索，满足中英文混合文本的基本查找。查询按受限语法构造并绑定参数，不把用户输入直接当原始 MATCH 表达式或 SQL。普通文本 `query` 作为一个转义后的字面短语；高级布尔查询留待后续。

trigram 的全文匹配不支持小于 3 个 Unicode 字符的子串。[SQLite FTS5 文档](https://www.sqlite.org/fts5.html)。据此首版将非空关键词限制为至少 3 个 Unicode 字符，短词返回 `SEARCH_QUERY_TOO_SHORT` 并提示补充关键词；无关键词的结构化筛选不受影响。中文两字词支持列为 M1 可用性评审项，若增加其他索引需补基准与契约，不能隐式退回无界全表扫描。

默认每页 50 条，上限 100，按 `(receivedAt, emailId)` 稳定排序。全文输入上限 256 字符；前端防抖和请求取消。向量索引、云端 embedding 和“自然语言转 SQL”均不在首版中。

## 6. 迁移与恢复

迁移在任何同步任务启动前执行，校验版本和 checksum，每个迁移在支持的事务边界内完成。升级前创建一致性备份；不支持直接降级打开更高版本数据库，提示恢复兼容备份。迁移失败保持原数据可恢复，禁止删除数据库重建作为通用修复手段。

健康检查覆盖 `integrity_check`、外键校验、FTS 一致性和 blob 引用缺失。修复优先重建派生索引或重新获取可下载缓存，不损坏草稿、发送历史和凭证引用。

## 7. 备份协议

v0.1 以停机备份为正式路径：暂停并等待任务退出 → 关闭所有数据库连接 → 复制完整数据目录 → 输出版本、文件清单与校验和。不能只复制运行中的 `mailx.sqlite`，也不能把备份目录直接放回当前运行目录覆盖。

后续在线备份采用 SQLite Backup API 生成数据库快照，同时冻结 blob 删除并复制快照引用的不可变文件；两者都完成后才能标记备份成功。[SQLite Backup API](https://www.sqlite.org/backup.html)。

恢复到新目录，校验清单和 schema 兼容性后再切换。桌面系统凭据库不随数据目录备份，恢复后需重新授权；Web 主密钥须独立保管，丢失时只能重新配置凭证。每个发布版本至少演练一次“新目录恢复并读取草稿”的流程。
