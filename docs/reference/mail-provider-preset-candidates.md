# 邮箱服务商预设候选

新浪免费邮箱、飞书邮箱（中国版）和 Lark 邮箱（国际版）已有独立添加入口，适用的账号范围见下表。预设存在不代表真实收发已经通过验收。其余服务商仍为候选，Fastmail 暂缓。

## 已采用的三个预设

| 服务商与首轮范围 | IMAP / SMTP（均为隐式 TLS） | 为什么先选、用户需要做什么 |
| --- | --- | --- |
| 新浪免费邮箱：`@sina.com`、`@sina.cn` | `.com`：`imap.sina.com:993` / `smtp.sina.com:465`；`.cn`：`imap.sina.cn:993` / `smtp.sina.cn:465` | 复用现有授权码流程即可覆盖两个国内个人邮箱域。网页端要同时开启 IMAP4/SMTP、生成客户端授权码；仅生成授权码不够。两个域分别指定、分别验证，不能混用端点，VIP 不在此轮。[服务器表](https://help.sina.com.cn/comquestiondetail/view/158/) · [授权码说明](https://help.sina.com.cn/comquestiondetail/view/1566/) |
| 飞书邮箱：中国版、飞书实际托管的成员邮箱 | `imap.feishu.cn:993` / `smtp.feishu.cn:465` | 管理员允许第三方客户端后，成员在飞书桌面端按设备生成专用密码。邮件组不能这样登录；在飞书里绑定外部邮箱也不表示由飞书托管。[飞书客户端配置](https://www.feishu.cn/hc/zh-CN/articles/902478147400-在第三方邮箱客户端登录飞书邮箱) |
| Lark 邮箱：国际版、Lark 实际托管的成员邮箱 | `imap.larksuite.com:993` / `smtp.larksuite.com:465` | 用 `lark` 独立于飞书的 `feishu`；不按公司域名自动分流。管理员放行第三方客户端后，成员在 Lark 桌面端生成设备专用密码。邮件组及仅绑定的外部邮箱不在范围内。[Lark 客户端配置](https://www.larksuite.com/hc/en-US/articles/378111206512-log-in-to-lark-mail-through-a-third-party-email-client) |

## 接着选哪家

| 候选 | 建议端点（IMAP / SMTP，隐式 TLS） | 决定排期的条件 |
| --- | --- | --- |
| 腾讯企业邮（企业微信邮箱） | `imap.exmail.qq.com:993` / `smtp.exmail.qq.com:465` | 先做境内普通成员邮箱，独立于个人 QQ / Foxmail 入口。用完整企业邮箱地址；开启安全登录时用客户端专用密码。腾讯另列境外主机，不在失败后自动切换。拿到可测试的组织账号时，可复用飞书入口的企业域处理。[腾讯云《腾讯企业邮》“邮件客户端配置”](https://main.qcloudimg.com/raw/document/product/pdf/613_46019_cn.pdf#page=25) |
| 阿里邮箱（企业通用版） | `imap.qiye.aliyun.com:993` / `smtp.qiye.aliyun.com:465` | 管理员需放行第三方客户端和相关协议，建议用三方客户端安全密码；组织强制时不能填网页密码。通用版与香港优化线路、信创版分别列出，不能套用到个人 `@aliyun.com`。有测试租户即可跟进。[端点与版本](https://help.aliyun.com/zh/document_detail/36576.html) · [管理员访问策略](https://help.aliyun.com/zh/document_detail/606337.html) |
| 中国电信 189 个人邮箱：`@189.cn` | `imap.189.cn:993` / `smtp.189.cn:465`（候选） | 可补足中国移动 139 以外的运营商邮箱。电信说明了主机、协议开关和专用密码；部分客户端示例要求登录名仅填 `@` 前部分，完整邮箱地址与协议用户名应分开测。993/465 有华为 WeLink 自身配置文档佐证，先用号码主邮箱确认收发与证书，再定默认值。不包含电信企业邮箱。[电信帮助](https://help.189.cn/client/client.html) · [WeLink 客户端示例，第 4.4 节](https://support.huaweicloud.com/faq-welink/faq-welink.pdf) |
| Yahoo Mail 国际版：先验 `@yahoo.com` | `imap.mail.yahoo.com:993` / `smtp.mail.yahoo.com:465` | 用户帮助给出完整邮箱地址和应用密码；先验证目标账号能实际生成密码并双向收发。官方浏览器登录是另一项工作：Yahoo 邮件 OAuth 权限需申请，不能在开发者控制台自行开通。Yahoo! JAPAN 使用不同主机与登录设置，不纳入国际版。[国际版配置](https://help.yahoo.com/kb/imap-settings-sln4075.html) · [开发者准入](https://senders.yahooinc.com/developer/developer-access/) · [日本版配置](https://support.yahoo-net.jp/PccMail/s/article/H000007321) |

**Fastmail（暂缓）**：不提供专用预设。后续重新评估时可从 `@fastmail.com` 主账号开始：官方端点为 `imap.fastmail.com:993` / `smtp.fastmail.com:465`，需有邮件权限的应用专用密码；Basic 套餐不支持第三方客户端。[服务器、登录名与套餐](https://www.fastmail.help/hc/en-us/articles/1500000278342-Server-names-and-ports)

另一个海外个人邮箱候选是 **GMX.com**：先在网页设置开启 POP3/IMAP 同步，可为 MailX 生成应用密码；IMAP 为 `imap.gmx.com:993`，SMTP 是 `mail.gmx.com:465`，不是 `smtp.gmx.com`。先限 `@gmx.com`，不把其他地区域名或 mail.com 视为同一预设。[GMX 端点](https://support.gmx.com/pop-imap/imap/server.html) · [协议开关](https://support.gmx.com/pop-imap/toggle.html) · [应用密码](https://support.gmx.com/security/2fa/application-specific-passwords.html)

**不按普通固定预设排期：** [Zoho Mail](https://www.zoho.com/mail/help/imap-access.html) 的准确端点依账号类型和数据中心而变，新注册免费套餐没有 IMAP，需先设计如何按账号选对端点；[Proton Mail](https://proton.me/support/imap-smtp-and-pop3-setup) 的桌面双向收发依赖付费套餐与本机 Bridge，不是填入两台公网服务器即可接入。

## 从候选到可用预设

- **个人域和托管域分开处理。** [新建账号校验](../../frontend/src/features/accounts/account-draft.ts)对个人邮箱按已登记后缀匹配；飞书和 Lark 允许用户在明确选择托管商后填写企业成员地址，但各自只预填本服务的固定主机。不从公司域拼接服务器地址，不向其他主机试密码，不覆盖已保存的配置与路由。
- **先用手动入口跑通一封往返邮件。** 每个拟宣称支持的域、套餐或租户类型都应有真实账号：核对收发两端登录、文件夹与正文、已读或星标写回；发送到独立测试收件箱，确认送达并回复，再检查 MailX 是否收到回复、已发送副本是否重复。另测支持时的 IDLE 或轮询、凭证撤销或管理员禁用后的提示。具体条件见[验证指引](../development/validation.md)，结果记当次 PR，不以“测试连接”代替投递。
- **没有证据就不加品牌特判。** 初期沿用[标准兼容策略](../architecture/provider-compatibility.md)；图标按[服务商图标规范](../../frontend/src/assets/providers/README.md)取官方原件，找不到合适原件时用通用邮件图标和名称。
