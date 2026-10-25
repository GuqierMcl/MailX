# 更新日志

本文件记录 MailX 面向用户的变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

- 面向用户的改动，在 PR 里把一行写到 `## [Unreleased]` 之下，分到 `### Feature`、`### Optimization`、`### Fixed`、`### Security`、`### Changed` 之一（没有内容的分类不写），每条以 `- ` 开头。
- 条目写给用户看，一行一句；测试、CI、重构、文档等内部改动不写。结尾附带 PR 链接，例如 `- 修复远程图片加载失败后无法重试 ([#25](https://github.com/GuqierMcl/MailX/pull/25))`。
- `Changed` 只放不兼容的行为、数据格式、配置变化与移除的功能（含数据迁移）；行为不变的改进放 `Optimization`。
- 预发布版（alpha / beta / rc）不单独成节，其发布说明取自 `[Unreleased]`；正式版由 `bun run version:bump` 把 `[Unreleased]` 归并为带日期的小节。
- 发版步骤见 [docs/development/release.md](docs/development/release.md)。

## [Unreleased]

### Feature

- 本地优先的桌面邮件工作台预览：经账号路由登录 IMAP，同步文件夹与邮件头，提供统一收件箱、本地头部搜索与分页。
- 按需下载并缓存邮件正文，在隔离的阅读视图中显示；支持已读 / 星标写回、附件保存与新邮件通知。
- 富文本写信（Tiptap、本地草稿、回复与转发、独立写信窗口），经发送队列通过 SMTP 发信。
- 中英文界面与语言设置：支持跟随系统，主窗口与写信窗口同步切换，新通知及新建回复 / 转发的引用使用所选语言。([#29](https://github.com/GuqierMcl/MailX/pull/29))

### Optimization

- 设置窗口默认打开第一项“通用”，便于调整语言。([#29](https://github.com/GuqierMcl/MailX/pull/29))

### Optimization

- macOS 上所有账号密码合并存放在一个钥匙串条目中，启动时最多只需授权一次，不再每个账号一次；旧条目会在首次启动时迁入、下次启动时清理。

### Fixed

- 没有文件名的附件在英文界面下也按所选语言显示占位名称，不再固定显示中文。([#32](https://github.com/GuqierMcl/MailX/pull/32))

### Changed

- 应用标识改为 `com.mailxapp.desktop`，用户数据目录改为 `MailX`（macOS：`~/Library/Application Support/MailX`，Windows：`%APPDATA%\MailX`，Linux：`~/.local/share/MailX`），系统凭据条目改用服务名 `MailX`。此前版本的数据留在旧目录、旧凭据不会自动迁移：需要把旧目录里的内容移到新目录，并重新输入各账号的密码或授权码。

[Unreleased]: https://github.com/GuqierMcl/MailX/commits/main
