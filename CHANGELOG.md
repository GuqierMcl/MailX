# 更新日志

本文件记录 MailX 面向用户的变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

- 面向用户的改动，在 PR 里把一行写到 `## [Unreleased]` 之下，分到 `### Feature`、`### Optimization`、`### Fixed`、`### Security`、`### Changed` 之一（没有内容的分类不写），每条以 `- ` 开头。
- 条目写给用户看，一行一句；测试、CI、重构、文档等内部改动不写。结尾附带 PR 链接，例如 `- 修复远程图片加载失败后无法重试 ([#25](https://github.com/GuqierMcl/MailX/pull/25))`。
- `Changed` 只放不兼容的行为、数据格式、配置变化与移除的功能（含数据迁移）；行为不变的改进放 `Optimization`。
- 预发布版（alpha / beta / rc）不单独成节，其发布说明取自 `[Unreleased]`；正式版由 `bun run version:bump` 把 `[Unreleased]` 归并为带日期的小节。
- 发版步骤见 [docs/development/release.md](docs/development/release.md)。

## [Unreleased]

### Feature

- 搜索现在会查找邮件正文和附件名，不只是主题和发件人；结果里会高亮命中的那段文字。搜索在本机完成，已打开过的邮件正文会被收进本地索引。 ([#67](https://github.com/GuqierMcl/MailX/pull/67))
- 刚发出、服务器上还没有副本的邮件也会出现在“已发送”里，可以阅读、回复、转发或从本机移除；已发送列表改为显示收件人。 ([#65](https://github.com/GuqierMcl/MailX/pull/65))
- 写信时可以在正文中插入图片：点工具栏按钮选择图片，或直接粘贴截图；图片随邮件一起发出，收件人打开时不会向外部加载任何内容。 ([#62](https://github.com/GuqierMcl/MailX/pull/62))
- 邮件列表的预览随行滚动到可视区域附近时自动加载，加载期间显示（加载中）；预览使用部分获取，不会把邮件标为已读。 ([#59](https://github.com/GuqierMcl/MailX/pull/59))
- 新增 139 邮箱添加入口，显示服务商图标、预填加密收发服务器，并提供授权码开通、90 天有效期与账号资格说明。 ([#55](https://github.com/GuqierMcl/MailX/pull/55))
- 写信窗口可添加和移除附件：用回形针按钮选择文件，或把文件拖入窗口；附件栏显示文件名与大小，超出总大小或数量上限时给出提示，已进入发送队列的草稿只读；转发邮件时自动带上原邮件的附件；服务器因邮件过大拒收时给出明确提示。 ([#49](https://github.com/GuqierMcl/MailX/pull/49))
- 关闭主窗口时可选择后台运行或退出，默认每次询问，可记住选择并在通用设置修改；支持 Windows/Linux 系统托盘与可选的 macOS 菜单栏入口。 ([#48](https://github.com/GuqierMcl/MailX/pull/48))
- 添加邮箱时可在保存前测试当前收信与发信登录，支持取消、修改后重测，也可跳过测试继续添加。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 新增网易 163 / 126 / yeah.net 邮箱添加入口，显示各自的服务商图标，预填加密收发服务器并提供客户端授权码指引。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- macOS 主窗口与写信窗口的侧栏与标题栏使用系统毛玻璃材质（跟随应用主题）。 ([#43](https://github.com/GuqierMcl/MailX/pull/43))
- 邮箱列表内容超出时，上下边缘渐隐，提示还有更多内容。 ([#43](https://github.com/GuqierMcl/MailX/pull/43))
- 账号管理支持移除本机账号，清理本地邮件、草稿和发件记录；发送中保护、凭证清理重试和写信窗口联动避免误发或恢复已移除账号。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- QQ 邮箱 / Foxmail 添加入口：支持 @qq.com、@vip.qq.com 和 @foxmail.com，显示 QQ 邮箱图标，预填收发服务器并提供授权码指引；账号设置支持分别验证收信与发信登录。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- 本地优先的桌面邮件工作台预览：经账号路由登录 IMAP，同步文件夹与邮件头，提供统一收件箱、本地头部搜索与分页。
- 按需下载并缓存邮件正文，在隔离的阅读视图中显示；支持已读 / 星标写回、附件保存与新邮件通知。
- 富文本写信（Tiptap、本地草稿、回复与转发、独立写信窗口），经发送队列通过 SMTP 发信。
- 中英文界面与语言设置：支持跟随系统，主窗口与写信窗口同步切换，新通知及新建回复 / 转发的引用使用所选语言。([#29](https://github.com/GuqierMcl/MailX/pull/29))
- 应用内更新：启动约 30 秒后和之后每 6 小时检查一次新版本（可在设置中关闭自动检查）；发现新版本时标题栏出现提示，弹窗里可以阅读更新说明、下载并立即重启安装，或选择下次退出时安装。链接一律用系统浏览器打开。

### Optimization

- 通过 QQ、网易和 139 等服务商入口添加邮箱时默认不设置标识颜色，手动 IMAP/SMTP 仍自动分配颜色，各入口均可自行选色。 ([#66](https://github.com/GuqierMcl/MailX/pull/66))
- 邮箱服务商图标改用官方原始素材，保留完整图形与原有配色，QQ 邮箱使用专属邮箱图标。 ([#64](https://github.com/GuqierMcl/MailX/pull/64))
- 重启后头像优先使用本地缓存，不再先联网查询；确认没有头像的发件人 24 小时内不再重复查找。 ([#60](https://github.com/GuqierMcl/MailX/pull/60))
- 邮件列表改用虚拟滚动，邮件很多时滚动更流畅；滚动到接近列表末尾时自动加载更多，加载失败仍可点击“加载更多”重试；列表按“预览行数”设置为预览区预留固定高度，每行等高，滚动时不再跳动。 ([#58](https://github.com/GuqierMcl/MailX/pull/58))
- 设置弹窗左下角显示 MailX 与当前版本号，切换设置页时始终可见。 ([#47](https://github.com/GuqierMcl/MailX/pull/47))
- 标识颜色提供叉号“不标识”选项，选择后隐藏颜色圆环，服务商图标恢复完整大小。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 账号管理的强调色添加按钮显示“添加账号”，不再因打开弹窗自动出现提示；使用服务商图标的账号头像保留透明背景，账号颜色仅显示为细圆环，并增加图标与圆环之间的留白。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 163 / 126 / yeah.net 使用各自的服务商图标。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 账号管理的添加按钮移至列表右上角并使用强调色；已选择服务商的账号默认使用服务商图标，可在每个账号的资料中单独关闭。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- 添加账号和修改连接时，IMAP / SMTP 登录用户名留空会使用邮箱地址。 ([#40](https://github.com/GuqierMcl/MailX/pull/40))
- 添加账号的服务商选择改为 Logo、名称与短描述的单行列表，支持按名称、域名和关键词搜索。 ([#40](https://github.com/GuqierMcl/MailX/pull/40))
- 设置窗口默认打开第一项“通用”，便于调整语言。([#29](https://github.com/GuqierMcl/MailX/pull/29))
- macOS 上所有账号密码合并存放在一个钥匙串条目中，启动时最多只需授权一次，不再每个账号一次；旧条目会在首次启动时迁入、下次启动时清理。

### Fixed

- 修复写信窗口中斜体不显示倾斜的问题：界面字体本身没有斜体字形，之前被禁止合成，现在只在正文里允许合成斜体。 ([#53](https://github.com/GuqierMcl/MailX/issues/53))
- 退出时等待草稿保存和正在进行的发送，保存失败或等待超时会取消退出，避免强制关窗丢失未保存内容。 ([#48](https://github.com/GuqierMcl/MailX/pull/48))
- 修复下载更新时更新说明被错误替换为“没有提供更新说明”的问题，下载完成和失败重试时也持续显示。 ([#46](https://github.com/GuqierMcl/MailX/pull/46))
- 邮箱服务商明确以安全策略拒绝访问时显示独立原因，停止自动重试；处理后可手动同步恢复，保留本地缓存与待写标记。 ([#44](https://github.com/GuqierMcl/MailX/pull/44))
- 没有文件名的附件在英文界面下也按所选语言显示占位名称，不再固定显示中文。([#32](https://github.com/GuqierMcl/MailX/pull/32))

### Changed

- 本地数据库升级到新版本：待发送与已发送邮件改存为独立文件，首次启动时自动迁移，迁移后旧版本无法再打开该数据库；已发送邮件在服务器的已发送文件夹出现同一封后会释放本地副本。 ([#49](https://github.com/GuqierMcl/MailX/pull/49))
- 从 0.1.0-alpha.4 升级时，本地数据库自动从 v10 升级到 v12，以支持移除账号和服务商图标偏好；旧版无法直接打开升级后的数据，如需降级，请使用升级前的数据备份。 ([#42](https://github.com/GuqierMcl/MailX/pull/42))
- 下载页的安装包文件名改为按系统命名（如 `MailX_<版本>_macos-arm64.dmg`、`MailX_<版本>_windows-x64-setup.exe`），便于直接按操作系统选择。
- 0.1.0-alpha.1、0.1.0-alpha.2 不含应用内更新，需要手动下载安装带更新器的版本一次；自 0.1.0-alpha.3 起支持应用内更新。
- 自 0.1.0-alpha.2 起，应用标识为 `com.mailxapp.desktop`，用户数据目录为 `MailX`（macOS：`~/Library/Application Support/MailX`，Windows：`%APPDATA%\MailX`，Linux：`~/.local/share/MailX`），系统凭据条目使用服务名 `MailX`。从 alpha.1 升级时，旧目录中的数据和旧凭据不会自动迁移，需要迁移数据并重新输入各账号的密码或授权码；已使用 alpha.2 或更新版本的用户无需重复迁移。

[Unreleased]: https://github.com/GuqierMcl/MailX/commits/main
