import { describe, expect, it } from 'vitest'
import { draftMentionsAttachment, mentionsAttachment, ownBodyText, replyQuoteSpan } from './reply-quote'

const p = (text = 'x') => ({ quote: false, paragraph: true, empty: !text })
const q = { quote: true, paragraph: false, empty: false }
const list = { quote: false, paragraph: false, empty: false }

describe('reply quote span', () => {
  it('is the last top-level quote plus the non-empty paragraph right before it', () => {
    expect(replyQuoteSpan([p(''), p('On … wrote:'), q])).toEqual({ start: 1, end: 2 })
    expect(replyQuoteSpan([p('mine'), q, p('more'), p('intro'), q, p('after')])).toEqual({ start: 3, end: 4 })
  })

  it('takes the quote alone when the block before it is empty or not a paragraph', () => {
    expect(replyQuoteSpan([p('a'), p(''), q])).toEqual({ start: 2, end: 2 })
    expect(replyQuoteSpan([list, q])).toEqual({ start: 1, end: 1 })
    expect(replyQuoteSpan([q])).toEqual({ start: 0, end: 0 })
  })

  it('is null without a top-level quote', () => {
    expect(replyQuoteSpan([])).toBeNull()
    expect(replyQuoteSpan([p('a'), list])).toBeNull()
  })
})

describe('own body text', () => {
  const html = '<p></p><p>Thanks, see below.</p><p>On Monday, Zhang wrote:</p><blockquote><p>The contract is attached.</p></blockquote>'

  it('leaves out the quoted original and its intro line of a reply', () => {
    for (const kind of ['reply', 'replyAll'] as const) {
      const text = ownBodyText(html, kind)
      expect(text).toContain('Thanks, see below.')
      expect(text).not.toContain('wrote')
      expect(text).not.toContain('contract')
    }
  })

  it('keeps everything in new mail', () => {
    expect(ownBodyText(html, 'new')).toContain('contract')
  })

  // The core's forward body: the user's lines, then the header in the draft's language, then the original.
  const forward = (divider: string, labels: string[]) => `<p>FYI, nothing attached here</p><p></p><p>${divider}</p>${labels.map((label) => `<p>${label} x</p>`).join('')}<p>The contract is attached.</p><blockquote><p>see attachment</p></blockquote>`

  it('keeps only what is above a forward header, in either language', () => {
    for (const body of [
      forward('---------- Forwarded message ----------', ['From:', 'Date:', 'Subject: 附件', 'To:']),
      forward('---------- 转发的邮件 ----------', ['发件人：', '日期：', '主题：附件', '收件人：']),
    ]) {
      const text = ownBodyText(body, 'forward')
      expect(text).toContain('FYI')
      expect(text).not.toMatch(/contract|attachment|附件|From|发件人/)
    }
  })

  it('keeps a forward whole when its header is gone', () => {
    expect(ownBodyText('<p>Hi</p><p>The contract is attached.</p>', 'forward')).toContain('contract')
  })

  it('ignores whitespace between blocks when finding the intro line', () => {
    expect(ownBodyText('<p>Hi</p>\n<p>Zhang wrote:</p>\n<blockquote><p>old</p></blockquote>\n', 'reply').trim()).toBe('Hi')
  })
})

describe('attachment words', () => {
  it('matches the Chinese words anywhere', () => {
    for (const text of ['请查收附件', '附上报价单', '附带一份', '随信附说明', '详见附表', '所附文件']) expect(mentionsAttachment(text)).toBe(true)
    expect(mentionsAttachment('附近的餐厅')).toBe(false)
  })

  it('matches the English words as whole words, ignoring case', () => {
    for (const text of ['I attach the file', 'See ATTACHED.', 'the attachment', 'two attachments', 'Enclosed is the form']) expect(mentionsAttachment(text)).toBe(true)
    for (const text of ['reattached', 'attaching it later', 'detachment', 'enclosedness']) expect(mentionsAttachment(text)).toBe(false)
  })
})

describe('draft attachment check', () => {
  it('reads the subject of new mail only, since replies and forwards inherit theirs', () => {
    expect(draftMentionsAttachment({ subject: '合同附件', bodyHtml: '<p>请看</p>' }, 'new')).toBe(true)
    for (const kind of ['reply', 'replyAll', 'forward'] as const) {
      expect(draftMentionsAttachment({ subject: 'Re: 合同附件', bodyHtml: '<p>收到</p>' }, kind)).toBe(false)
      expect(draftMentionsAttachment({ subject: 'Re: 合同', bodyHtml: '<p>附上新版</p>' }, kind)).toBe(true)
    }
  })
})
