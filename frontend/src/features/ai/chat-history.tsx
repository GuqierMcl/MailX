import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { Delete02Icon, HistoryIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { toast } from '@/components/ui/toast'
import { IconButton } from '@/components/icon-button'
import { relativeTime } from '@/features/now/relative-time'
import { requestSettings } from '@/features/settings/settings-requests'
import { useSettings } from '@/features/settings/settings-queries'
import type { ConversationSummary } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { assistantKeys } from '@/lib/query-client'
import { cn } from '@/lib/utils'
import type { AssistantChat } from './use-assistant-chat'

/** Conversations loaded at a time. */
const HISTORY_PAGE = 30

type Group = 'today' | 'yesterday' | 'earlier'
const GROUPS: Group[] = ['today', 'yesterday', 'earlier']

/** Which heading a conversation sits under, by the calendar day it was last written to. */
function groupOf(updatedAt: string, now = new Date()): Group {
  const time = Date.parse(updatedAt)
  if (Number.isNaN(time)) return 'earlier'
  if (time >= new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) return 'today'
  return time >= new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).getTime() ? 'yesterday' : 'earlier'
}

/** Moves between the rows with the arrow keys; Home and End jump to the ends. */
function moveFocus(list: HTMLElement | null, event: KeyboardEvent) {
  if (!list || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
  const rows = [...list.querySelectorAll<HTMLElement>('[data-history-row]')]
  if (rows.length === 0) return
  const at = rows.indexOf(document.activeElement as HTMLElement)
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1
    : event.key === 'ArrowDown' ? Math.min(rows.length - 1, at + 1) : Math.max(0, at < 0 ? 0 : at - 1)
  rows[next].focus()
  event.preventDefault()
}

/** "确定" and "取消" for a delete, in place of the row; the safe one has the focus. */
function Ask({ question, onConfirm, onCancel, className }: { question: string; onConfirm: () => void; onCancel: () => void; className?: string }) {
  const cancel = useRef<HTMLButtonElement>(null)
  useEffect(() => { cancel.current?.focus({ preventScroll: true }) }, [])
  return (
    <div
      role="group"
      aria-label={question}
      // Esc takes back the question, not the whole panel.
      onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); event.preventDefault(); onCancel() } }}
      className={cn('flex min-h-8 items-center gap-1 px-2.5 text-sm', className)}
    >
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{question}</span>
      <Button ref={cancel} type="button" variant="ghost" size="sm" onClick={onCancel}>{t('ai:cancel')}</Button>
      <Button type="button" variant="destructive" size="sm" onClick={onConfirm}>{t('ai:history.confirm')}</Button>
    </div>
  )
}

function Row({ item, current, asking, language, onOpen, onAsk, onCancel, onDelete }: {
  item: ConversationSummary
  current: boolean
  asking: boolean
  language: string
  onOpen: () => void
  onAsk: () => void
  onCancel: () => void
  onDelete: () => void
}) {
  const row = useRef<HTMLButtonElement>(null)
  // Back from "删除？" without deleting: the focus returns to the row.
  const wasAsking = useRef(false)
  useLayoutEffect(() => {
    if (wasAsking.current && !asking) row.current?.focus({ preventScroll: true })
    wasAsking.current = asking
  }, [asking])
  if (asking) return <Ask question={t('ai:history.deleteAsk')} onConfirm={onDelete} onCancel={onCancel} />
  return (
    <div data-slot="history-item" className="group/row relative flex min-h-8 items-center rounded-md">
      <button
        ref={row}
        type="button"
        data-history-row
        data-id={item.id}
        aria-current={current || undefined}
        onClick={onOpen}
        onKeyDown={(event) => { if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); onAsk() } }}
        className={cn('flex min-w-0 flex-1 items-center gap-3 rounded-md px-2.5 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50', current && 'bg-muted')}
      >
        <span className="min-w-0 flex-1 truncate">{item.title.trim() || t('ai:history.untitled')}</span>
        <time dateTime={item.updatedAt} className="shrink-0 text-xs text-muted-foreground transition-opacity duration-100 group-focus-within/row:opacity-0 group-hover/row:opacity-0">
          {relativeTime(item.updatedAt, language)}
        </time>
      </button>
      <IconButton
        label={t('ai:history.delete')}
        size="icon-sm"
        onClick={onAsk}
        className="absolute right-1 text-muted-foreground opacity-0 transition-opacity duration-100 group-focus-within/row:opacity-100 group-hover/row:opacity-100"
      >
        <HugeiconsIcon icon={Delete02Icon} aria-hidden strokeWidth={2} />
      </IconButton>
    </div>
  )
}

