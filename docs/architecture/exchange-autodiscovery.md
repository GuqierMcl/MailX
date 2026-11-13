# Exchange 自动发现与邮箱设置探测

状态：**已实现**。本文说明添加账号时的一次性只读探测：服务商列表里的通用「IMAP/SMTP」入口（`GENERIC_PROVIDER`）的连接步骤先只显示邮箱地址、密码与折叠的网络路由，「下一步」由 `core/src/discovery/` 尝试发现邮箱设置，并按**推荐协议**登录——发现可用 EWS 端点时优先 Exchange(EWS)，否则 IMAP / SMTP——登录验证通过才进入下一步（保存）；登录失败或探测无结果转手工表单（保留预填值与失败报告），「手工设置」进入完整表单并可用探测结果预填（详见 1 与 4 节）。实现与边界以代码为准，真实 Exchange 服务器与三平台的实测范围见[验证记录](../development/validation.md)；相关命令契约见[应用 API](../contracts/app-api.md)，账号与凭证语义见[账号配置](account-configuration.md)，EWS 后端见 [EWS 邮件后端](ews-backend.md)。

## 1. 定位与边界

自动探测只发生在**添加账号**流程，且只在通用入口进行：

- 它是添加账号前的一次性只读发现，不创建账号、不写凭证、不改动已保存的连接，也不启动同步或发信。
- 探测结果只用于填充表单或选择通道（IMAP / SMTP 或 EWS）；它不改变[服务商兼容层](provider-compatibility.md)的策略，不凭邮箱后缀或 UI 图标启用品牌分支，也不改变已保存账号的路由与兼容规则。
- **编辑已保存账号不自动探测**：已有账号的连接按原配置读写，IMAP / SMTP 账号的行为零改动。
- 连接步骤先只显示邮箱地址、密码与折叠的网络路由；「下一步」在探测完成后按推荐协议登录——发现可用 EWS 端点（HTTP/NTLM）时优先 EWS，否则 IMAP / SMTP——登录验证通过才进入下一步（保存）。「手工设置」进入完整表单，已填邮箱地址则自动探测并用结果预填（Exchange 端点或 IMAP/SMTP 主机/端口/加密）；登录失败或探测无结果（需要密码、被拒绝、网络 / TLS / 超时、无结果、不可用）自动转手工填写表单，保留已预填的值与失败报告，可修正后再次提交。

探测本身不是登录验证，也不证明邮件可收发；「下一步」在探测之后另行完成登录验证（`verifyDraftConnection`，对带 `ews` 的连接为一次 NTLM 握手加 `FindFolder`，同一结果填入收发两个协议槽，见[应用 API](../contracts/app-api.md)），保存账号后仍沿用现有的连接与凭证信任规则。

## 2. 探测范围

### 2.1 Exchange Autodiscover（POX）

`core/src/discovery/` 实现 Exchange Autodiscover POX 客户端，约束如下：

- **主机来源**：只向邮箱地址域名派生的主机发请求（例如 `example.com` 对应的 Autodiscover 主机），不向调用方提供的任意 URL 发请求，也不跟随到无关域。
- **仅 HTTPS**：所有请求走 HTTPS，证书按系统信任库校验，不提供「忽略证书错误」的开关。
- **可选 Basic 凭据**：请求可带 Basic 凭据（密码由本次输入提供、不入库）；不带凭据时也可进行匿名探测。
- **有界**：最多 2 跳 redirect，整体 20 秒总预算；超时或超跳即结束，不无限重试。
- **解析器**：沿用有界扫描器，按本地名匹配元素，**拒绝 DTD**（不做实体展开），不解析外部实体。
- **错误分类**：服务端错误映射为固定枚举，不把服务端原文、XML 片段或内部地址回传界面或写入普通日志。
- **路由**：走账号所选路由（`direct` / `inherit` / `custom`）；自定义代理失败返回 `proxy`，**不回退直连**，与既有协议一致。

除 IMAP / SMTP 协议设置外，解析 `EXCH` / `EXPR` / `EXHTTP` / `HTTP` / `WEB`（Outlook Anywhere / RPC over HTTP / EWS）节点，读取：

| 字段 | 用途 |
| --- | --- |
| `EwsUrl` / `ASUrl` / `OOFUrl` / `Server` | EWS 或外部端点地址、服务器名 |
| `<SSL>On</SSL>` | 是否要求 HTTPS |
| `AuthPackage` | 认证方式，识别 `NTLM` / `Negotiate` / `Basic` |
| `CertPrinName` | 证书主体名，形如 `msstd:mail.example.com` |
| `LoginName` | 登录名，形如 `DOMAIN\user` |

