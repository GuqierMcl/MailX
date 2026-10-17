import { useEffect, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import type { Account, EmailDetail, EmailSummary, Folder } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { EmailList, type ListFilter } from './email-list'
import { EmailView } from './email-view'
import { emailQuery } from './format'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { SettingsDialog, type SettingsTarget } from '@/features/settings/settings-dialog'
import { MailSidebar } from './mail-sidebar'
import { mailboxTitle, type Mailbox } from './mailbox'

type Loadable<T> = { data: T; loading: boolean; error: string | null }

const SEARCH_DELAY_MS = 200

export function Workbench({ client }: { client: MailxClient }) {
  const [settings, setSettings] = useState<SettingsTarget | null>(null)
  const [treeVersion, setTreeVersion] = useState(0)
  const [tree, setTree] = useState<Loadable<{ accounts: Account[]; folders: Folder[] }>>({
    data: { accounts: [], folders: [] },
    loading: true,
    error: null,
  })
  const [mailbox, setMailbox] = useState<Mailbox>({ kind: 'unified', role: 'inbox' })
  const [filter, setFilter] = useState<ListFilter>('all')
  const [search, setSearch] = useState('')
  const [query, setQuery] = useState('')
  const [list, setList] = useState<Loadable<EmailSummary[]>>({ data: [], loading: true, error: null })
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [detail, setDetail] = useState<Loadable<EmailDetail | null>>({ data: null, loading: false, error: null })

  useEffect(() => {
    let active = true
    ;(async () => {
      try {
        const accounts = await client.listAccounts()
        const folders = (await Promise.all(accounts.map((account) => client.listFolders(account.id)))).flat()
        if (active) setTree({ data: { accounts, folders }, loading: false, error: null })
      } catch (cause) {
        if (active) setTree((current) => ({ ...current, loading: false, error: clientFailure(cause).message }))
      }
    })()
    return () => {
      active = false
    }
  }, [client, treeVersion])

  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [search])

  const folderId = mailbox.kind === 'folder' ? mailbox.folder.id : null
  const folderRole = mailbox.kind === 'unified' ? mailbox.role : null
  useEffect(() => {
    let active = true
    setList((current) => ({ ...current, loading: true, error: null }))
    client
      .listEmails(
        emailQuery({
          folderId,
          folderRole,
          unread: filter === 'unread' ? true : null,
          text: query || null,
        }),
      )
      .then(
        (page) => active && setList({ data: page.items, loading: false, error: null }),
        (cause) => active && setList({ data: [], loading: false, error: clientFailure(cause).message }),
      )
    return () => {
      active = false
    }
  }, [client, folderId, folderRole, filter, query])

  useEffect(() => {
    if (!selectedId) return
    let active = true
    setDetail({ data: null, loading: true, error: null })
    client.getEmail(selectedId).then(
      (email) => active && setDetail({ data: email, loading: false, error: null }),
      (cause) => active && setDetail({ data: null, loading: false, error: clientFailure(cause).message }),
    )
    return () => {
      active = false
    }
  }, [client, selectedId])

  const { accounts, folders } = tree.data
  const dialog = (
    <SettingsDialog
      client={client}
      target={settings}
      onClose={() => setSettings(null)}
      onAccountsChanged={() => setTreeVersion((version) => version + 1)}
    />
  )
  if (!tree.loading && !tree.error && accounts.length === 0) {
    return (
      <>
        <NoAccounts onAddAccount={() => setSettings({ section: 'accounts', newAccount: true })} />
        {dialog}
      </>
    )
  }
  const showAccountBadges = mailbox.kind === 'unified' && accounts.length > 1

  const selectMailbox = (next: Mailbox) => {
    setMailbox(next)
    setSelectedId(null)
    setDetail({ data: null, loading: false, error: null })
  }

  return (
    <TooltipProvider>
      <SidebarProvider className="h-full min-h-0">
        <MailSidebar
          accounts={accounts}
          folders={folders}
          loading={tree.loading}
          mailbox={mailbox}
          onSelect={selectMailbox}
          onOpenSettings={() => setSettings({ section: 'general' })}
        />
        <TitleBarActions side="leading">
          <SidebarTrigger aria-label="折叠或展开侧边栏" />
        </TitleBarActions>
        <SidebarInset className="min-h-0 min-w-0">
          <ResizablePanelGroup orientation="horizontal" className="min-h-0">
            <ResizablePanel defaultSize="40" minSize={300}>
              <EmailList
                title={mailboxTitle(mailbox)}
                emails={list.data}
                accounts={showAccountBadges ? accounts : null}
                loading={list.loading}
                error={tree.error ?? list.error}
                filter={filter}
                search={search}
                selectedId={selectedId}
                onFilterChange={setFilter}
                onSearchChange={setSearch}
                onSelect={(email) => setSelectedId(email.id)}
              />
            </ResizablePanel>
            <ResizableHandle withHandle />
            <ResizablePanel defaultSize="60" minSize={360}>
              <EmailView email={selectedId ? detail.data : null} loading={detail.loading} error={detail.error} />
            </ResizablePanel>
          </ResizablePanelGroup>
        </SidebarInset>
      </SidebarProvider>
      {dialog}
    </TooltipProvider>
  )
}

function NoAccounts({ onAddAccount }: { onAddAccount: () => void }) {
  return (
    <Empty className="h-full">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <HugeiconsIcon icon={Mail01Icon} />
        </EmptyMedia>
        <EmptyTitle>还没有邮箱账号</EmptyTitle>
        <EmptyDescription>添加一个邮箱后，邮件会显示在这里。</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onAddAccount}>
          <HugeiconsIcon icon={Add01Icon} />
          添加邮箱账号
        </Button>
      </EmptyContent>
    </Empty>
  )
}
