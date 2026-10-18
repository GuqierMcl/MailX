import type {
  Account,
  AccountConfigInput,
  EmailDetail,
  EmailSummary,
  Folder,
  FolderRole,
  MailAddress,
  RouteConfig,
} from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'

/**
 * In-memory client for designing the UI in a plain browser (`bun run dev`).
 * Data is synthetic and lost on reload; nothing touches the network or disk.
 */
export function createMockClient(): MailxClient {
  const accounts = MOCK_ACCOUNTS.map((account) => structuredClone(account))
  const emails = buildEmails()
  const folders = accounts.flatMap((account) => buildFolders(account.id, emails))

  const notFound = () => new MailxClientError(missing)

  return {
    available: true,
    getStartupStatus: async () => ({ state: 'ready', revision: 1 }),
    retryStartup: async () => ({ state: 'ready', revision: 1 }),
    listAccounts: async () => structuredClone(accounts),
    async getAccount(accountId) {
      const account = accounts.find((row) => row.id === accountId)
      if (!account) throw notFound()
      return structuredClone(account)
    },
    async createAccount({ config }) {
      const account = accountFromConfig(crypto.randomUUID(), 1, config)
      accounts.push(account)
      folders.push(...buildFolders(account.id, emails))
      return structuredClone(account)
    },
    async updateAccount({ accountId, expectedRevision, config }) {
      const index = accounts.findIndex((row) => row.id === accountId)
      if (index < 0) throw notFound()
      if (accounts[index].revision !== expectedRevision) {
        throw new MailxClientError({ code: 'conflict', message: '配置已被修改，请重新加载后重试。', field: null })
      }
      accounts[index] = { ...accountFromConfig(accountId, expectedRevision + 1, config), createdAt: accounts[index].createdAt }
      return structuredClone(accounts[index])
    },
    listFolders: async (accountId) => structuredClone(folders.filter((folder) => folder.accountId === accountId)),
    listSyncStates: async () =>
      accounts.map((account) => ({ accountId: account.id, phase: 'idle', lastSyncedAt: new Date().toISOString(), failure: null })),
    onSyncState: () => () => undefined,
    async listEmails(query) {
      const text = query.text?.trim().toLowerCase()
      const items = emails
        .map((email) => email.summary)
        .filter((email) =>
          (!query.accountIds || query.accountIds.includes(email.accountId)) &&
          (!query.folderId || email.folderId === query.folderId) &&
          (!query.folderRole || folders.some((folder) => folder.id === email.folderId && folder.role === query.folderRole)) &&
          (query.unread == null || email.isRead === !query.unread) &&
          (query.hasAttachment == null || email.hasAttachment === query.hasAttachment) &&
          (!text || [email.subject, email.preview, email.from.name ?? '', email.from.address].some((field) => field.toLowerCase().includes(text))),
        )
        .sort((a, b) => b.receivedAt.localeCompare(a.receivedAt) || b.id.localeCompare(a.id))
      const offset = query.cursor ? Number(query.cursor) : 0
      const limit = query.limit ?? 50
      const page = items.slice(offset, offset + limit)
      return {
        items: structuredClone(page),
        nextCursor: offset + limit < items.length ? String(offset + limit) : null,
        coverage: { headersComplete: true, bodiesComplete: false, syncedAfter: null, syncedBefore: null },
      }
    },
    async getEmail(emailId) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      return structuredClone(email)
    },
  }
}

const missing = { code: 'notFound', message: '内容不存在。', field: null } as const

const WORK = '11111111-1111-4111-8111-111111111111'
const PERSONAL = '22222222-2222-4222-8222-222222222222'

function accountFromConfig(id: string, revision: number, config: AccountConfigInput): Account {
  const now = new Date().toISOString()
  const route: RouteConfig =
    config.route.kind === 'custom'
      ? {
          ...config.route,
          authentication: config.route.authentication && {
            username: config.route.authentication.username,
            passwordConfigured: true,
            trust: 'trusted',
          },
        }
      : config.route
  const separate = config.authentication.kind === 'password' && config.authentication.mode === 'separate'
  const auth = (suffix: string) => ({ id: `${id}-${suffix}`, generation: 1, passwordConfigured: true, trust: 'trusted' as const })
  return {
    id,
    revision,
    displayName: config.displayName,
    emailAddress: config.emailAddress,
    providerId: config.providerId,
    mailboxSourceId: `mock-${id}`,
    imap: config.imap,
    smtp: config.smtp,
    authentication: separate
      ? { mode: 'separate', imap: auth('imap'), smtp: auth('smtp') }
      : { mode: 'shared', imap: auth('shared'), smtp: auth('shared') },
    route,
    routeRevision: 1,
    defaultRouteRevision: null,
    connectionStatus: 'unverified',
    createdAt: now,
    updatedAt: now,
  }
}

