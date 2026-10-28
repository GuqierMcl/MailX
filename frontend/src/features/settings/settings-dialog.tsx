import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { ComputerIcon, InformationCircleIcon, Notification03Icon, PaintBoardIcon, PencilEdit02Icon, Shield01Icon } from '@hugeicons/core-free-icons'
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
import { GeneralPane } from './general-pane'
import { AutoCheckRow, UpdateStatusRow } from '@/features/updater/about-updates'

export type SettingsSection = 'general' | 'appearance' | 'notifications' | 'compose' | 'security' | 'about'
/** Where the dialog opens. */
export type SettingsTarget = { section: SettingsSection }



type SettingsDialogProps = {
  client: MailxClient
  target: SettingsTarget | null
  onClose: () => void
}

/** Application preferences. Email accounts have their own dialog. */
export function SettingsDialog({ client, target, onClose }: SettingsDialogProps) {
  useTranslation()
  const SECTIONS: { id: SettingsSection; name: string; icon: IconSvgElement }[] = [
    { id: 'general', name: t('settings:general'), icon: ComputerIcon },
    { id: 'appearance', name: t('settings:appearance'), icon: PaintBoardIcon },
    { id: 'notifications', name: t('settings:notifications'), icon: Notification03Icon },
    { id: 'compose', name: t('settings:compose'), icon: PencilEdit02Icon },
    { id: 'security', name: t('settings:security'), icon: Shield01Icon },
    { id: 'about', name: t('settings:about'), icon: InformationCircleIcon },
  ]
  const [section, setSection] = useState<SettingsSection>('general')

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
        <DialogDescription className="sr-only">{t('settings:dialogDescription')}</DialogDescription>
        {/* Narrow windows turn the side navigation into a top strip instead of hiding it. */}
        <SidebarProvider className="grid min-h-0 flex-1 grid-cols-1 grid-rows-[auto_minmax(0,1fr)_auto] md:grid-cols-[var(--sidebar-width)_minmax(0,1fr)] md:grid-rows-[minmax(0,1fr)_auto]">
          <Sidebar collapsible="none"
            className="h-auto w-full shrink-0 flex-row items-center border-b pr-12 md:col-start-1 md:row-start-1 md:h-full md:w-(--sidebar-width) md:flex-col md:items-stretch md:border-b-0 md:pr-0">
            <SidebarHeader className="shrink-0 px-4 py-3 md:pt-5 md:pb-1">
              <DialogTitle className="text-base font-semibold">{t('settings:title')}</DialogTitle>
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
          <main className="flex min-h-0 min-w-0 flex-col overflow-hidden md:col-start-2 md:row-span-2 md:row-start-1">
            <header className="flex h-14 shrink-0 items-center gap-2 px-6 md:h-16 md:pr-12">
              <h2 className="text-sm font-medium">{sectionName}</h2>
            </header>
            <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
              {section === 'general' && <GeneralPane client={client} />}
              {section === 'appearance' && <AppearancePane client={client} />}
              {section === 'notifications' && <NotificationsPane client={client} />}
              {section === 'compose' && <ComposePane client={client} />}
              {section === 'security' && <SecurityPane client={client} />}
              {section === 'about' && <AboutSection client={client} />}
            </div>
          </main>
          <footer className="flex min-w-0 flex-col gap-0.5 bg-sidebar px-4 pt-3 pb-4 md:col-start-1 md:row-start-2">
            <p className="text-sm font-medium text-sidebar-foreground/70">MailX</p>
            <p className="text-xs break-all text-muted-foreground tabular-nums">{__APP_VERSION__}</p>
          </footer>
        </SidebarProvider>
      </DialogContent>
    </Dialog>
  )
}

function AboutSection({ client }: { client: MailxClient }) {
  useTranslation()
  return (
    <SettingsPage>
      <SettingsGroup title="MailX">
        <SettingsRow label={t('settings:version')}>
          <span className="text-sm text-muted-foreground tabular-nums">{__APP_VERSION__}</span>
        </SettingsRow>
        <UpdateStatusRow />
        <AutoCheckRow client={client} />
      </SettingsGroup>
      <SettingsGroup title={t('settings:privacy')} description={t('settings:privacyDescription')}>
        <SettingsRow label={t('settings:dataLocation')} description={t('settings:dataLocationDescription')} />
        <SettingsRow label={t('settings:password')} description={t('settings:passwordDescription')} />
        <SettingsRow label={t('settings:network')} description={t('settings:networkDescription')} />
        <SettingsRow label={t('settings:telemetry')} description={t('settings:telemetryDescription')} />
        <SettingsRow label={t('settings:mailChanges')} description={t('settings:mailChangesDescription')} />
        <SettingsRow label={t('settings:messageContent')} description={t('settings:messageContentDescription')} />
      </SettingsGroup>
    </SettingsPage>
  )
}
