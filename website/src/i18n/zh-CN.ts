/**
 * Words on the site's own pages. The guide's words live in docs/guides.
 * Write for everyone: no protocol names or jargon; details go to the guide.
 */
const strings = {
  siteName: 'MailX',
  description: '本地优先的 AI 原生邮件客户端，极速轻量，优雅聚合你的所有邮箱。适用于 macOS、Windows 和 Linux，免费开源。',
  nav: { download: '下载', docs: '文档', changelog: '更新日志', github: 'GitHub' },
  home: {
    title: 'MailX：本地优先的 AI 原生邮件客户端',
    headline: '本地优先的 AI 原生邮件客户端',
    tagline: '极速轻量，优雅聚合你的所有邮箱。',
    download: '免费下载',
    docs: '使用指南',
    platforms: '适用于 macOS、Windows 和 Linux · 免费开源',
    screenshotAlt: 'MailX 主窗口：左侧是邮箱，中间是收件箱，右侧是 AI 助手',
    featuresTitle: '把邮件这件事，做得简单一点',
    features: [
      { title: '所有邮箱，一个收件箱', body: 'Gmail、iCloud、Outlook、QQ、163……加进来就能放在一起看，不用在几个应用之间来回切换。' },
      { title: '邮件留在你的电脑上', body: '不用注册 MailX 账号，邮件不经过我们的服务器。打开就能看，没网的时候也能翻已经下载的邮件。' },
      { title: '让 AI 帮你读、帮你写', body: '问一句“今天有哪些要回复”，或者让它总结长邮件、起草回复。用的是你自己选的 AI，发出去之前先问过你。' },
      { title: '打开就快', body: '一打开就是已经下载好的邮件，不用等加载；新邮件一到就提醒你。' },
      { title: '看信更安心', body: '邮件里的图片默认不加载，邮件内容在隔离的环境中显示，碰不到应用本身。' },
      { title: '外文邮件一键翻译', body: '读到看不懂的邮件，一键翻成中文，原来的排版保持不变。' },
    ],
    closingTitle: '现在就试试 MailX',
    closingBody: '免费、开源，没有广告，也不收集你的使用数据。',
  },
  download: {
    title: '下载 MailX',
    lead: '选择你的系统。下载后打开安装包，按提示完成安装即可。',
    version: (version: string, date: string) => `最新版本 ${version} · ${date} 发布`,
    releaseNotes: '这一版更新了什么',
    recommended: '适合你的电脑',
    macos: { name: 'macOS', note: '已通过 Apple 公证，下载后可以直接打开。', arm: 'Apple 芯片（M 系列）', intel: 'Intel 芯片', which: '不确定是哪种？点左上角苹果菜单 →“关于本机”，看“芯片”或“处理器”一栏。' },
    windows: { name: 'Windows', note: '适用于 64 位 Windows。第一次打开时如果出现蓝色提示，点“更多信息”→“仍要运行”。', setup: '下载安装程序' },
    linux: { name: 'Linux', note: 'AppImage 可以在应用内自动更新；deb、rpm 需要手动安装新版本。', appimage: 'AppImage', deb: 'deb（Debian / Ubuntu）', rpm: 'rpm（Fedora / openSUSE）' },
    fallback: '暂时无法读取最新版本，请前往 GitHub 下载。',
    allReleases: '全部版本',
    checksums: '校验文件（SHA-256）',
    updates: '安装后，MailX 会在有新版本时自动提醒你更新。',
  },
  changelog: {
    title: '更新日志',
    lead: 'MailX 每个版本的变化。',
  },
  footer: { license: '基于 Apache-2.0 协议开源', privacy: '隐私' },
}

export type Strings = typeof strings
export default strings