const MOCK_ACCOUNTS: Account[] = [
  accountFromConfig(WORK, 1, {
    displayName: '工作邮箱',
    emailAddress: 'lin.wei@example.com',
    providerId: null,
    imap: { host: 'imap.example.com', port: 993, tls: 'implicit', username: 'lin.wei@example.com' },
    smtp: { host: 'smtp.example.com', port: 465, tls: 'implicit', username: 'lin.wei@example.com' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    route: { kind: 'direct' },
  }),
  accountFromConfig(PERSONAL, 1, {
    displayName: '个人邮箱',
    emailAddress: 'linwei.home@example.org',
    providerId: null,
    imap: { host: 'imap.example.org', port: 993, tls: 'implicit', username: 'linwei.home@example.org' },
    smtp: { host: 'smtp.example.org', port: 587, tls: 'startTls', username: 'linwei.home@example.org' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    route: { kind: 'direct' },
  }),
]

const FOLDER_NAMES: [FolderRole, string][] = [
  ['inbox', '收件箱'],
  ['drafts', '草稿'],
  ['sent', '已发送'],
  ['archive', '归档'],
  ['junk', '垃圾邮件'],
  ['trash', '已删除'],
]

const folderId = (accountId: string, role: FolderRole) => `${accountId}:${role}`

/** User folders by account: `[path, name, parent path, selectable]`; `selectable: false` mimics IMAP `\\Noselect`. */
const CUSTOM_FOLDERS: Record<string, [string, string, string | null, boolean][]> = {
  [WORK]: [
    ['projects', '项目', null, true],
    ['projects/mailx', 'MailX', 'projects', true],
    ['projects/q3', '第三季度', 'projects', true],
    ['clients', '客户', null, false],
    ['clients/east', '华东区', 'clients', true],
  ],
  [PERSONAL]: [['travel', '旅行', null, true]],
}

function buildFolders(accountId: string, emails: EmailDetail[]): Folder[] {
  const custom = (CUSTOM_FOLDERS[accountId] ?? []).map(
    ([path, name, parent, selectable]) => [`${accountId}:${path}`, name, 'other', parent && `${accountId}:${parent}`, selectable] as const,
  )
  const system = FOLDER_NAMES.map(([role, name]) => [folderId(accountId, role), name, role, null, true] as const)
  return [...system, ...custom].map(([id, name, role, parentId, selectable]) => {
    const inFolder = emails.filter((email) => email.summary.folderId === id)
    return {
      id,
      accountId,
      parentId,
      name,
      role,
      selectable,
      unreadCount: inFolder.filter((email) => !email.summary.isRead).length,
      totalCount: inFolder.length,
    }
  })
}

type Seed = {
  account: string
  role?: FolderRole
  /** Path of a user folder from `CUSTOM_FOLDERS`; overrides `role`. */
  folder?: string
  from: MailAddress
  subject: string
  text: string
  hoursAgo: number
  read?: boolean
  starred?: boolean
  attachments?: [string, string, number][]
}

const ME_WORK = { name: '林伟', address: 'lin.wei@example.com' }
const ME_HOME = { name: '林伟', address: 'linwei.home@example.org' }

const SEEDS: Seed[] = [
  {
    account: WORK,
    from: { name: '陈晓', address: 'chen.xiao@example.com' },
    subject: '第三季度产品路线图评审',
    text: '大家好，\n\n周四下午 3 点在 5 楼会议室评审第三季度路线图，请提前看一下附件里的草稿，重点关注同步模块的排期。\n\n如果时间冲突请回复我。\n\n陈晓',
    hoursAgo: 0.4,
    attachments: [['Q3 路线图草稿.pdf', 'application/pdf', 1_284_000]],
  },
  {
    account: WORK,
    from: { name: 'GitHub', address: 'notifications@github.example' },
    subject: '[MailX] Pull request #1: feat(desktop): add native-feeling custom title bar',
    text: '有新的评论等待你回复。\n\n> 在 Windows 上关闭按钮的悬停颜色看起来正确，Linux 还需要测试。',
    hoursAgo: 1.5,
  },
  {
    account: WORK,
    from: { name: '王芳', address: 'wang.fang@example.com' },
    subject: '回复：发票报销流程变更',
    text: '收到，我下周一前把十月份的发票整理好交给财务。新流程里电子发票是不是不需要再打印纸质版了？',
    hoursAgo: 3,
    read: true,
    starred: true,
  },
  {
    account: WORK,
    from: { name: '运维通知', address: 'ops@example.com' },
    subject: '周六凌晨数据库维护窗口',
    text: '本周六 02:00–04:00 将对主数据库进行版本升级，期间内部系统只读。请提前安排好需要写入的任务。',
    hoursAgo: 9,
    read: true,
  },
  {
    account: WORK,
    from: { name: '李娜', address: 'li.na@example.com' },
    folder: 'projects/mailx',
    subject: '设计稿：邮件列表的紧凑模式',
    text: '附上紧凑模式的两版设计，A 版本每行显示两行摘要，B 版本只显示主题。你更倾向哪一版？',
    hoursAgo: 26,
    attachments: [
      ['紧凑模式-A.png', 'image/png', 412_000],
      ['紧凑模式-B.png', 'image/png', 398_000],
    ],
  },
  {
    account: WORK,
    from: { name: '张强', address: 'zhang.qiang@example.com' },
    folder: 'projects/mailx',
    subject: '关于 IMAP IDLE 的兼容性测试结果',
    text: '测试了 6 家服务商，其中 2 家在空闲 29 分钟后会主动断开 IDLE，需要我们提前重连。详细表格见附件。',
    hoursAgo: 50,
    read: true,
    attachments: [['IDLE 兼容性.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 86_000]],
  },
  {
    account: WORK,
    role: 'sent',
    from: ME_WORK,
    subject: '回复：第二季度复盘',
    text: '谢谢大家，复盘纪要已经整理到共享文档，行动项负责人请在周五前更新进度。',
    hoursAgo: 75,
    read: true,
  },
  {
    account: WORK,
    role: 'drafts',
    from: ME_WORK,
    subject: '（无主题）',
    text: '关于下个版本的发布计划，我想先确认一下……',
    hoursAgo: 4,
    read: true,
  },
  {
    account: WORK,
    role: 'archive',
    from: { name: '人力资源部', address: 'hr@example.com' },
    subject: '年度体检预约通知',
    text: '年度体检预约通道已开放，请在 11 月 15 日前完成预约。',
    hoursAgo: 240,
    read: true,
  },
  {
    account: PERSONAL,
    from: { name: '中国国际航空', address: 'service@airline.example' },
    folder: 'travel',
    subject: '您的行程单：北京 — 上海 11月2日',
    text: '尊敬的旅客，您的航班 CA1501 将于 11 月 2 日 08:00 起飞，请提前 90 分钟到达机场办理值机。',
    hoursAgo: 2,
    attachments: [['电子行程单.pdf', 'application/pdf', 152_000]],
  },
  {
    account: WORK,
    folder: 'clients/east',
    from: { name: '赵敏', address: 'zhao.min@client.example' },
    subject: '合同续签的几点意见',
    text: '附件是我们法务对续签合同的修改意见，主要集中在服务等级和数据存放地两条，方便的话周三电话沟通。',
    hoursAgo: 20,
  },
  {
    account: PERSONAL,
    from: { name: '妈妈', address: 'mom@example.net' },
    subject: '周末回家吃饭吗',
    text: '周末有空回来吃饭吧，你爸买了条鱼。记得带上上次说的那本书。',
    hoursAgo: 6,
  },
  {
    account: PERSONAL,
    from: { name: '读书会', address: 'club@example.org' },
    subject: '十一月书单投票',
    text: '本月候选书目：《三体》《置身事内》《人类简史》。点击下面的链接投票，周日截止。',
    hoursAgo: 30,
    read: true,
  },
  {
    account: PERSONAL,
    role: 'junk',
    from: { name: '中奖通知', address: 'winner@prize.example' },
    subject: '恭喜您获得 iPhone 一部！！！',
    text: '点击链接领取奖品。',
    hoursAgo: 12,
  },
  {
    account: PERSONAL,
    from: { name: '物业服务中心', address: 'service@property.example' },
    subject: '停水通知',
    text: '因管道检修，11 月 1 日 9:00–15:00 小区 3、4 号楼停水，请提前储水。',
    hoursAgo: 80,
    read: true,
    starred: true,
  },
]

// Relative to load time so the list always looks recent.
const BASE_TIME = Date.now()

function buildEmails(): EmailDetail[] {
  return SEEDS.map((seed, index) => {
    const id = `mock-email-${index + 1}`
    const me = seed.account === WORK ? ME_WORK : ME_HOME
    const sent = seed.role === 'sent' || seed.role === 'drafts'
    const summary: EmailSummary = {
      id,
      accountId: seed.account,
      folderId: seed.folder ? `${seed.account}:${seed.folder}` : folderId(seed.account, seed.role ?? 'inbox'),
      subject: seed.subject,
      from: seed.from,
      preview: seed.text.replace(/\s+/g, ' ').slice(0, 120),
      receivedAt: new Date(BASE_TIME - seed.hoursAgo * 3_600_000).toISOString(),
      isRead: seed.read ?? false,
      isStarred: seed.starred ?? false,
      hasAttachment: (seed.attachments?.length ?? 0) > 0,
      bodyState: 'ready',
      pendingChanges: false,
    }
    return {
      summary,
      to: sent ? [{ name: '团队', address: 'team@example.com' }] : [me],
      cc: [],
      replyTo: [],
      body: { text: seed.text, html: null },
      attachments: (seed.attachments ?? []).map(([filename, mimeType, size], position) => ({
        id: `${id}-attachment-${position + 1}`,
        filename,
        mimeType,
        size,
        isInline: false,
      })),
    }
  })
}
