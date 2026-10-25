import { afterEach, describe, expect, it } from 'vitest'
import { i18n } from '@/i18n'
import { recipientsPreview, reasonText, STATE_LABELS } from './outbox-text'

afterEach(() => { void i18n.changeLanguage('zh-CN') })

describe('outbox display text', () => {
  it('resolves states and failure reasons at use time after a language change', async () => {
    expect(STATE_LABELS.queued).toBe('等待发送')
    await i18n.changeLanguage('en')
    expect(STATE_LABELS.queued).toBe('Waiting to send')
    expect(reasonText('network')).toBe('The network connection was interrupted')
    expect(reasonText('constructor')).toBe('Send failed')
    expect(reasonText(null)).toBe('Unknown reason')
  })

  it('translates recipient count text while keeping names and their existing separator', async () => {
    await i18n.changeLanguage('en')
    expect(recipientsPreview({ recipients: ['林薇', 'a@example.org'], recipientCount: 5 }))
      .toBe('林薇、a@example.org and others (5 recipients)')
    expect(recipientsPreview({ recipients: ['{{name}}'], recipientCount: 1 })).toBe('{{name}}')
  })
})
