import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { Add01Icon, ArrowRight01Icon, InformationCircleIcon, Mail01Icon, Settings01Icon, UserMultipleIcon } from '@hugeicons/core-free-icons'
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
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldSet, FieldTitle } from '@/components/ui/field'
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from '@/components/ui/sidebar'
import { Skeleton } from '@/components/ui/skeleton'
import { AccountForm } from '@/features/accounts/account-form'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { useTheme, type ThemePreference } from '@/lib/theme'

export type SettingsSection = 'general' | 'accounts' | 'about'
/** Where the dialog opens; `newAccount` goes straight to an empty account form. */
export type SettingsTarget = { section: SettingsSection; newAccount?: boolean }

const SECTIONS: { id: SettingsSection; name: string; icon: IconSvgElement }[] = [
  { id: 'general', name: '通用', icon: Settings01Icon },
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
  const [section, setSection] = useState<SettingsSection>('general')
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
        <DialogContent className="overflow-hidden p-0 md:max-h-[640px] md:max-w-[760px] lg:max-w-[880px]">
          <DialogTitle className="sr-only">设置</DialogTitle>
          <DialogDescription className="sr-only">管理 MailX 的外观与邮箱账号。</DialogDescription>
          <SidebarProvider className="min-h-0 items-start">
            <Sidebar collapsible="none" className="hidden md:flex">
              <SidebarContent>
                <SidebarGroup>
                  <SidebarGroupContent>
                    <SidebarMenu>
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
            <main className="flex h-[600px] flex-1 flex-col overflow-hidden">
              <header className="flex h-16 shrink-0 items-center gap-2 px-6">
                <Breadcrumb>
                  <BreadcrumbList>
                    <BreadcrumbItem className="hidden md:block">设置</BreadcrumbItem>
                    <BreadcrumbSeparator className="hidden md:block" />
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
                        <BreadcrumbPage>{sectionName}</BreadcrumbPage>
                      </BreadcrumbItem>
                    )}
                  </BreadcrumbList>
                </Breadcrumb>
              </header>
              <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
                {section === 'general' && <GeneralSection />}
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

function SectionBody({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-6 px-6 pb-6">{children}</div>
}

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: 'system', label: '跟随系统' },
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
]

function GeneralSection() {
  const [theme, setTheme] = useTheme()
  return (
    <SectionBody>
      <FieldSet>
        <FieldGroup>
          <Field>
            <FieldTitle>外观</FieldTitle>
            <FieldDescription>选择界面的颜色主题。</FieldDescription>
            <RadioGroup value={theme} onValueChange={(value) => setTheme(value as ThemePreference)} aria-label="外观">
              {THEMES.map((option) => (
                <Field key={option.value} orientation="horizontal">
                  <RadioGroupItem id={`theme-${option.value}`} value={option.value} />
                  <FieldLabel htmlFor={`theme-${option.value}`}>{option.label}</FieldLabel>
                </Field>
              ))}
            </RadioGroup>
          </Field>
        </FieldGroup>
      </FieldSet>
    </SectionBody>
  )
}

function AboutSection() {
  return (
    <SectionBody>
      <Field>
        <FieldContent>
          <FieldTitle>MailX</FieldTitle>
          <FieldDescription>版本 {__APP_VERSION__}</FieldDescription>
        </FieldContent>
      </Field>
      <Field>
        <FieldContent>
          <FieldTitle>数据与凭证</FieldTitle>
          <FieldDescription>账号配置保存在本机，密码由系统凭据库管理，不会显示在应用中。</FieldDescription>
        </FieldContent>
      </Field>
    </SectionBody>
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
    if (loadFailure) return <SectionBody>{loadFailure}</SectionBody>
    if (editing && !account) {
      return (
        <SectionBody>
          <div role="status" aria-label="正在加载账号配置" className="flex flex-col gap-3">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-48 w-full" />
          </div>
        </SectionBody>
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
    <SectionBody>
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
        <>
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
        </>
      )}
    </SectionBody>
  )
}