### 2.2 匿名 EWS 回退探测

Autodiscover 无结果时，再对 `https://mail.<domain>` 与 `https://<domain>` 的 `/EWS/Exchange.asmx` 做**不带凭据**的匿名探测：

- 只看响应的 `WWW-Authenticate` 是否含 `NTLM` 或 `Negotiate`，据此判断是否存在可用的 EWS 端点；不尝试认证，不读取邮件内容，也不写入任何状态。
- 同样遵守 HTTPS-only、账号路由、自定义代理失败不回退直连与整体预算。

回退探测只用于「是否可能存在 EWS 通道」的判断，不足以确定完整连接参数；具体端点、认证方式与登录名仍以 Autodiscover 结果或用户手动填写为准。

## 3. 命令与返回

新增只读命令 `accounts_discover_mail_settings`（前端方法 `discoverMailSettings`）：

```ts
// 请求
{ request: { emailAddress, password: string | null, route: RouteInput } }

// 返回
type MailSettingsDiscovery = {
  status: 'found' | 'needsCredentials' | 'rejected' | 'notFound'
    | 'network' | 'proxy' | 'tls' | 'timeout' | 'unsupported';
  imap: /* 探测到的 IMAP 设置或空 */;
  smtp: /* 探测到的 SMTP 设置或空 */;
  ews:  /* 探测到的 EWS 端点信息或空 */;
  source: /* 结果来源（Autodiscover 或匿名 EWS 回退） */;
};
```

- `status` 为固定词表，不含服务端文本；`needsCredentials` 表示本次需要密码才能继续，`rejected` 表示被拒绝，`notFound` 表示没有可用结果，`network` / `proxy` / `tls` / `timeout` / `unsupported` 分别对应网络、代理、TLS、超时与不可用。
- 命令**只读**：不创建账号、不写凭证、不改动已存连接；重复以相同输入调用应得到一致结论，不产生副作用。
- `route` 遵循 `RouteInput` 的既有约束，探测与后续账号连接使用同一套路由；password 为可空字符串，`null` 表示匿名 / 无密码探测。
- 类型由 `cargo run -p mailx-core --example export_types` 生成到 `frontend/src/generated/accounts.ts`，与其余账号 DTO 同源。

命令与返回字段的完整语义、窗口授权与错误码见[应用 API](../contracts/app-api.md#3-当前账号操作)。

## 4. 与账号保存的关系

- 探测结果中的 IMAP / SMTP 或 EWS 信息用于生成账号配置；EWS 通道对应可空的 `ews` 连接（结构见[应用 API](../contracts/app-api.md)与[账号配置](account-configuration.md)）。
- EWS 端点在保存时加入凭证信任范围；URL 或登录名变更即不再信任，需重新输入密码确认，规则与既有端点信任一致。
- 探测本身不建立凭证信任、不推进账号 `revision`，也不产生 `connectionStatus` 结论。
- EWS 账号的收发、标记写回、正文缓存与发信分派见 [EWS 邮件后端](ews-backend.md)；本地映射与游标表见[数据库迁移](database-migrations.md)。

## 5. 安全约束

- 服务端响应、XML 与重定向目标都是不可信输入：拒绝 DTD 与外部实体，限制跳转与总预算，错误以固定枚举表达。
- 不向调用方指定的任意主机发送带凭据请求，主机只由邮箱域名派生；自定义代理失败不回退直连。
- 探测产生的端点、登录名与错误信息不含真实组织标识；验证记录只保留占位与脱敏结论，真实标识不入仓库（见[验证记录](../development/validation.md)）。
- 探测不读取凭证库、不写 SecretStore、不记录密码；前端只在本次提交期间短暂持有输入。

## 6. 后续方向

以下仍为后续方向，不属于当前实现：

- 用 `SyncFolderItems` 做 EWS 增量同步与删除同步（当前每轮 `FindItem` 取前 500 封）。
- EPA 通道绑定哈希、EWS 的 `Subscribe` / `GetEvents` 流式订阅与 EWS 附件降级。
- OAuth 用于 EWS、共享邮箱与世纪互联运营的 Microsoft 365。
- 真实 Exchange 服务器端到端收发与三平台打包验证，见[验证记录](../development/validation.md)。
