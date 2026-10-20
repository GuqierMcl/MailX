import { useCallback, useEffect, useRef, useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { Add01Icon, ArrowRight01Icon, InformationCircleIcon, Mail01Icon, PaintBoardIcon, UserMultipleIcon } from '@hugeicons/core-free-icons'
import type { Account } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/components/ui/breadcrumb'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from '@/components/ui/sidebar'
import { Skeleton } from '@/components/ui/skeleton'
import { AccountForm } from '@/features/accounts/account-form'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { AppearancePane } from './appearance-pane'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'

export type SettingsSection = 'appearance' | 'accounts' | 'about'
/** Where the dialog opens; `newAccount` goes straight to an empty account form. */
export type SettingsTarget = { section: SettingsSection; newAccount?: boolean }

const SECTIONS: { id: SettingsSection; name: string; icon: IconSvgElement }[] = [
  { id: 'appearance', name: '外观', icon: PaintBoardIcon },
  { id: 'accounts', name: '账号', icon: UserMultipleIcon },
  { id: 'about', name: '关于', icon: InformationCircleIcon },
]

type SettingsDialogProps = {
  client: MailxClient
  target: SettingsTarget | null
  onClose: () => void
  onAccountsChanged: () => void
}

/** Application settings. Unsaved account edits are confirmed before any navigation or close. */
export function SettingsDialog({ client, target, onClose, onAccountsChanged }: SettingsDialogProps) {
  const [section, setSection] = useState<SettingsSection>('appearance')
  // `undefined`: account list; `null`: new account; otherwise the account id being edited.
  const [editing, setEditing] = useState<string | null | undefined>(undefined)
  const [editingName, setEditingName] = useState<string | null>(null)
  const [dirty, setDirty] = useState(false)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<(() => void) | null>(null)

  // Reset the view whenever the dialog is opened with a new target.
  const [openedFor, setOpenedFor] = useState<SettingsTarget | null>(null)
  if (target !== openedFor) {
    setOpenedFor(target)
    if (target) {
      setSection(target.section)
      setEditing(target.newAccount ? null : undefined)
      setDirty(false)
    }
  }

  const guard = (action: () => void) => {
    if (busy) return
    if (dirty) setPending(() => action)
    else action()
  }
  const leaveForm = (action: () => void) =>
    guard(() => {
      setDirty(false)
      action()
    })

  const sectionName = SECTIONS.find((item) => item.id === section)?.name
  const formTitle = editing === null ? '添加邮箱账号' : editing ? editingName : null

  return (
    <>
      <Dialog open={target !== null} onOpenChange={(open) => !open && leaveForm(onClose)}>
        <DialogContent className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[760px] lg:max-w-[880px]">
          <DialogDescription className="sr-only">管理 MailX 的外观与邮箱账号。</DialogDescription>
          {/* Narrow windows turn the side navigation into a top strip instead of hiding it. */}
          <SidebarProvider className="min-h-0 flex-1 flex-col md:flex-row">
            <Sidebar collapsible="none"
              className="h-auto w-full shrink-0 flex-row items-center border-b pr-12 md:h-full md:w-(--sidebar-width) md:flex-col md:items-stretch md:border-b-0 md:pr-0">
              <SidebarHeader className="shrink-0 px-4 py-3 md:pt-5 md:pb-1">
                <DialogTitle className="text-base font-semibold">设置</DialogTitle>
              </SidebarHeader>
              <SidebarContent>
                <SidebarGroup className="py-2 md:p-2">
                  <SidebarGroupContent>
                    <SidebarMenu className="flex-row md:flex-col">
                      {SECTIONS.map((item) => (
                        <SidebarMenuItem key={item.id}>
                          <SidebarMenuButton
                            isActive={item.id === section}
                            onClick={() =>
                              leaveForm(() => {
                                setSection(item.id)
                                setEditing(undefined)
                              })
                            }
                          >
                            <HugeiconsIcon icon={item.icon} />
                            <span>{item.name}</span>
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      ))}
                    </SidebarMenu>
                  </SidebarGroupContent>
                </SidebarGroup>
              </SidebarContent>
            </Sidebar>
            <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <header className="flex h-14 shrink-0 items-center gap-2 px-6 md:h-16 md:pr-12">
                <Breadcrumb>
                  <BreadcrumbList className="text-sm">
                    {formTitle ? (
                      <>
                        <BreadcrumbItem>
                          <BreadcrumbLink render={<button type="button" />} onClick={() => leaveForm(() => setEditing(undefined))}>
                            {sectionName}
                          </BreadcrumbLink>
                        </BreadcrumbItem>
                        <BreadcrumbSeparator />
                        <BreadcrumbItem>
                          <BreadcrumbPage>{formTitle}</BreadcrumbPage>
                        </BreadcrumbItem>
                      </>
                    ) : (
                      <BreadcrumbItem>
                        <BreadcrumbPage className="font-medium">{sectionName}</BreadcrumbPage>
                      </BreadcrumbItem>
                    )}
                  </BreadcrumbList>
                </Breadcrumb>
              </header>
              <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
                {section === 'appearance' && <AppearancePane client={client} />}
                {section === 'accounts' && (
                  <AccountsSection
                    client={client}
                    editing={editing}
                    onEdit={(id) => setEditing(id)}
                    onEditingName={setEditingName}
                    onBack={() => leaveForm(() => setEditing(undefined))}
                    onDirtyChange={setDirty}
                    onBusyChange={setBusy}
                    onSaved={onAccountsChanged}
                  />
                )}
                {section === 'about' && <AboutSection />}
              </div>
            </main>
          </SidebarProvider>
        </DialogContent>
      </Dialog>
      <AlertDialog open={pending !== null} onOpenChange={(open) => !open && setPending(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>放弃未保存的修改？</AlertDialogTitle>
            <AlertDialogDescription>当前表单的修改和已输入的密码将被清除。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                pending?.()
                setPending(null)
              }}
            >
              放弃修改
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function AboutSection() {
  return (
    <SettingsPage>
      <SettingsGroup title="MailX">
        <SettingsRow label="版本">
          <span className="text-sm text-muted-foreground tabular-nums">{__APP_VERSION__}</span>
        </SettingsRow>
      </SettingsGroup>
      <SettingsGroup title="数据与凭证">
        <SettingsRow label="本机数据" description="账号配置、邮件缓存与偏好设置（settings.json）都保存在本机的数据目录中。" />
        <SettingsRow label="密码" description="密码由系统凭据库管理，不会显示在应用中。" />
      </SettingsGroup>
    </SettingsPage>
  )
}

type AccountsSectionProps = {
  client: MailxClient
  editing: string | null | undefined
  onEdit: (id: string | null) => void
  onEditingName: (name: string | null) => void
  onBack: () => void
  onDirtyChange: (dirty: boolean) => void
  onBusyChange: (busy: boolean) => void
  onSaved: () => void
}

function AccountsSection({ client, editing, onEdit, onEditingName, onBack, onDirtyChange, onBusyChange, onSaved }: AccountsSectionProps) {
  const [accounts, setAccounts] = useState<Account[] | null>(null)
  const [account, setAccount] = useState<Account | null>(null)
  const [error, setError] = useState<ClientFailure | null>(null)
  const [formVersion, setFormVersion] = useState(0)
  const sequence = useRef(0)

  const loadList = useCallback(async () => {
    const current = ++sequence.current
    setError(null)
    try {
      const rows = await client.listAccounts()
      if (current === sequence.current) setAccounts(rows)
    } catch (cause) {
      if (current === sequence.current) setError(clientFailure(cause))
    }
  }, [client])

  const loadAccount = useCallback(
    async (id: string) => {
      const current = ++sequence.current
      setAccount(null)
      setError(null)
      try {
        const detail = await client.getAccount(id)
        if (current !== sequence.current) return
        setAccount(detail)
        onEditingName(detail.displayName)
        setFormVersion((version) => version + 1)
      } catch (cause) {
        if (current === sequence.current) setError(clientFailure(cause))
      }
    },
    [client, onEditingName],
  )

  useEffect(() => {
    // Load outside the effect body; the sequence counter drops stale responses.
    void Promise.resolve().then(() => {
      if (editing === undefined) void loadList()
      else if (editing) void loadAccount(editing)
    })
  }, [editing, loadList, loadAccount])

  const loadFailure = error && (
    <Alert variant="destructive">
      <AlertTitle>无法加载账号配置</AlertTitle>
      <AlertDescription>{error.message}</AlertDescription>
      <Button variant="outline" className="mt-3 w-fit" onClick={() => void (editing ? loadAccount(editing) : loadList())}>
        重试加载
      </Button>
    </Alert>
  )

  if (editing !== undefined) {
    if (loadFailure) return <SettingsPage>{loadFailure}</SettingsPage>
    if (editing && !account) {
      return (
        <SettingsPage>
          <div role="status" aria-label="正在加载账号配置" className="flex flex-col gap-3">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-48 w-full" />
          </div>
        </SettingsPage>
      )
    }
    return (
      <AccountForm
        key={`${editing ?? 'new'}:${formVersion}`}
        account={editing ? account : null}
        client={client}
        onBusyChange={onBusyChange}
        onDirtyChange={onDirtyChange}
        onReload={() => (editing ? void loadAccount(editing) : onBack())}
        onSaved={(saved) => {
          onDirtyChange(false)
          onSaved()
          setAccount(saved)
          onEditingName(saved.displayName)
          // A new account continues as an edit of the saved record.
          onEdit(saved.id)
        }}
      />
    )
  }

  return (
    <SettingsPage>
      {loadFailure}
      {!accounts && !error && (
        <div role="status" aria-label="正在加载账号配置" className="flex flex-col gap-2">
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
        </div>
      )}
      {accounts?.length === 0 && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={Mail01Icon} />
            </EmptyMedia>
            <EmptyTitle>从添加一个邮箱开始</EmptyTitle>
            <EmptyDescription>分别配置收信、发信和网络路由。设置保存在本机，密码交由系统凭据库管理。</EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button onClick={() => onEdit(null)}>
              <HugeiconsIcon icon={Add01Icon} />
              添加第一个账号
            </Button>
          </EmptyContent>
        </Empty>
      )}
      {accounts && accounts.length > 0 && (
        <SettingsGroup title="邮箱账号" description="选择一个账号查看或修改收信、发信与网络路由配置。">
          <ItemGroup className="gap-2" aria-label="邮箱账号列表">
            {accounts.map((row) => (
              <Item key={row.id} variant="outline" render={<button type="button" onClick={() => onEdit(row.id)} />}>
                <ItemMedia>
                  <Avatar>
                    <AvatarFallback>{Array.from(row.displayName)[0]?.toUpperCase()}</AvatarFallback>
                  </Avatar>
                </ItemMedia>
                <ItemContent>
                  <ItemTitle>{row.displayName}</ItemTitle>
                  <ItemDescription>{row.emailAddress}</ItemDescription>
                </ItemContent>
                <ItemActions>
                  <HugeiconsIcon icon={ArrowRight01Icon} className="size-4 text-muted-foreground" />
                </ItemActions>
              </Item>
            ))}
          </ItemGroup>
          <Button variant="outline" className="w-fit" onClick={() => onEdit(null)}>
            <HugeiconsIcon icon={Add01Icon} />
            添加账号
          </Button>
        </SettingsGroup>
      )}
    </SettingsPage>
  )
}
