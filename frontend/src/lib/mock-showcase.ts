import type { AccountConfigInput, ChatTurnTool, MailAddress } from '@/generated/accounts'
import type { CustomFolders, Seed } from '@/lib/mock-client'
import type { SeedChat } from '@/lib/mock-conversations'

/**
 * Fictitious data for screenshots (`?seed=showcase`, or `VITE_MAILX_SEED=showcase` in the desktop dev build).
 * People, services and organizations are invented and use reserved example domains; mailbox addresses on
 * real providers use obvious demo names so the provider icons and sign-in kinds look like the real thing.
 * The bodies are plain text so the reader needs no HTML document.
 */

const WORK = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
const PERSONAL = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
const STUDIO = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc'
const OUTLOOK = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd'
const ICLOUD = 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'
const QQ = 'ffffffff-ffff-4fff-8fff-ffffffffffff'
const NETEASE = '16316316-3163-4163-8163-163163163163'
const CAMPUS = '0e0e0e0e-0e0e-4e0e-8e0e-0e0e0e0e0e0e'

const me = (address: string): MailAddress => ({ name: 'Alex Morgan', address })
const ME = {
  [WORK]: me('alex.morgan@example.com'),
  [PERSONAL]: me('alex.morgan.demo@gmail.com'),
  [OUTLOOK]: me('alex.morgan.demo@outlook.com'),
  [ICLOUD]: me('alex.morgan.demo@icloud.com'),
  [QQ]: me('alex.morgan.demo@qq.com'),
  [NETEASE]: me('alex_morgan_demo@163.com'),
  [CAMPUS]: me('alex.morgan@example.org'),
  [STUDIO]: me('alex@example.net'),
} as Record<string, MailAddress>

const profile = (displayName: string, color: string) => ({ displayName, color, senderName: null, useProviderIcon: true })
const keep = { kind: 'password', mode: 'shared', password: { action: 'keep' } } as const

function imap(id: string, providerId: string | null, name: string, color: string, servers: [string, string, 465 | 587]): AccountConfigInput {
  const username = ME[id].address
  return {
    ...profile(name, color),
    emailAddress: username,
    providerId,
    imap: { host: servers[0], port: 993, tls: 'implicit', username },
    smtp: { host: servers[1], port: servers[2], tls: servers[2] === 465 ? 'implicit' : 'startTls', username },
    authentication: keep,
    route: { kind: 'direct' },
  }
}

export type ShowcaseAccount =
  | { id: string; config: AccountConfigInput }
  | { id: string; oauth: { providerId: 'microsoft-personal' | 'microsoft-work'; address: string; profile: ReturnType<typeof profile> } }

/** One account per common sign-in kind: Microsoft OAuth, app passwords, authorization codes, Exchange EWS and plain IMAP. */
const accounts: ShowcaseAccount[] = [
  { id: WORK, oauth: { providerId: 'microsoft-work', address: ME[WORK].address, profile: profile('Work', '#2563eb') } },
  { id: PERSONAL, config: imap(PERSONAL, 'gmail', 'Gmail', '#16a34a', ['imap.gmail.com', 'smtp.gmail.com', 465]) },
  { id: OUTLOOK, oauth: { providerId: 'microsoft-personal', address: ME[OUTLOOK].address, profile: profile('Outlook', '#0891b2') } },
  { id: ICLOUD, config: imap(ICLOUD, 'icloud', 'iCloud', '#64748b', ['imap.mail.me.com', 'smtp.mail.me.com', 587]) },
  { id: QQ, config: imap(QQ, 'qq', 'QQ', '#ca8a04', ['imap.qq.com', 'smtp.qq.com', 465]) },
  { id: NETEASE, config: imap(NETEASE, 'netease163', '163', '#dc2626', ['imap.163.com', 'smtp.163.com', 465]) },
  {
    id: CAMPUS,
    config: {
      ...profile('University', '#7c3aed'),
      emailAddress: ME[CAMPUS].address,
      providerId: 'exchange-self-hosted',
      backend: 'ews',
      ews: { url: 'https://mail.example.org/EWS/Exchange.asmx', username: ME[CAMPUS].address },
      authentication: keep,
      route: { kind: 'direct' },
    },
  },
  { id: STUDIO, config: imap(STUDIO, null, 'Studio', '#d97706', ['imap.example.net', 'smtp.example.net', 465]) },
]