function HistoryList({ client, chat, saveHistory, close, onStartOver, onOpened }: {
  client: MailxClient
  chat: AssistantChat
  saveHistory: boolean
  close: () => void
  onStartOver: () => void
  onOpened?: () => void
}) {
  const { i18n } = useTranslation()
  const queryClient = useQueryClient()
  const list = useRef<HTMLDivElement>(null)
  const [asking, setAsking] = useState<string | 'all' | null>(null)
  const query = useInfiniteQuery({
    queryKey: assistantKeys.conversations(),
    queryFn: ({ pageParam }) => client.assistantListConversations({ limit: HISTORY_PAGE, cursor: pageParam }),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.nextCursor,
    // The list is the core's to say: look again whenever the panel opens, and when the core says it changed.
    staleTime: 0,
    refetchOnMount: 'always',
  })
  const { refetch } = query
  useEffect(() => client.onAssistantConversationsChanged(() => { void refetch() }), [client, refetch])

  const items = useMemo(() => query.data?.pages.flatMap((page) => page.items) ?? [], [query.data])
  const grouped = useMemo(() => {
    const now = new Date()
    return GROUPS.map((group) => ({ group, items: items.filter((item) => groupOf(item.updatedAt, now) === group) })).filter((entry) => entry.items.length > 0)
  }, [items])

  // After a delete the focus goes to the row that took its place.
  const focusNext = useRef<string | null>(null)
  useEffect(() => {
    const id = focusNext.current
    if (id === null) return
    focusNext.current = null
    const rows = [...(list.current?.querySelectorAll<HTMLElement>('[data-history-row]') ?? [])]
    ;(rows.find((row) => row.dataset.id === id) ?? rows[0])?.focus({ preventScroll: true })
  }, [items])

  const open = useCallback(async (id: string) => {
    const result = await chat.openConversation(id)
    if (result === 'opened') {
      close()
      onOpened?.()
    }
    else if (result === 'failed') {
      toast.error(t('ai:history.openFailed'))
      void refetch()
    }
  }, [chat, close, onOpened, refetch])

  const remove = useCallback(async (id: string) => {
    const index = items.findIndex((item) => item.id === id)
    focusNext.current = (items[index + 1] ?? items[index - 1])?.id ?? null
    setAsking(null)
    try {
      await client.assistantDeleteConversation(id)
    } catch (error) {
      // Already gone is what was wanted.
      if (!(error instanceof MailxClientError && error.failure.code === 'notFound')) {
        focusNext.current = null
        toast.error(`${t('ai:history.deleteFailed')} ${errorText(error)}`)
        return
      }
    }
    await queryClient.invalidateQueries({ queryKey: assistantKeys.conversations() })
    // The conversation on screen is gone: what is left is an empty one.
    if (id === chat.conversationId) onStartOver()
  }, [chat.conversationId, client, items, onStartOver, queryClient])

  const clear = useCallback(async () => {
    setAsking(null)
    try {
      await client.assistantClearConversations()
    } catch (error) {
      toast.error(`${t('ai:history.deleteFailed')} ${errorText(error)}`)
      return
    }
    await queryClient.invalidateQueries({ queryKey: assistantKeys.conversations() })
    onStartOver()
  }, [client, onStartOver, queryClient])

  const loaded = query.data !== undefined
  return (
    <>
      {!saveHistory && (
        <p className="flex shrink-0 items-center justify-between gap-2 px-2.5 py-1 text-xs text-muted-foreground">
          <span>{t('ai:history.notSaved')}</span>
          <button type="button" onClick={() => { close(); requestSettings({ section: 'ai' }) }} className="rounded-sm text-foreground underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">
            {t('ai:history.turnOn')}
          </button>
        </p>
      )}
      <div
        ref={list}
        data-slot="history-list"
        onKeyDown={(event) => moveFocus(list.current, event)}
        className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain"
      >
        {query.isError && !loaded && (
          <p className="flex items-center justify-between gap-2 px-2.5 py-2 text-sm text-muted-foreground">
            <span>{t('ai:history.loadFailed')}</span>
            <Button type="button" variant="ghost" size="sm" onClick={() => { void refetch() }}>{t('ai:chat.retry')}</Button>
          </p>
        )}
        {loaded && items.length === 0 && <p className="px-2.5 py-6 text-center text-sm text-muted-foreground">{t('ai:history.empty')}</p>}
        {grouped.map(({ group, items: entries }) => (
          <section key={group} aria-label={t(`ai:history.${group}`)} className="flex flex-col pb-1">
            <h3 className="px-2.5 pt-2 pb-1 text-xs font-normal text-muted-foreground">{t(`ai:history.${group}`)}</h3>
            {entries.map((item) => (
              <Row
                key={item.id}
                item={item}
                current={item.id === chat.conversationId}
                asking={asking === item.id}
                language={i18n.language}
                onOpen={() => { void open(item.id) }}
                onAsk={() => setAsking(item.id)}
                onCancel={() => setAsking(null)}
                onDelete={() => { void remove(item.id) }}
              />
            ))}
          </section>
        ))}
        {query.hasNextPage && (
          <Button type="button" variant="ghost" size="lg" disabled={query.isFetchingNextPage} onClick={() => { void query.fetchNextPage() }} className="mt-1 shrink-0 self-center text-muted-foreground">
            {t('ai:history.loadMore')}
          </Button>
        )}
      </div>
      {items.length > 0 && (
        asking === 'all'
          ? <Ask question={t('ai:history.clearAsk')} onConfirm={() => { void clear() }} onCancel={() => setAsking(null)} className="mt-1 shrink-0" />
          : (
            <div className="mt-1 flex shrink-0 justify-end">
              <Button type="button" variant="ghost" size="sm" onClick={() => setAsking('all')} className="text-destructive hover:bg-destructive/10 hover:text-destructive">
                {t('ai:history.clearAll')}
              </Button>
            </div>
          )
      )}
    </>
  )
}

