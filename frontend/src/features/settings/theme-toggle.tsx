import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { HugeiconsIcon } from '@hugeicons/react'
import { ComputerIcon, Moon02Icon, Sun03Icon } from '@hugeicons/core-free-icons'
import type { ThemeMode } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import type { MailxClient } from '@/lib/mailx-client'
import { useSettings, useUpdateAppearance } from './settings-queries'

const ORDER: ThemeMode[] = ['system', 'light', 'dark']

const ICONS = { system: ComputerIcon, light: Sun03Icon, dark: Moon02Icon }

/** Title-bar button that cycles the color mode: system, light, dark. */
export function ThemeToggle({ client }: { client: MailxClient }) {
  useTranslation()
  const LABELS: Record<ThemeMode, string> = { system: t('settings:system'), light: t('settings:light'), dark: t('settings:dark') }
  const settings = useSettings(client)
  const update = useUpdateAppearance(client)
  const appearance = settings.data?.settings.appearance
  if (!appearance) return null

  const next = ORDER[(ORDER.indexOf(appearance.theme) + 1) % ORDER.length]
  const label = t('settings:themeToggle', { current: LABELS[appearance.theme], next: LABELS[next] })
  return (
    <TitleBarActions side="trailing">
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button variant="ghost" size="icon-sm" aria-label={label} onClick={() => update.mutate({ ...appearance, theme: next })}>
                <HugeiconsIcon icon={ICONS[appearance.theme]} strokeWidth={2} />
              </Button>
            }
          />
          <TooltipContent side="bottom">{label}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </TitleBarActions>
  )
}