const customFolders: CustomFolders = {
  [WORK]: [
    ['projects', 'Projects', null, true],
    ['projects/atlas', 'Atlas', 'projects', true],
  ],
  [PERSONAL]: [['travel', 'Travel', null, true]],
  [NETEASE]: [['thesis', '论文', null, true]],
}

const SUBJECTS = {
  roadmap: 'Quarterly roadmap: what we need from you',
  tanaka: '来週の打ち合わせについて',
  thesis: '论文第三章修改意见',
  badminton: '周六羽毛球，场地订好了',
  itinerary: 'Your itinerary: Lisbon → Berlin, November 14',
  buildPassed: '[atlas-web] Build #482 passed',
  buildFailed: '[atlas-web] Build #479 failed',
  maintenance: 'Scheduled maintenance: Saturday 22:00–02:00',
  photos: "Photos from Grandma's birthday",
} as const

const seeds: Seed[] = [
  {
    account: WORK,
    from: { name: 'Sara Novak', address: 'sara.novak@example.com' },
    subject: 'Weekly team update — week 43',
    text: `Hi team,

Here is where we landed this week.

Highlights
  • The onboarding flow shipped to the staging environment on Tuesday.
  • Search latency work is on track; the profiling notes are in the shared doc.
  • We closed 14 of the 19 issues tagged for the October milestone.

In progress
  • Calendar invite parsing (Diego)
  • Offline draft handling (Amara)
  • Release checklist for the next beta (Tom)

Needs a decision
  1. Do we keep the compact list density as the default?
  2. Which two languages do we translate first?

Please add your thoughts to the planning doc by Friday noon. The design review is on Thursday at 14:00, so any open questions are best raised before then.

Thanks, and have a good week.

Sara`,
    hoursAgo: 0.5,
  },
  {
    account: WORK,
    from: { name: 'Build Bot', address: 'build-bot@example.com' },
    subject: SUBJECTS.buildPassed,
    text: 'Build #482 on branch main passed in 6 min 12 s.\n\n214 tests ran, 0 failed, 3 skipped.\nArtifacts are available for 14 days.',
    hoursAgo: 1.2,
    read: true,
  },
  {
    account: WORK,
    from: { name: 'Diego Ramos', address: 'diego.ramos@example.com' },
    subject: 'Design review: onboarding flow, Thursday 14:00',
    text: 'Hi Alex, I attached the latest frames for the onboarding flow. Please look at the empty states in particular. We will walk through them on Thursday.',
    hoursAgo: 2.5,
    attachments: [['onboarding-review.pdf', 'application/pdf', 2_140_000]],
  },
  {
    account: QQ,
    from: { name: '王磊', address: 'wang.lei@example.com' },
    subject: SUBJECTS.badminton,
    text: '周六上午十点，体育馆 3 号场，订了两个小时。小周和阿杰也来，正好双打。你那边方便的话回我一声，我好把人数报给前台。球我带，你带拍子就行。',
    hoursAgo: 1.5,
  },
  {
    account: NETEASE,
    from: { name: '张老师', address: 'zhang.wen@example.org' },
    folder: 'thesis',
    subject: SUBJECTS.thesis,
    text: `Alex 你好：

第三章我看完了，整体结构清楚，实验部分比上一版扎实很多。有三处需要再改一下：

1. 3.2 节的对比实验，基线方法的参数设置要写明来源；
2. 图 3-4 的纵轴单位缺失，建议和表 3-1 统一；
3. 小结部分偏长，可以压缩到半页以内。

批注版放在附件里。下周三之前把修改稿发我就可以。

张文`,
    hoursAgo: 3,
    starred: true,
    attachments: [['第三章-批注版.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', 412_000]],
  },
  {
    account: PERSONAL,
    from: { name: 'Acme Billing', address: 'billing@example.net' },
    subject: 'Your invoice INV-2026-0192 is ready',
    text: 'Your invoice for October is ready. The total of $24.00 is due on November 12. You can find the PDF attached to this message.',
    hoursAgo: 4,
    read: true,
    attachments: [['INV-2026-0192.pdf', 'application/pdf', 96_000]],
  },
  {
    account: ICLOUD,
    from: { name: 'Emma Morgan', address: 'emma.morgan@example.org' },
    subject: SUBJECTS.photos,
    text: 'Here are the photos from Sunday! Grandma loved the cake. The one of everyone on the porch is my favourite — could you print a copy for her?',
    hoursAgo: 5,
    attachments: [['porch.jpg', 'image/jpeg', 2_400_000], ['cake.jpg', 'image/jpeg', 1_900_000]],
  },
  {
    account: WORK,
    from: { name: 'Amara Okafor', address: 'amara.okafor@example.com' },
    subject: 'Invitation: Q4 planning sync',
    text: 'You are invited to the Q4 planning sync on Monday, 10:30–11:30. We will go through priorities, staffing and the release calendar. Agenda to follow.',
    hoursAgo: 6,
    starred: true,
  },
  {
    account: OUTLOOK,
    from: { name: 'Harbor Fitness', address: 'hello@example.com' },
    subject: 'Booking confirmed: Yoga Flow, Saturday 09:00',
    text: 'Your spot in Yoga Flow on Saturday at 09:00 is confirmed. Please arrive ten minutes early. You can cancel free of charge up to 12 hours before the class.',
    hoursAgo: 7,
    read: true,
  },
  {
    account: CAMPUS,
    from: { name: 'IT Service Desk', address: 'it-help@example.org' },
    subject: SUBJECTS.maintenance,
    text: 'Mail and calendar services will be unavailable on Saturday from 22:00 to 02:00 while we upgrade the servers. Messages sent to you during this time will be delivered afterwards.',
    hoursAgo: 8,
  },
  {
    account: PERSONAL,
    from: { name: 'Skyline Air', address: 'reservations@example.net' },
    folder: 'travel',
    subject: SUBJECTS.itinerary,
    text: 'Your booking is confirmed. Flight SK 218 departs Lisbon at 07:45 and arrives in Berlin at 12:05. Online check-in opens 24 hours before departure.',
    hoursAgo: 9,
    read: true,
    starred: true,
    attachments: [['itinerary.pdf', 'application/pdf', 134_000]],
  },
  {
    account: QQ,
    from: { name: '顺路快递', address: 'notice@example.net' },
    subject: '您的包裹已到达驿站',
    text: '您的包裹已到达小区东门驿站，取件码 6-2-1035，请在 3 天内取件。营业时间 08:00–21:00。',
    hoursAgo: 11,
    read: true,
  },
  {
    account: WORK,
    from: { name: 'Priscilla Hart', address: 'priscilla.hart@example.com' },
    subject: SUBJECTS.roadmap,
    html: true,
    text: `Hello Alex,

Thank you for joining the planning call yesterday. Below is a short summary of what we agreed, and the three decisions we still need from your side before the roadmap is final.

What we agreed
  • The beta ships in the first week of December.
  • We keep the current pricing until the end of the year.
  • Support hours stay the same, but we add a Saturday on-call rotation.

What we need from you
  1. Confirm the list of languages for the launch.
  2. Choose which of the two onboarding flows to keep.
  3. Tell us who will own the migration guide.

If you can reply by Friday, we can still include everything in the next planning round. Please let me know if anything above looks wrong.

Best regards,
Priscilla`,
    translatedSubject: '季度路线图：我们需要你的确认',
    hoursAgo: 0.2,
  },
  {
    account: PERSONAL,
    from: { name: '田中 ゆき', address: 'tanaka.yuki@example.org' },
    subject: SUBJECTS.tanaka,
    html: true,
    text: `アレックスさん

お世話になっております。来週の打ち合わせについて、日程のご相談です。

火曜日の午後三時、または水曜日の午前十時はいかがでしょうか。場所は駅前のカフェを予定しています。資料は前日までにメールでお送りしますので、事前にご確認いただけると助かります。

ご都合が悪い場合は、遠慮なくお知らせください。別の日程をすぐに調整します。

どうぞよろしくお願いいたします。

田中`,
    translatedSubject: 'Regarding next week\'s meeting',
    hoursAgo: 0.8,
  },
  {
    account: STUDIO,
    from: { name: 'Lena Fischer', address: 'lena.fischer@example.net' },
    subject: 'Ihre Bestellung wurde versendet',
    html: true,
    text: `Hallo Alex,

wir freuen uns, Ihnen mitzuteilen, dass Ihre Bestellung heute versendet wurde. Die Lieferung ist für Donnerstag geplant und wird von einem unserer Partner zugestellt.

Die Sendungsverfolgung finden Sie in Ihrem Kundenkonto. Wenn Sie nicht zu Hause sind, wird das Paket bei Ihrem Nachbarn abgegeben oder in einer Filiale hinterlegt.

Vielen Dank für Ihr Vertrauen und einen schönen Tag noch.

Lena Fischer`,
    translatedSubject: 'Your order has been shipped',
    hoursAgo: 3.2,
  },
  {
    account: WORK,
    from: { name: '陈思远', address: 'chen.siyuan@example.com' },
    subject: '下周评审会议改期通知',
    text: '大家好，原定周二的评审会议改到周三下午 15:00，地点不变。请大家提前阅读评审材料，有问题可以直接回复这封邮件。',
    hoursAgo: 14,
  },
  {
    account: CAMPUS,
    from: { name: 'Prof. Helen Brooks', address: 'helen.brooks@example.org' },
    subject: 'Seminar slides for Thursday',
    text: 'Dear all, the slides for Thursday are attached. Please read sections 2 and 3 before we meet; we will spend most of the hour on the open questions at the end.',
    hoursAgo: 16,
    read: true,
    starred: true,
    attachments: [['seminar-week-9.pptx', 'application/vnd.openxmlformats-officedocument.presentationml.presentation', 3_600_000]],
  },
  {
    account: STUDIO,
    from: { name: 'The Weekly Byte', address: 'newsletter@example.net' },
    subject: 'The Weekly Byte #128: Local-first software, one year on',
    text: 'This week: what changed for local-first apps over the past year, a short guide to sync conflicts, and five tools worth a look.',
    hoursAgo: 20,
    read: true,
  },
  {
    account: PERSONAL,
    from: { name: 'Yuki Tanaka', address: 'yuki.tanaka@example.org' },
    subject: 'Weekend hike?',
    text: 'The forecast looks good for Saturday. I was thinking of the ridge trail, about four hours round trip. Want to join? We could meet at 08:30.',
    hoursAgo: 26,
  },
  {
    account: WORK,
    from: { name: 'Hassan Ali', address: 'hassan.ali@example.com' },
    folder: 'projects/atlas',
    subject: 'Re: Contract renewal — a few edits',
    text: 'Thanks for the draft. I marked three small edits in the attached copy, mostly around the notice period. Happy to talk it through tomorrow.',
    hoursAgo: 30,
    read: true,
    attachments: [['renewal-edits.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', 58_000]],
  },
  {
    account: OUTLOOK,
    from: { name: 'Marco Bianchi', address: 'marco.bianchi@example.net' },
    subject: 'Long time no see — coffee next week?',
    text: "I'm in town from Tuesday to Friday for a conference. Any chance you're free for a coffee one of those mornings? It's been far too long.",
    hoursAgo: 33,
  },
  {
    account: STUDIO,
    from: { name: 'Priya Nair', address: 'priya.nair@example.net' },
    subject: 'Logo exploration, round 2',
    text: 'Here are the second round of marks. Version C is my favourite: it holds up at small sizes. Let me know which direction to refine.',
    hoursAgo: 38,
    attachments: [['logo-round-2.png', 'image/png', 820_000]],
  },
  {
    account: NETEASE,
    from: { name: '城市电力', address: 'bill@example.net' },
    subject: '您的 10 月电费账单',
    text: '尊敬的用户：您 10 月的用电量为 186 度，应缴电费 98.40 元，缴费截止日期为 11 月 20 日。电子发票见附件。',
    hoursAgo: 40,
    read: true,
    attachments: [['电子发票-202610.pdf', 'application/pdf', 72_000]],
  },
  {
    account: PERSONAL,
    from: { name: '李梅', address: 'li.mei@example.org' },
    subject: '周末聚餐的地点',
    text: '周六晚上聚餐定在老城区那家新开的餐厅，六点半见。记得提前订位，我已经问过，他们可以保留到七点。',
    hoursAgo: 45,
    read: true,
  },
  {
    account: ICLOUD,
    from: { name: 'Cloud Storage', address: 'billing@example.net' },
    subject: 'Your storage plan renews on December 2',
    text: 'Your 200 GB plan renews on December 2 for $2.99. No action is needed. You can change or cancel your plan in your account settings.',
    hoursAgo: 50,
    read: true,
  },
  {
    account: WORK,
    from: { name: 'Build Bot', address: 'build-bot@example.com' },
    subject: SUBJECTS.buildFailed,
    text: 'Build #479 on branch feature/search failed after 4 min 40 s.\n\n2 tests failed: search_index_rebuild, search_cursor_paging.',
    hoursAgo: 52,
    read: true,
  },
  {
    account: STUDIO,
    from: { name: 'Acme Billing', address: 'billing@example.net' },
    subject: 'Payment received — thank you',
    text: 'We received your payment of 120.00 for the Studio plan. A receipt is available in your account.',
    hoursAgo: 60,
    read: true,
  },
  {
    account: PERSONAL,
    from: { name: 'City Library', address: 'library@example.org' },
    subject: 'Reminder: 2 items due Friday',
    text: 'Two items on your card are due this Friday. You can renew them online unless another reader has placed a hold.',
    hoursAgo: 72,
    read: true,
  },
  {
    account: WORK,
    from: { name: 'Tom Becker', address: 'tom.becker@example.com' },
    subject: "Notes from Tuesday's retro",
    text: 'Short summary of the retro: keep the weekly demo, shorten stand-ups to ten minutes, and write down release decisions in one place.',
    hoursAgo: 80,
    read: true,
    starred: true,
  },
  {
    account: STUDIO,
    from: { name: 'Elena Rossi', address: 'elena.rossi@example.net' },
    subject: 'Photos from the offsite',
    text: 'The photos from last week are in the shared album. I picked the best twenty; the rest are in the archive folder.',
    hoursAgo: 90,
    read: true,
  },
  {
    account: PERSONAL,
    from: { name: 'Example Accounts', address: 'login@example.org' },
    subject: 'Your MailX verification code',
    text: 'Hello Alex,\n\nYour verification code is: 001234\n\nUse this code to sign in. This is a synthetic preview message.',
    verificationCode: '001234',
    hoursAgo: 0.03,
  },
]

