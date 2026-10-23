import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { InformationCircleIcon, Notification03Icon, PaintBoardIcon, PencilEdit02Icon, Shield01Icon } from '@hugeicons/core-free-icons'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
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
import type { MailxClient } from '@/lib/mailx-client'
import { SecurityPane } from './security-pane'
import { AppearancePane } from './appearance-pane'
import { NotificationsPane } from './notifications-pane'
import { ComposePane } from './compose-pane'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'

export type SettingsSection = 'appearance' | 'notifications' | 'compose' | 'security' | 'about'
/** Where the dialog opens. */
export type SettingsTarget = { section: SettingsSection }

const SECTIONS: { id: SettingsSection; name: string; icon: IconSvgElement }[] = [
  { id: 'appearance', name: '外观', icon: PaintBoardIcon },
  { id: 'notifications', name: '通知', icon: Notification03Icon },
  { id: 'compose', name: '发信', icon: PencilEdit02Icon },
  { id: 'security', name: '安全性', icon: Shield01Icon },
  { id: 'about', name: '关于', icon: InformationCircleIcon },
]

type SettingsDialogProps = {
  client: MailxClient
  target: SettingsTarget | null
  onClose: () => void
}

/** Application preferences. Email accounts have their own dialog. */
export function SettingsDialog({ client, target, onClose }: SettingsDialogProps) {
  const [section, setSection] = useState<SettingsSection>('appearance')

  // Reset the view whenever the dialog is opened with a new target.
  const [openedFor, setOpenedFor] = useState<SettingsTarget | null>(null)
  if (target !== openedFor) {
    setOpenedFor(target)
    if (target) setSection(target.section)
  }

  const sectionName = SECTIONS.find((item) => item.id === section)?.name

  return (
    <Dialog open={target !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[760px] lg:max-w-[880px]">
        <DialogDescription className="sr-only">管理 MailX 的外观、通知、发信与安全性。</DialogDescription>
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
                        <SidebarMenuButton isActive={item.id === section} onClick={() => setSection(item.id)}>
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
              <h2 className="text-sm font-medium">{sectionName}</h2>
            </header>
            <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
              {section === 'appearance' && <AppearancePane client={client} />}
              {section === 'notifications' && <NotificationsPane client={client} />}
              {section === 'compose' && <ComposePane client={client} />}
              {section === 'security' && <SecurityPane client={client} />}
              {section === 'about' && <AboutSection />}
            </div>
          </main>
        </SidebarProvider>
      </DialogContent>
    </Dialog>
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
      <SettingsGroup title="数据与隐私" description="MailX 是在你电脑上运行的邮件应用，没有自己的服务器，也不需要注册 MailX 账号。">
        <SettingsRow label="邮件和设置保存在哪里" description="账号信息、邮件内容、已保存的图片和各项设置，都只保存在这台电脑上，不会上传到任何地方。" />
        <SettingsRow label="密码" description="你的邮箱密码交给系统保管（Mac 的钥匙串、Windows 的凭据管理器或 Linux 的密钥服务），不会写进 MailX 自己的文件，界面上也不会再显示。" />
        <SettingsRow label="MailX 会联网做什么" description="连接你自己的邮箱服务器，收取邮件；在你允许后加载邮件里的网络图片；在你开启后查找发件人头像。除此之外 MailX 不会主动连接其他地址。点开邮件里的链接时，是由你的系统浏览器打开的。" />
        <SettingsRow label="不收集你的使用情况" description="MailX 不包含统计、数据分析或崩溃上报，也不会在后台自动检查或下载更新。" />
        <SettingsRow label="对你邮箱里的邮件" description="MailX 只会把“已读”和“星标”的变化同步回邮箱服务器，不会删除、移动或改写你的邮件。" />
        <SettingsRow label="邮件里的内容" description="邮件在一个与应用隔离的区域里显示，邮件里的脚本不会运行，也无法接触你的账号、密码和设置。网络图片默认每次询问。" />
      </SettingsGroup>
    </SettingsPage>
  )
}
