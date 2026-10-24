import type { AccountConfigInput, MailAddress } from '@/generated/accounts'
import type { CustomFolders, Seed } from '@/lib/mock-client'

/**
 * Fictitious data for README screenshots in the browser preview (`?seed=showcase`).
 * Every person, service and address is invented and uses reserved example domains.
 * The bodies are plain text so the reader needs no HTML document.
 */

const WORK = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
const PERSONAL = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
const STUDIO = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc'

const ME_WORK: MailAddress = { name: 'Alex Morgan', address: 'alex.morgan@example.com' }
const ME_PERSONAL: MailAddress = { name: 'Alex Morgan', address: 'alex@example.org' }
const ME_STUDIO: MailAddress = { name: 'Alex Morgan', address: 'alex@example.net' }

function config(displayName: string, color: string, address: string, domain: string): AccountConfigInput {
  return {
    displayName,
    color,
    emailAddress: address,
    providerId: null,
    imap: { host: `imap.${domain}`, port: 993, tls: 'implicit', username: address },
    smtp: { host: `smtp.${domain}`, port: 465, tls: 'implicit', username: address },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    route: { kind: 'direct' },
  }
}

const customFolders: CustomFolders = {
  [WORK]: [
    ['projects', 'Projects', null, true],
    ['projects/atlas', 'Atlas', 'projects', true],
  ],
  [PERSONAL]: [['travel', 'Travel', null, true]],
  [STUDIO]: [],
}

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
    subject: '[atlas-web] Build #482 passed',
    text: 'Build #482 on branch main passed in 6 min 12 s.\n\n214 tests ran, 0 failed, 3 skipped.\nArtifacts are available for 14 days.',
    hoursAgo: 1.2,
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
    account: PERSONAL,
    from: { name: 'Acme Billing', address: 'billing@example.net' },
    subject: 'Your invoice INV-2026-0192 is ready',
    text: 'Your invoice for October is ready. The total of $24.00 is due on November 12. You can find the PDF attached to this message.',
    hoursAgo: 4,
    read: true,
    attachments: [['INV-2026-0192.pdf', 'application/pdf', 96_000]],
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
    account: PERSONAL,
    from: { name: 'Skyline Air', address: 'reservations@example.net' },
    folder: 'travel',
    subject: 'Your itinerary: Lisbon → Berlin, November 14',
    text: 'Your booking is confirmed. Flight SK 218 departs Lisbon at 07:45 and arrives in Berlin at 12:05. Online check-in opens 24 hours before departure.',
    hoursAgo: 9,
    read: true,
    starred: true,
    attachments: [['itinerary.pdf', 'application/pdf', 134_000]],
  },
  {
    account: WORK,
    from: { name: '陈思远', address: 'chen.siyuan@example.com' },
    subject: '下周评审会议改期通知',
    text: '大家好，原定周二的评审会议改到周三下午 15:00，地点不变。请大家提前阅读评审材料，有问题可以直接回复这封邮件。',
    hoursAgo: 14,
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
    account: STUDIO,
    from: { name: 'Priya Nair', address: 'priya.nair@example.net' },
    subject: 'Logo exploration, round 2',
    text: 'Here are the second round of marks. Version C is my favourite: it holds up at small sizes. Let me know which direction to refine.',
    hoursAgo: 38,
    attachments: [['logo-round-2.png', 'image/png', 820_000]],
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
    account: WORK,
    from: { name: 'Build Bot', address: 'build-bot@example.com' },
    subject: '[atlas-web] Build #479 failed',
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
]

export const SHOWCASE_DATASET = {
  accounts: [
    { id: WORK, config: config('Work', '#2563eb', ME_WORK.address, 'example.com') },
    { id: PERSONAL, config: config('Personal', '#16a34a', ME_PERSONAL.address, 'example.org') },
    { id: STUDIO, config: config('Studio', '#d97706', ME_STUDIO.address, 'example.net') },
  ],
  seeds,
  customFolders,
  me: { [WORK]: ME_WORK, [PERSONAL]: ME_PERSONAL, [STUDIO]: ME_STUDIO } as Record<string, MailAddress>,
}