/**
 * The history button of the conversation header and the panel it opens (ai-chat-ui.md): saved
 * conversations, newest first under 今天 / 昨天 / 更早, one click to open one. A light popover, not a
 * dialog; Esc closes it and the focus goes back to the button. Deleting asks in the row itself.
 * The list is only mounted (and only fetched) while the panel is open.
 */
export function ChatHistory({ client, chat, onStartOver, onOpened, className }: {
  client: MailxClient
  chat: AssistantChat
  /** What 新对话 does; also what happens when the conversation on screen is deleted. */
  onStartOver: () => void
  /** Called after a conversation from the list is on screen, for a host that must then show it. */
  onOpened?: () => void
  /** Extra classes for the button, for a host that wants it quieter. */
  className?: string
}) {
  useTranslation()
  const [open, setOpen] = useState(false)
  const settings = useSettings(client)
  const saveHistory = settings.data?.settings.assistant.saveHistory ?? true
  const close = useCallback(() => setOpen(false), [])
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger render={
        <IconButton label={t('ai:history.title')} size="icon-lg" className={className}>
          <HugeiconsIcon icon={HistoryIcon} aria-hidden strokeWidth={2} />
        </IconButton>
      } />
      <PopoverContent align="end" aria-label={t('ai:history.title')} className="max-h-105 w-80 gap-0 p-1.5">
        <HistoryList client={client} chat={chat} saveHistory={saveHistory} close={close} onStartOver={onStartOver} onOpened={onOpened} />
      </PopoverContent>
    </Popover>
  )
}
