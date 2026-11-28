/**
 * Words on the site's own pages. The guide's words live in docs/guides.
 * Write for everyone: no protocol names or jargon; details go to the guide.
 */
const strings = {
  siteName: 'MailX',
  description: '本地优先的 AI 原生邮件客户端，极速轻量，优雅聚合你的所有邮箱。适用于 macOS、Windows 和 Linux，免费开源。',
  nav: { home: '首页', download: '下载', docs: '文档', changelog: '更新日志', github: 'GitHub', darkMode: '深色模式' },
  home: {
    title: 'MailX：本地优先的 AI 原生邮件客户端',
    headline: '本地优先的 AI 原生邮件客户端',
    tagline: '极速轻量，优雅聚合你的所有邮箱。',
    download: '免费下载',
    docs: '使用指南',
    platforms: '适用于 macOS、Windows 和 Linux · 免费开源',
    screenshotAlt: 'MailX 主窗口：左侧是邮箱，中间是收件箱，右侧是 AI 助手',
    providersTitle: '你的邮箱，多半已经支持',
    providers: ['Gmail', 'iCloud 邮箱', 'Outlook', 'Microsoft 365', 'Exchange', 'QQ 邮箱', '163 邮箱', '126 邮箱', 'Yeah 邮箱', '139 邮箱', '新浪邮箱', '飞书邮箱', 'Lark 邮箱'],
    providersNote: '不在上面？大多数其他邮箱手动填写服务器信息也能添加。',
    featuresTitle: '把邮件这件事，做得简单一点',
    features: [
      { icon: 'inbox', title: '所有邮箱，一个收件箱', body: '工作的、私人的、学校的，加进来就能放在一起看，不用在几个应用之间来回切换。' },
      { icon: 'laptop', title: '邮件留在你的电脑上', body: '不用注册 MailX 账号，邮件不经过我们的服务器。没网的时候，也能翻已经下载的邮件。' },
      { icon: 'sparkle', title: '让 AI 帮你读、帮你写', body: '总结长邮件、起草回复、改写语气。用的是你自己选的 AI，第一次发送邮件内容前会先征求同意。' },
      { icon: 'bolt', title: '打开就快', body: '一打开就是已经下载好的邮件，不用等网络；有新邮件会及时提醒你。' },
      { icon: 'shield', title: '看信更安心', body: '邮件里的网络图片默认先询问再加载，邮件内容在隔离的环境中显示，碰不到应用本身。' },
      { icon: 'translate', title: '外文邮件一键翻译', body: '读到看不懂的邮件，就地翻译，原来的排版保持不变，随时切回原文。' },
    ],
    showcases: [
      { shot: 'now', title: '先看一眼，就知道今天怎么样', body: '“此刻”把要紧的放在一页：新邮件、未读、刚收到的验证码，点一下就能复制。', points: [], alt: 'MailX 的“此刻”页面：验证码、新邮件和近况' },
      { shot: 'reading', title: '列表和正文，一起看', body: '左边挑，右边读，不用来回跳转。喜欢更专注，也可以切换成单栏。', points: [], alt: 'MailX 阅读邮件：左侧是邮件列表，右侧是正文' },
      { shot: 'chat', title: '直接问你的邮件', body: '像聊天一样说出你想做的事，剩下的交给助手。', points: ['“今天有哪些邮件要回复？”跨所有邮箱帮你找出来', '起草回复，你看过、改过再发', '发信默认每次都问你，也不能删除邮件'], alt: 'MailX 的 AI 会话：列出需要回复的邮件并写好草稿' },
      { shot: 'compose', title: '话到嘴边，让 AI 帮你写好', body: '写下大意，让它展开、润色、换个语气，或者检查错字。', points: ['先看结果，满意再采用', '一次撤销就能回到原来的文字', '引用的原邮件不会被改动'], alt: 'MailX 写信窗口：AI 把一句话改写成完整的邮件，等待采用' },
    ],
    privacy: {
      title: '你的邮件，本来就该在你手里',
      points: ['邮件、草稿和附件保存在你的电脑上', '密码保存在系统自带的密码库里', '不收集你的使用数据'],
      link: '看看 MailX 会连接哪些地方',
    },
    faqTitle: '常见问题',
    faq: [
      { q: 'MailX 收费吗？', a: 'MailX 免费、开源，没有广告。如果使用 AI，需要你自己添加 AI 服务商，对方可能会收费；也可以使用运行在自己电脑上的模型。' },
      { q: '需要注册账号吗？', a: '不需要。直接添加你已有的邮箱就能用。' },
      { q: '我的邮件会被上传吗？', a: 'MailX 没有自己的邮件服务器。只有在你同意并使用 AI 或翻译时，相关内容才会发给你选择的服务。' },
      { q: '一定要用 AI 吗？', a: '不用。不添加 AI 服务商，AI 就不会工作，其他功能不受影响。' },
      { q: '怎么更新？', a: 'macOS、Windows 和 Linux 的 AppImage 版可以在应用内更新。deb 和 rpm 版会提醒你，需要手动安装新版本。' },
      { q: '手机上能用吗？', a: '目前还不支持，手机版在后续规划中。现在可以在 macOS、Windows 和 Linux 电脑上使用。' },
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
    others: '其他系统',
    macos: { name: 'macOS', note: '已通过 Apple 公证，下载后可以直接打开。', arm: 'Apple 芯片（M 系列）', intel: 'Intel 芯片', which: '不确定是哪种？点左上角苹果菜单 →“关于本机”，看“芯片”或“处理器”一栏。' },
    windows: { name: 'Windows', note: '适用于 64 位 Windows。第一次打开时如果出现蓝色提示，点“更多信息”→“仍要运行”。', x64: 'x64（Intel / AMD）', arm64: 'ARM64（骁龙等）', which: '不确定是哪种？打开“设置”→“系统”→“系统信息”，看“系统类型”一栏。' },
    linux: { name: 'Linux', note: 'AppImage 可以在应用内自动更新；deb、rpm 需要手动安装新版本。', ubuntuMinimum: 'Ubuntu 用户请使用 24.04 或更新版本。', x64: 'x64（Intel / AMD）', arm64: 'ARM64', appimage: 'AppImage', deb: 'deb（Debian / Ubuntu）', rpm: 'rpm（Fedora / openSUSE）' },
    fallback: '暂时无法读取最新版本，请前往 GitHub 下载。',
    allReleases: '全部版本',
    checksums: '校验文件（SHA-256）',
    updates: '安装后，MailX 会在有新版本时自动提醒你更新。',
  },
  changelog: {
    title: '更新日志',
    lead: 'MailX 每个版本的变化。',
    older: (count: number) => `更早的 ${count} 个版本`,
  },
  footer: { license: '基于 Apache-2.0 协议开源', privacy: '隐私' },
}

export type Strings = typeof strings
export default strings
