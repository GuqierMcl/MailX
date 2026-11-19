# AI 服务商与模型配置

状态：**Implemented（自动测试，另有一次细节未记录的维护者手测）** · 本文是内置 AI 的基础设施：用户连接一个或多个模型服务商，选一个默认模型。已实现配置、模型列表、连接测试、服务商图标和非流式 `complete` 原语；首个使用方是邮件翻译：用户把翻译服务选为“AI 模型”后，邮件主题与正文文字才会发给所选连接（见[邮件翻译](translation.md#10-ai-模型作为翻译来源已实现仅自动测试)）；不选就不会向模型发送邮件内容。自动测试只访问本机模拟服务；维护者手测报告没有问题，但没有记录平台与服务商，见[验证指引](../development/validation.md#ai-provider-validation)。

## 1. 概念

- **服务商连接**（`AiConnection`）：一个服务商 + 一把密钥 + 接口地址。可以有多个，同一服务商也可以连多次（例如两把不同的密钥）。
- **默认模型**（`AiModelRef`）：某个连接下的一个模型。各用途默认跟随它，也可以单独指定；目前只有翻译可以单独指定（`translationModel`），助手尚未实现。
- **目录**：应用内置常用服务商配置，[models.dev](https://models.dev) 提供更多服务商配置和模型预设。目录用于填表、标注模型信息和选择预设，**保存后的连接不随目录变化**。

## 2. 数据

AI 设置保存在设置文件的 `ai` 字段，只能通过下文的 AI 命令修改，通用的设置更新接口不接受它，也不会改动它。`ai` 不在生成的 `AppSettings` TypeScript 类型里，前端经 `ai_state` 读取。设置文件上限为 256 KiB（原 64 KiB）以容纳最多 20 个连接。设置文件中损坏的 `ai` 部分只丢弃损坏的那一项，其余保留。

```text
AiSettings {
  connections: AiConnection[]          // 最多 20 个
  defaultModel: AiModelRef | null
  translationModel: AiModelRef | null  // 翻译用的模型；null 表示跟随默认模型
  route: ServiceRoute                  // AI 专用代理；与翻译共用的路由类型（原名 TranslationRoute，JSON 不变）
}
AiConnection {
  id: string                           // core 生成的 UUID
  providerId: string                   // models.dev / 内置 id，自定义为 "custom"
  name: string                         // 显示名，1–64 字符
  protocol: "openaiCompatible" | "anthropic"
  baseUrl: string                      // 保存时规范化（去掉末尾 /）
  useProxy: boolean                    // 只有设置了 AI 代理时才有意义
}
AiModelRef {
  connectionId: string
  modelId: string
  name: string
  contextLength: number | null         // 来自 models.dev，未知为 null
  outputLength: number | null
}
```

- 删除连接时同时删除它的密钥；默认模型或翻译模型指向它时分别清空。读取设置时，指向不存在连接的默认模型或翻译模型同样清空。
- AI 服务（`AiService`）由设置、`SecretStore` 和数据目录构建，**不依赖邮件库**，邮件库打开失败时仍可使用。
- 密钥存 `SecretStore`：`mailx/ai/<connectionId>/api-key`；AI 代理密码：`mailx/ai/proxy-password`。没有任何接口返回密钥，只返回是否已设置。密钥必须是可打印 ASCII（它要放进 HTTP 头），最长 4096 字节；密钥可以为空（本机的 Ollama、LM Studio 不需要）。
- macOS 上所有密钥都在同一个钥匙串条目里（[凭据保险库](secret-vault.md)），增加连接不会增加授权弹窗。

## 3. 接口地址与网络

- 地址只接受 `http` 或 `https`，必须有主机，不能带账号、查询参数或片段，最长 2048 字符。**是否使用 http 由用户决定**，界面不拦截、不警告。
- 不跟随重定向（3xx 视为请求被拒绝）；https 总是校验证书，没有关闭开关。
- 连接的 `useProxy` 为真且设置了 AI 代理时走代理，否则直连。代理失败报错，**不回退直连**。
- models.dev 目录与图标的请求：设置了 AI 代理就走代理，否则直连。
- 目录与图标缓存在 `<数据目录>/ai-cache/`：`catalog.json`、`icons/<id>.svg`。
- 界面在连接行显示接口的主机名，让用户知道密钥发往哪里。

## 4. 协议

| 协议 | 模型列表 | 测试调用 | 认证 |
| --- | --- | --- | --- |
| `openaiCompatible` | `GET {baseUrl}/models` | `POST {baseUrl}/chat/completions` | `Authorization: Bearer <key>`（密钥为空时不带） |
| `anthropic` | `GET {baseUrl}/models?limit=1000` | `POST {baseUrl}/messages`（`max_tokens: 16`；`complete` 未指定时默认 1024） | `x-api-key: <key>`，`anthropic-version: 2023-06-01` |

- 两种协议的 `baseUrl` 都含版本段，例如 `https://api.openai.com/v1`、`https://api.anthropic.com/v1`。
- 测试调用发送固定的一句英文（不含任何邮件内容），非流式；返回 200 且结构正确即为可用，结果带耗时。
- core 内同时提供一个非流式的 `complete(system, user, maxOutput)` 原语，测试调用和后续翻译都用它。流式输出留给助手功能。

## 5. 模型列表

首次添加连接、默认模型、助手与翻译使用同一个选择器。在搜索框上方切换来源，不更改连接、协议或密钥：

- **接口模型（默认）**：请求当前连接的列表接口。使用目录中该服务商的同 ID 条目补充名称、上下文长度、输出长度和工具能力；已知不输出文本的模型不展示，判断依据是 models.dev 的 `type` 与输出模态。网关返回 `owned_by` 时，显示在连接名称旁并用于分组与搜索；同名但 ID 不同的条目保留，行内显示完整 ID。`owned_by` 只用于展示，不改写 ID，也不会用于路由。
- **预设目录**：单独浏览连接所属服务商的本机目录副本，不请求连接的接口或读取其密钥；接口不支持模型列表、列表为空或请求失败都不影响浏览。目录预设不代表当前密钥或服务地址可用，自定义连接没有已匹配的目录。目录刷新仍走 AI 目录路由，不附带模型服务密钥。
- 两个来源都支持手动输入模型 ID。选择后保存原连接 ID 与完整模型 ID，不补前缀、不改路由；用“测试”验证实际调用。

接口列表不持久化，每次打开选择器实时获取，首次连接时复用连接检查的结果。`404` / `405` 表示没有列表接口，允许保存连接但未验证密钥；其他错误保留错误提示，不用目录结果代替。只保存选中模型的快照（`AiModelRef`）。

## 6. 目录

- **内置常用服务商**：一张写在 core 里的小表（`core/src/ai/catalog.rs` 的 `BUILT_INS`），离线可用，地址以代码为准。另有“自定义”项（id `custom`），用户自选协议、填写地址。当前内容：

| id | 名称 | 协议 | 默认地址 |
| --- | --- | --- | --- |
| `openai` | OpenAI | `openaiCompatible` | `https://api.openai.com/v1` |
| `anthropic` | Anthropic | `anthropic` | `https://api.anthropic.com/v1` |
| `deepseek` | DeepSeek | `openaiCompatible` | `https://api.deepseek.com` |
| `openrouter` | OpenRouter | `openaiCompatible` | `https://openrouter.ai/api/v1` |
| `zhipuai` | Zhipu AI | `openaiCompatible` | `https://open.bigmodel.cn/api/paas/v4` |
| `alibaba-cn` | Alibaba (China) | `openaiCompatible` | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| `moonshotai-cn` | Moonshot AI (China) | `openaiCompatible` | `https://api.moonshot.cn/v1` |
| `siliconflow-cn` | SiliconFlow (China) | `openaiCompatible` | `https://api.siliconflow.cn/v1` |
| `volcengine` | Volcengine Ark | `openaiCompatible` | `https://ark.cn-beijing.volces.com/api/v3` |
| `minimax-cn` | MiniMax (China) | `anthropic` | `https://api.minimax.cn/anthropic/v1` |
| `ollama` | Ollama（本机，无需密钥） | `openaiCompatible` | `http://127.0.0.1:11434/v1` |
| `lmstudio` | LM Studio（本机，无需密钥） | `openaiCompatible` | `http://127.0.0.1:1234/v1` |

  这张表只内置服务商配置，不内置模型列表。Ollama 通过它的 OpenAI 兼容接口接入，没有单独的原生协议适配。
- **models.dev**：按需获取 `https://models.dev/api.json`（打开 AI 设置时，缓存超过 24 小时才在后台刷新；带 ETag）。只保留可用的服务商并裁剪字段后缓存到本机：
  - `npm` 为 `@ai-sdk/openai-compatible`、`@ai-sdk/openai` 的归为 `openaiCompatible`，`@ai-sdk/anthropic` 的归为 `anthropic`；其余协议不显示；非内置的 `@ai-sdk/openai` 服务商没有给出地址的丢弃；
  - 地址含模板变量（`${…}`）、需要多项凭证（`env` 多于一项）或缺少地址且不是内置服务商的不显示；
  - 模型只保留 id、名称、上下文长度、输出长度、是否支持工具调用；模型自带不同 `provider` 覆盖的跳过。
- 获取失败时继续使用旧缓存，`AiCatalog.refreshError` 带回固定错误码；没有缓存时只显示内置服务商和“自定义”。

## 7. 服务商图标

- 内置服务商的图标随前端打包（`frontend/src/assets/ai-providers/`，取自 models.dev 的图标集，来源记录在该目录 README）。
- 其他服务商在显示时由 core 获取 `https://models.dev/logos/<providerId>.svg`：`providerId` 只接受小写字母、数字和连字符；SVG 不超过 64 KB、根元素必须是 SVG 命名空间的 `<svg>`、不含 `script` 与 `foreignObject`，否则视为失败。成功结果缓存在本机 7 天。
- SVG **不栅格化**，以 data URL 交给前端，只作为 CSS 遮罩（`mask-image`）使用，颜色跟随主题；**绝不作为标记插入 DOM**。邮件里的远程 SVG 仍按[远程图片](remote-image-loading.md)规则栅格化，两者不共用此路径。
- 加载失败或“自定义”服务商显示名称首字母。

## 8. 应用接口

| 命令 | 作用 |
| --- | --- |
| `ai_state()` → `AiState` | 连接（各带 `hasKey`）、默认模型、代理路由、代理密码是否已设置 |
| `ai_save_connection(SaveAiConnection)` → `SavedAiConnection` | 新建（`id` 为 null）或修改连接，返回 `{ state, connectionId, models }`；`apiKey` 为 null 表示不改密钥，空串表示清除。`verify` 为真时先用这些参数获取模型列表，失败则返回错误且不保存（接口返回 `404` / `405` 除外）；未检查或不支持列表时 `models` 为 null |
| `ai_remove_connection({ id })` → `AiState` | 删除连接与密钥 |
| `ai_set_default_model({ model })` → `AiState` | 设置或清空默认模型；连接必须存在 |
| `ai_set_translation_model({ model })` → `AiState` | 设置或清空翻译模型（清空即跟随默认模型）；连接必须存在 |
| `ai_set_route({ route })` → `AiState` | 设置 AI 代理 |
| `ai_set_proxy_password({ password })` / `ai_remove_proxy_password()` → `AiState` | 代理密码 |
| `ai_catalog({ refresh })` → `AiCatalog` | 服务商目录（不含模型）、更新时间、是否过期与 `refreshError`；`refresh` 为真时联网刷新 |
| `ai_list_models({ connectionId, source })` → `AiModel[] \| null` | `source` 为 `provider` 时实时获取，接口返回 `404` / `405` 时结果为 null；为 `catalog` 时读取本机目录，无预设时返回空数组。见第 5 节 |
| `ai_test({ connectionId, modelId })` → `AiTestResult` | 测试调用，返回耗时 |
| `ai_provider_icon({ providerId })` → `AiProviderIcon \| null` | 见第 7 节 |

翻译选用 `ai` 来源时，宿主另从这里取得“翻译用的连接”：翻译模型，否则默认模型，及其连接与密钥（`translation_binding` 只读设置、不读密钥，`translation_target` 才读密钥）；都没有时为 `aiNotConfigured`。
## 9. 错误

服务商返回的原文不传给界面，只按状态码归类为固定错误码，界面按错误码显示中英文文案：

| code | 含义 |
| --- | --- |
| `aiKeyRejected` | 401 / 403：密钥无效或没有权限 |
| `aiQuotaExceeded` | 402：余额或额度不足 |
| `aiModelNotFound` | 测试调用 404：模型不存在 |
| `aiRateLimited` | 429：请求太频繁 |
| `aiRequestRejected` | 其他 3xx / 4xx |
| `aiUnavailable` | 5xx |
| `aiNetwork` / `aiProxy` / `aiTimeout` | 连不上、代理失败、超时 |
| `aiInvalidResponse` | 答案无法解析或超过大小上限 |
| `aiNotConfigured` | 连接不存在 |

超时：目录 30 秒，模型列表 15 秒，测试调用 60 秒，图标 10 秒，翻译调用 120 秒（`complete` 现在由调用方传入超时）。答案上限：目录 16 MiB，其余 4 MiB。

## 10. 验证

自动测试覆盖：翻译模型的保存、校验与随连接清理，翻译用连接的选择（翻译模型优先于默认模型）；地址校验、两种协议的列表与测试调用（本机模拟服务）、错误码映射、目录裁剪与缓存、图标校验、密钥不出现在调试输出与错误中、删除连接清理密钥与默认模型。范围与未验证项见[验证指引](../development/validation.md#ai-provider-validation)；真实服务商、models.dev 与图标主机尚未调用过。

## 11. 尚未实现

流式输出与 `ModelProvider` 接口、助手用途的模型选择、通用的内容披露与取消（翻译的同意与取消由翻译自己处理）、Ollama 原生协议；见 [AI Runtime](../ai-runtime/README.md)。