/** The AI draft the first saved chat points to. */
const REPLY_DRAFT = {
  account: QQ,
  source: SUBJECTS.badminton,
  to: { name: '王磊', address: 'wang.lei@example.com' },
  subject: `Re: ${SUBJECTS.badminton}`,
  text: '没问题，周六上午十点 3 号场见，我带拍子。要是需要多带一副，提前跟我说一声。',
}

type ChatRefs = {
  /** Id of the showcase email with this subject. */
  email: (subject: string) => string | null
  /** The AI draft in `REPLY_DRAFT`, made on first use. */
  draft: () => { id: string; subject: string } | null
}

/** Saved chats for the assistant history: they lean on the emails above so the cards open real messages. */
function chats(zh: boolean, refs: ChatRefs): SeedChat[] {
  const card = (label: string, subject: string) => {
    const id = refs.email(subject)
    return id ? `[${label}](mailx://email/${id})` : label
  }
  const ids = (...subjects: string[]) => subjects.map(refs.email).filter((id): id is string => id !== null)
  const tool = (callId: string, name: string, summary: ChatTurnTool['summary'], textOffset = 0): ChatTurnTool =>
    ({ callId, name, status: 'done', summary, textOffset })
  const draft = refs.draft()

  const triageAsk = zh ? '我所有邮箱里，今天有哪些需要回复的？' : 'Which emails across my accounts need a reply today?'
  const triageReply = zh
    ? `看了 8 个账号今天的未读邮件，有 4 封在等你回复：

1. **路线图确认**（Work · Microsoft 365）— Priscilla 需要你在周五前定三件事：上线语言、保留哪个引导流程、迁移指南由谁负责。
   ${card('Quarterly roadmap: what we need from you', SUBJECTS.roadmap)}
2. **论文修改意见**（163）— 张老师提了三处修改，下周三前交修改稿。
   ${card('论文第三章修改意见', SUBJECTS.thesis)}
3. **下周的会面时间**（Gmail）— 田中提议周二 15:00 或周三 10:00，邮件是日语写的。
   ${card('来週の打ち合わせについて', SUBJECTS.tanaka)}
4. **周六羽毛球**（QQ）— 王磊在等你确认人数。
   ${card('周六羽毛球，场地订好了', SUBJECTS.badminton)}

另外，University 账号周六晚上 22:00 到次日 02:00 有服务器维护，不用回复，但那段时间收不到新邮件。`
    : `I looked at today's unread mail in all 8 accounts. Four messages are waiting on you:

1. **Roadmap decisions** (Work · Microsoft 365) — Priscilla needs three answers by Friday: launch languages, which onboarding flow to keep, and who owns the migration guide.
   ${card('Quarterly roadmap: what we need from you', SUBJECTS.roadmap)}
2. **Thesis feedback** (163) — three changes to chapter 3, due next Wednesday.
   ${card('论文第三章修改意见', SUBJECTS.thesis)}
3. **Meeting time next week** (Gmail) — Tanaka suggests Tuesday 15:00 or Wednesday 10:00. The message is in Japanese.
   ${card('来週の打ち合わせについて', SUBJECTS.tanaka)}
4. **Saturday badminton** (QQ) — Wang Lei is waiting to confirm numbers.
   ${card('周六羽毛球，场地订好了', SUBJECTS.badminton)}

Also: the University account has server maintenance on Saturday from 22:00 to 02:00. No reply needed, but no new mail will arrive during that window.`
  const draftAsk = zh ? '先帮我回王磊，说周六可以，我带拍子。' : "Reply to Wang Lei first: Saturday works, I'll bring my racket."
  const draftReply = zh
    ? '草稿写好了，用的是你的 QQ 账号，可以直接打开修改或发送：'
    : "Here's the draft, from your QQ account. Open it to edit or send:"

  const archiveAsk = zh ? '把已经看过的构建通知都归档' : 'Archive the build notifications I have already read'
  const archiveReply = zh
    ? '找到 2 封已读的构建通知，都在 Work 账号里，已归档。需要的话可以在“已归档”里找回。'
    : 'Found 2 read build notifications, both in the Work account, and archived them. They are in Archive if you need them.'

  const tripAsk = zh ? '我去柏林的航班几点起飞？' : 'When does my flight to Berlin leave?'
  const tripReply = zh
    ? `11 月 14 日，SK 218 航班，**07:45** 从里斯本起飞，12:05 到柏林。起飞前 24 小时开放网上值机。

${card('Your itinerary: Lisbon → Berlin, November 14', SUBJECTS.itinerary)}`
    : `November 14, flight SK 218. It leaves Lisbon at **07:45** and lands in Berlin at 12:05. Online check-in opens 24 hours before departure.

${card('Your itinerary: Lisbon → Berlin, November 14', SUBJECTS.itinerary)}`

  const photoAsk = zh ? '奶奶生日的照片是谁发的？' : "Who sent the photos from Grandma's birthday?"
  const photoReply = zh
    ? `是 Emma 发到你 iCloud 邮箱的，附件里有两张照片。她想让你把门廊那张全家福打印一份给奶奶。

${card("Photos from Grandma's birthday", SUBJECTS.photos)}`
    : `Emma sent them to your iCloud address, with two photos attached. She'd like you to print the one of everyone on the porch for Grandma.

${card("Photos from Grandma's birthday", SUBJECTS.photos)}`

  return [
    {
      minutesAgo: 18,
      turns: [
        {
          user: triageAsk,
          text: triageReply,
          tools: [tool('call-1', 'mail_search', { type: 'found', count: 11 }), tool('call-2', 'mail_read', { type: 'read', emailIds: ids(SUBJECTS.roadmap, SUBJECTS.thesis, SUBJECTS.tanaka, SUBJECTS.badminton) })],
          reasoning: [
            {
              text: zh
                ? '用户要的是今天需要回复的邮件，范围是全部账号。先搜今天的未读，再逐封读正文，判断是不是在等回复。'
                : 'The user wants today\'s mail that needs a reply, across every account. Search today\'s unread first, then read each one to see whether it is waiting on an answer.',
              durationMs: 3800, textOffset: 0, toolsBefore: 0,
            },
            {
              text: zh
                ? '11 封里有 4 封明确在等回复。路线图有周五的截止时间，排第一；维护通知不用回复，但值得提一句。'
                : 'Four of the eleven clearly expect an answer. The roadmap has a Friday deadline, so it goes first; the maintenance notice needs no reply but is worth a mention.',
              durationMs: 1600, textOffset: 0, toolsBefore: 2,
            },
          ],
        },
        {
          user: draftAsk,
          text: draftReply,
          tools: draft ? [tool('call-3', 'draft_reply', { type: 'draft', draftId: draft.id, subject: draft.subject }, draftReply.length)] : [],
        },
      ],
    },
    {
      minutesAgo: 60 * 3,
      turns: [{
        user: archiveAsk,
        text: archiveReply,
        tools: [
          tool('call-1', 'mail_search', { type: 'found', count: 2 }),
          tool('call-2', 'mail_archive', { type: 'changed', count: 2, emailIds: ids(SUBJECTS.buildPassed, SUBJECTS.buildFailed), undo: null }),
        ],
        confirmations: [{ callId: 'call-2', tool: 'mail_archive', action: 'organize', outcome: 'approved', textOffset: 0 }],
      }],
    },
    {
      minutesAgo: 60 * 22,
      turns: [{ user: tripAsk, text: tripReply, tools: [tool('call-1', 'mail_search', { type: 'found', count: 1 })] }],
    },
    {
      minutesAgo: 60 * 46,
      turns: [{ user: photoAsk, text: photoReply, tools: [tool('call-1', 'mail_search', { type: 'found', count: 1 })] }],
    },
  ]
}

export const SHOWCASE_DATASET = {
  accounts,
  seeds,
  customFolders,
  me: ME,
  replyDraft: REPLY_DRAFT,
  chats,
}
