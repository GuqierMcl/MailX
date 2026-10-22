import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { InformationCircleIcon, PaintBoardIcon, Shield01Icon } from '@hugeicons/core-free-icons'
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
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'

export type SettingsSection = 'appearance' | 'security' | 'about'
/** Where the dialog opens. */
export type SettingsTarget = { section: SettingsSection }

const SECTIONS: { id: SettingsSection; name: string; icon: IconSvgElement }[] = [
  { id: 'appearance', name: '外观', icon: PaintBoardIcon },
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
        <DialogDescription className="sr-only">管理 MailX 的外观与安全性。</DialogDescription>
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
      <SettingsGroup title="数据与凭证">
        <SettingsRow label="本机数据" description="账号配置、邮件缓存与偏好设置（settings.json）都保存在本机的数据目录中。" />
        <SettingsRow label="密码" description="密码由系统凭据库管理，不会显示在应用中。" />
      </SettingsGroup>
    </SettingsPage>
  )
}
