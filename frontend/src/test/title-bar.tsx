import type { ReactNode } from 'react'
import { TitleBar, TitleBarSlotsProvider } from '@/features/window-chrome/title-bar'

/** Gives a page the title bar its slots portal into, as the app root does. */
export function withTitleBar(ui: ReactNode) {
  return (
    <TitleBarSlotsProvider>
      <TitleBar host={null} />
      {ui}
    </TitleBarSlotsProvider>
  )
}
