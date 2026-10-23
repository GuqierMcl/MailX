import { describe, expect, it } from 'vitest'
import { emailQuery } from '@/features/mail/format'
import { createMockClient } from './mock-client'

describe('mock email pagination', () => {
  it('paginates the filtered inbox without duplicating or omitting summaries', async () => {
    const client = createMockClient()
    const query = emailQuery({ folderRole: 'inbox', unread: true, limit: 2 })
    const all = await client.listEmails({ ...query, limit: 200 })
    const ids: string[] = []
    let cursor: string | null = null
    do {
      const page = await client.listEmails({ ...query, cursor })
      expect(page.items.length).toBeLessThanOrEqual(2)
      expect(page.items.every((item) => !item.isRead)).toBe(true)
      ids.push(...page.items.map((item) => item.id))
      cursor = page.nextCursor
    } while (cursor)
    expect(ids.length).toBeGreaterThan(2)
    expect(ids).toEqual(all.items.map((item) => item.id))
    expect(new Set(ids).size).toBe(ids.length)
  })
})

describe('mock flag changes', () => {
  it('moves the folder unread number like the core does, and never below zero', async () => {
    const client = createMockClient()
    const [account] = await client.listAccounts()
    const unreadOf = async (folderId: string) =>
      (await client.listFolders(account.id)).find((folder) => folder.id === folderId)!.unreadCount
    const page = await client.listEmails({ ...emailQuery({}), accountIds: [account.id], unread: true, limit: 50 })
    const target = page.items[0]
    const before = await unreadOf(target.folderId)
    expect(await client.setFlag(target.id, 'read', true)).toBe('queued')
    expect(await unreadOf(target.folderId)).toBe(before - 1)
    // The same value again changes nothing, including the number.
    expect(await client.setFlag(target.id, 'read', true)).toBe('unchanged')
    expect(await unreadOf(target.folderId)).toBe(before - 1)
    await client.setFlag(target.id, 'read', false)
    expect(await unreadOf(target.folderId)).toBe(before)
    // A star never moves it.
    await client.setFlag(target.id, 'starred', true)
    expect(await unreadOf(target.folderId)).toBe(before)
  })
})

