# 验证码识别

状态：**core 本地识别与开发探测入口已实现；新邮件通知接入为后续阶段**。这是规则识别实验，尚无真实邮件语料上的准确率或覆盖率结论。测试证据见[验证记录](../development/validation.md#verification-code-validation)。

## 1. 分阶段边界

第一阶段提供宿主无关的纯函数、合成样本测试与本地命令行探测。函数不访问账号、数据库或网络，不获取正文、不写日志，不发通知或操作剪贴板；没有新增设置、Tauri IPC 或 MCP 工具。

第二阶段计划识别新到达邮件，并在原生通知中单独显示验证码：Windows / macOS 提供“复制验证码”按钮，只有用户点击才复制；Linux 在标题或正文首行突出验证码，不要求复制操作，不提高系统紧急级别。Windows 通知中心激活、macOS 通知后端、正文获取预算、通知去重与隐藏预览规则须在接入阶段实现并分别验收。自动复制不在此方向内。

## 2. core 接口

模块：`mailx_core::mail::verification_code`。

| 接口 | 输入与结果 |
| --- | --- |
| `detect(subject, body)` | 已解码的主题和纯文本正文，返回 `Result<Detection, DetectionError>` |
| `detect_eml(raw)` | 完整的 RFC 5322 / MIME 字节，解析主题和正文后执行同一检测器；不适用于部分 IMAP 预览 |
| `Detection::Found` | 唯一 `Candidate { code, evidence }` |
| `Detection::Ambiguous` | 多个不同候选，不选择其中一个作为答案 |
| `Detection::NotFound` | 没有满足规则的候选，不代表邮件一定不含验证码 |
| `DetectionError` | `InputTooLarge` 表示超过处理预算；`InvalidMessage` 表示 MIME 不受支持或结构 / 编码无效 |

`code` 保留原有前导零与字母大小写。同码的多处出现合并，大小写不同按不同候选处理。`evidence` 仅携带来源（主题 / 正文）、规则和固定关键词，不携带周围原文；规则区分关键词在码前、在码后，以及主题关键词与正文开头的独立数字行。

类型提供显式 JSON 序列化供本地探测使用，输出状态为 `found`、`ambiguous`、`notFound`。候选的 `Debug` 对码值脱敏。序列化不等于新增远程读取权限；后续宿主仍需独立定义数据披露与动作授权。

## 3. 首批规则

- 接受连续的 4–8 位 ASCII 数字，或同长度且同时含 ASCII 字母和数字的码；全字母、全角、空格分组与带连字符的码暂不支持。
- 要求局部明确语境，例如“验证码 / 驗證碼 / 校验码 / 动态密码”、`verification code`、`security code`、`login code`、`one-time password`、`OTP`，或码后紧邻 `to sign in`、`to verify your email` 等明确用法。
- 关键词与码之间只接受短连接语和标点，例如 `is`、`your`、“为”“是”，不以距离近为唯一依据。因此 `verification code expires in 2026`、`verification code: order number 123456` 不匹配。
- 主题有明确验证码语境时，可接受正文开头独立一行的数字，或正文中的 `code: …`；不把任意段落中的独立数字提升为验证码。
- 同一语境列举多个不同码（如 `123456 or 654321`）、主题与正文码冲突时返回歧义。重复出现的同一码仍是唯一候选。
- 排除链接 / 邮箱地址 token 内的数字，拒绝从长字符串、分组数字、电话、日期、小数和带不可见连接符的数字中截取片段；金额符号、百分比和明确旧码语境也会排除。
- 回复 / 转发前缀的主题不参与检测。纯文本正文跳过 `>` 引用行，在常见的 `On … wrote:`、原始邮件或转发分隔行处停止。新写正文仍可被识别。

这是保守的有限规则，不是自然语言理解：未列出的语言、换行方式、引用分隔符、带品牌名的连接语可能漏识别；识别结果不证明发件人身份、验证码有效期，也不代表用户正在进行相应登录。

## 4. MIME 与预算

MIME 路径复用现有 `mail-parser` 解码、渲染模块的结构 / 编码检查、HTML 净化与文本转换。附件与内嵌的其他邮件不作为识别正文。非空 `text/plain` 优先；没有可用纯文本时才从 HTML 提取文本。因此相互不一致的纯文本 / HTML alternative 以纯文本为准，不声称两种表示一致。

HTML 提取前移除 `blockquote`、常见引用容器、脚本 / 样式 / 模板及明确隐藏内容（`hidden`、`aria-hidden`、内联 `display:none` / `visibility:hidden` / `mso-hide:all`）。移除时保留边界，避免把两段数字拼接成新码。HTML 实体、编码和正常行内文本拼接由现有解析流程处理。它不运行浏览器布局、不求值任意 CSS，也不加载远程图片或样式表，不能保证识别所有视觉隐藏内容。

| 预算 | 上限与处理 |
| --- | --- |
| 原始 MIME | 1 MiB，在解析前检查 |
| 主题 | 4 KiB，解码后检查 |
| 纯文本 / HTML 源表示 | 各 64 KiB，选用前均检查；HTML 转文本后再次检查文本上限 |
| 不同候选 | 最多 16 个，超过时失败 |

超过预算时拒绝本次检测，不扫描截断前缀后声称找到唯一答案。字节限制约束输入规模，不是解析器的精确内存峰值保证。检测器按有界上下文扫描，网络地址范围只预处理一次。

## 5. 本地模拟探测

在仓库根目录运行，使用提交的合成样本：

```sh
bun run otp:probe --eml tests/fixtures/verification-code/sample.eml
```

该样本包括当前验证码、HTML 编码与引用中的旧码。预期结果为 `found`，码为 `482913`，旧引用码不进入候选。

也可仅提供主题，或提供主题与 UTF-8 文本文件；文件路径 `-` 表示从 stdin 读取：

```sh
bun run otp:probe --subject 'Your verification code: 001234'
bun run otp:probe --subject 'Your verification code' --body-file /path/to/synthetic-body.txt
bun run otp:probe --eml -
bun run otp:probe --help
```

无需 Bun 时使用 `cargo run -p mailx-core --locked --example verification_code_probe --` 加相同参数。JSON 输出到 stdout，包含码值；只对明确提供的文件或 stdin 进行读取，错误输出不附带路径或邮件原文。`found`、`ambiguous`、`notFound` 均是成功运行，参数、输入或输出失败返回非零退出码。分享输出和提交样本时只使用合成或脱敏数据，不提交真实邮件。

批量回归覆盖见 `core/src/mail/verification_code/tests.rs` 和 `core/tests/verification_code_mime.rs`，执行方式：

```sh
cargo test -p mailx-core --lib mail::verification_code --locked
cargo test -p mailx-core --test verification_code_mime --locked
```

模拟测试通过只能证明这些样本上的规则行为。接入通知前仍需扩充脱敏模板，分别检查误识别、漏识别、歧义和 MIME 提取结果。
