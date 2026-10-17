import { useCallback, useEffect, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon, Settings01Icon } from '@hugeicons/core-free-icons'
import type { Account } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { FieldContent, FieldDescription, FieldGroup } from '@/components/ui/field'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupLabel, SidebarHeader, SidebarInset, SidebarMenu, SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger, useSidebar } from '@/components/ui/sidebar'
import { Skeleton } from '@/components/ui/skeleton'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { AccountForm } from './account-form'

export function AccountsPage({ client }: { client: MailxClient }) {
  return <TooltipProvider><SidebarProvider className="h-full min-h-0"><AccountWorkspace client={client} /></SidebarProvider></TooltipProvider>
}

function AccountWorkspace({ client }: { client: MailxClient }) {
  const [accounts, setAccounts] = useState<Account[]>([])
  const [account, setAccount] = useState<Account | null>(null)
  const [editing, setEditing] = useState(false)
  const [loading, setLoading] = useState(client.available)
  const [error, setError] = useState<ClientFailure | null>(null)
  const [success, setSuccess] = useState(false)
  const [busy, setBusy] = useState(false)
  const [dirty, setDirty] = useState(false)
  const [editorVersion, setEditorVersion] = useState(0)
  const [navigation, setNavigation] = useState<(() => void) | null>(null)
  const sequence = useRef(0)
  const { setOpenMobile } = useSidebar()

  const load = useCallback(async (preferredId?: string) => {
    if (!client.available) return
    const current = ++sequence.current
    setLoading(true)
    setError(null)
    try {
      const rows = await client.listAccounts()
      const id = rows.find((row) => row.id === preferredId)?.id ?? rows[0]?.id
      const detail = id ? await client.getAccount(id) : null
      if (current !== sequence.current) return
      setAccounts(rows)
      setAccount(detail)
      setEditing(!!detail)
      setDirty(false)
      setSuccess(false)
      setEditorVersion((version) => version + 1)
    } catch (cause) {
      if (current === sequence.current) setError(clientFailure(cause))
    } finally {
      if (current === sequence.current) setLoading(false)
    }
  }, [client])

  useEffect(() => {
    let active = true
    void Promise.resolve().then(() => { if (active) void load() })
    const invalidatePending = () => { sequence.current++ }
    return () => { active = false; invalidatePending() }
  }, [load])

  function guard(action: () => void) {
    if (busy) return
    if (dirty) setNavigation(() => action)
    else action()
  }

  function newAccount() {
    guard(() => {
      sequence.current++
      setLoading(false)
      setAccount(null)
      setEditing(true)
      setError(null)
      setSuccess(false)
      setDirty(false)
      setEditorVersion((version) => version + 1)
      setOpenMobile(false)
    })
  }

  async function selectAccount(id: string) {
    const current = ++sequence.current
    setLoading(true)
    setError(null)
    setOpenMobile(false)
    try {
      const detail = await client.getAccount(id)
      if (current !== sequence.current) return
      setAccount(detail)
      setEditing(true)
      setSuccess(false)
      setDirty(false)
      setEditorVersion((version) => version + 1)
    } catch (cause) {
      if (current === sequence.current) setError(clientFailure(cause))
    } finally {
      if (current === sequence.current) setLoading(false)
    }
  }

  function saved(value: Account) {
    setAccounts((current) => current.some((item) => item.id === value.id)
      ? current.map((item) => item.id === value.id ? value : item)
      : [...current, value])
    setAccount(value)
    setEditing(true)
    setDirty(false)
    setSuccess(true)
    setEditorVersion((version) => version + 1)
  }

  return <>
    <Sidebar className="account-sidebar" collapsible="offcanvas">
      <SidebarHeader className="brand-header">
        <Avatar className="brand-avatar"><AvatarImage src={`${import.meta.env.BASE_URL}logo.svg`} alt="" /><AvatarFallback>MX</AvatarFallback></Avatar>
        <FieldContent><CardTitle className="brand-name">MailX</CardTitle><CardDescription>你的本地邮件空间</CardDescription></FieldContent>
      </SidebarHeader>
      <SidebarGroup>
        <SidebarGroupLabel className="section-label"><HugeiconsIcon icon={Settings01Icon} size={16} />账号设置</SidebarGroupLabel>
        <Button onClick={newAccount} disabled={!client.available || busy || loading} className="add-account-button"><HugeiconsIcon icon={Add01Icon} size={16} />添加账号</Button>
      </SidebarGroup>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel className="account-list-label">邮箱账号 <Badge variant="secondary">{accounts.length}</Badge></SidebarGroupLabel>
          <SidebarMenu aria-label="邮箱账号列表">
            {accounts.map((item) => <SidebarMenuItem key={item.id}>
              <SidebarMenuButton size="lg" isActive={account?.id === item.id} disabled={busy || loading} className="account-list-item"
                aria-current={account?.id === item.id ? 'page' : undefined}
                onClick={() => guard(() => { void selectAccount(item.id) })}>
                <Avatar className="account-avatar"><AvatarFallback>{item.displayName.slice(0, 1).toUpperCase()}</AvatarFallback></Avatar>
                <FieldContent className="min-w-0"><CardTitle className="truncate text-sm">{item.displayName}</CardTitle><CardDescription className="truncate">{item.emailAddress}</CardDescription></FieldContent>
              </SidebarMenuButton>
            </SidebarMenuItem>)}
          </SidebarMenu>
          {!loading && accounts.length === 0 && <FieldDescription className="sidebar-empty">添加的邮箱会显示在这里。</FieldDescription>}
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="sidebar-footer">
        <Separator />
        <FieldDescription>配置保存在本机 · 密码由系统凭据库管理</FieldDescription>
      </SidebarFooter>
    </Sidebar>
    <SidebarInset className="workspace-main">
      <Card className="workspace-topbar">
        <CardHeader className="topbar-content">
          <SidebarTrigger aria-label="展开或收起账号列表" />
          <CardTitle className="text-sm">账号设置</CardTitle>
          <Button variant="ghost" className="ml-auto" disabled={!client.available || busy || loading} onClick={() => guard(() => { void load(account?.id) })}>重新加载</Button>
        </CardHeader>
      </Card>
      <ScrollArea className="workspace-scroll">
        <CardContent className="workspace-content">
          {!client.available ? <Empty className="welcome-empty">
            <EmptyHeader><EmptyMedia variant="icon"><HugeiconsIcon icon={Mail01Icon} size={24} /></EmptyMedia><EmptyTitle className="text-xl">在桌面应用中管理邮箱</EmptyTitle><EmptyDescription>当前页面没有连接 MailX 桌面宿主，无法读取或保存账号配置。</EmptyDescription></EmptyHeader>
            <EmptyContent><Alert role="status"><AlertTitle>桌面能力不可用</AlertTitle><AlertDescription>打开 MailX 桌面应用后，可在这里添加邮箱并安全保存凭证。</AlertDescription></Alert></EmptyContent>
          </Empty> : <>
            {error && <Alert variant="destructive" className="mb-5"><AlertTitle>无法加载账号配置</AlertTitle><AlertDescription>{error.message}</AlertDescription><Button variant="outline" className="mt-3 w-fit" onClick={() => guard(() => { void load(account?.id) })}>重试加载</Button></Alert>}
            {success && <Alert className="success-note" role="status"><AlertTitle>已保存到本机</AlertTitle><AlertDescription>连接尚未验证。你可以继续修改账号设置。</AlertDescription></Alert>}
            {loading ? <FieldGroup aria-label="正在加载账号配置" role="status" className="loading-placeholder"><Skeleton className="h-8 w-48" /><Skeleton className="h-24 w-full" /><Skeleton className="h-64 w-full" /><FieldDescription>正在读取本地配置…</FieldDescription></FieldGroup>
              : editing ? <AccountForm key={`${account?.id ?? 'new'}:${editorVersion}`} account={account} client={client} onSaved={saved} onBusyChange={setBusy}
                  onDirtyChange={(value) => { setDirty(value); if (value) setSuccess(false) }} onReload={() => guard(() => { void load(account?.id) })} />
              : !error && <Empty className="welcome-empty">
                  <EmptyHeader><EmptyMedia variant="icon"><HugeiconsIcon icon={Mail01Icon} size={24} /></EmptyMedia><EmptyTitle className="text-xl">从添加一个邮箱开始</EmptyTitle><EmptyDescription>分别配置收信、发信和网络路由。设置保存在本机，密码交由系统凭据库管理。</EmptyDescription></EmptyHeader>
                  <EmptyContent><Button onClick={newAccount} size="lg"><HugeiconsIcon icon={Add01Icon} size={16} />添加第一个账号</Button><FieldDescription className="text-center">当前版本支持保存配置，尚不连接邮箱。</FieldDescription></EmptyContent>
                </Empty>}
          </>}
        </CardContent>
      </ScrollArea>
    </SidebarInset>
    <AlertDialog open={navigation !== null} onOpenChange={(open) => { if (!open) setNavigation(null) }}>
      <AlertDialogContent><AlertDialogHeader><AlertDialogTitle>放弃未保存的修改？</AlertDialogTitle><AlertDialogDescription>当前表单的修改和已输入的密码将被清除。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel>继续编辑</AlertDialogCancel><AlertDialogAction onClick={() => { navigation?.(); setNavigation(null) }}>放弃修改</AlertDialogAction></AlertDialogFooter></AlertDialogContent>
    </AlertDialog>
  </>
}