describe('mock drafts and sending', () => {
  const to = [{ name: null, address: 'a@example.com' }]
  const content = { to, cc: [], bcc: [], subject: '你好', bodyText: '正文' }
  const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

  it('creates, updates with revisions and lists drafts', async () => {
    const client = createMockClient()
    const draft = await client.createDraft({ accountId: null, ...content, clientMutationId: 'm1' })
    expect((await client.createDraft({ accountId: null, ...content, clientMutationId: 'm1' })).id).toBe(draft.id)
    expect(draft.revision).toBe(1)
    const saved = await client.updateDraft({ id: draft.id, expectedRevision: 1, accountId: null, ...content, subject: '改' })
    expect([saved.revision, saved.subject]).toEqual([2, '改'])
    await expect(client.updateDraft({ id: draft.id, expectedRevision: 1, accountId: null, ...content })).rejects.toMatchObject({ failure: { code: 'conflict' } })
    expect((await client.listDrafts()).map((row) => row.id)).toEqual([draft.id])
    await client.deleteDraft(draft.id)
    await expect(client.getDraft(draft.id)).rejects.toMatchObject({ failure: { code: 'notFound' } })
  })

  it('walks a queued item through sending to accepted and drops the draft', async () => {
    const client = createMockClient({ sendStepMs: 5 })
    const draft = await client.createDraft({ accountId: null, ...content, clientMutationId: 'm1' })
    const states: string[] = []
    const draftChanges: string[] = []
    client.onOutbox((change) => states.push(change.state))
    client.onDrafts((change) => draftChanges.push(change.change))
    const item = await client.queueSend({ draftId: draft.id, expectedRevision: 1, clientMutationId: 'q1' })
    expect(item.state).toBe('queued')
    expect((await client.queueSend({ draftId: draft.id, expectedRevision: 1, clientMutationId: 'q1' })).id).toBe(item.id)
    await expect(client.updateDraft({ id: draft.id, expectedRevision: 1, accountId: null, ...content })).rejects.toMatchObject({ failure: { code: 'draftLocked' } })
    await wait(60)
    expect(states).toEqual(['queued', 'sending', 'accepted'])
    expect(draftChanges).toEqual(['locked', 'deleted'])
    const done = await client.getOutbox(item.id)
    expect(done.state).toBe('accepted')
    expect(done.serverReply?.text).toContain('simulated')
    expect(await client.listOutbox()).toEqual([])
    expect((await client.listOutbox({ includeAll: true })).map((row) => row.id)).toEqual([item.id])
  })

  it('refuses a send without recipients or with a stale revision', async () => {
    const client = createMockClient()
    const empty = await client.createDraft({ accountId: null, ...content, to: [], clientMutationId: 'm1' })
    await expect(client.queueSend({ draftId: empty.id, expectedRevision: 1, clientMutationId: 'q1' })).rejects.toMatchObject({ failure: { code: 'validation', field: 'draft.recipients' } })
    await expect(client.queueSend({ draftId: empty.id, expectedRevision: 9, clientMutationId: 'q2' })).rejects.toMatchObject({ failure: { code: 'conflict' } })
  })

  it('cancels during the undo window and unlocks the draft', async () => {
    const client = createMockClient({ sendStepMs: 5 })
    await client.updateSettings({ compose: { defaultAccountId: '', undoSendSeconds: 5 } })
    const draft = await client.createDraft({ accountId: null, ...content, clientMutationId: 'm1' })
    const draftChanges: string[] = []
    client.onDrafts((change) => draftChanges.push(change.change))
    const item = await client.queueSend({ draftId: draft.id, expectedRevision: 1, clientMutationId: 'q1' })
    expect(new Date(item.notBefore).getTime()).toBeGreaterThan(Date.now() + 4000)
    expect((await client.cancelOutbox(item.id)).state).toBe('canceled')
    await wait(30)
    expect((await client.getOutbox(item.id)).state).toBe('canceled')
    expect(draftChanges).toEqual(['locked', 'unlocked'])
    await expect(client.updateDraft({ id: draft.id, expectedRevision: 1, accountId: null, ...content })).resolves.toMatchObject({ revision: 2 })
    await expect(client.cancelOutbox(item.id)).rejects.toMatchObject({ failure: { code: 'conflict' } })
  })

  it('cannot cancel once sending has begun', async () => {
    const client = createMockClient({ sendStepMs: 20 })
    const draft = await client.createDraft({ accountId: null, ...content, clientMutationId: 'm1' })
    const item = await client.queueSend({ draftId: draft.id, expectedRevision: 1, clientMutationId: 'q1' })
    await wait(30)
    expect((await client.getOutbox(item.id)).state).toBe('sending')
    await expect(client.cancelOutbox(item.id)).rejects.toMatchObject({ failure: { code: 'alreadySending' } })
  })

  it('builds reply and forward drafts from a cached message', async () => {
    const client = createMockClient()
    const [account] = await client.listAccounts()
    const page = await client.listEmails({ ...emailQuery({ folderRole: 'inbox', limit: 1 }), accountIds: [account.id] })
    const reply = await client.createDraftFrom({ emailId: page.items[0].id, kind: 'reply', clientMutationId: 'r1' })
    expect(reply.subject.startsWith('回复：')).toBe(true)
    expect(reply.to).toEqual([page.items[0].from])
    const forward = await client.createDraftFrom({ emailId: page.items[0].id, kind: 'forward', clientMutationId: 'r2' })
    expect(forward.to).toEqual([])
    await expect(client.openComposer(reply.id)).resolves.toBeUndefined()
    await expect(client.openComposer('missing')).rejects.toMatchObject({ failure: { code: 'notFound' } })
  })
})
