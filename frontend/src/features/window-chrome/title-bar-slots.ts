import { createContext } from 'react'

export type TitleBarSide = 'leading' | 'center' | 'trailing'
export type TitleBarSlotElements = Record<TitleBarSide, HTMLElement | null>

/** DOM nodes the title bar exposes for page actions, and the setter it registers them with. */
export const TitleBarSlotsContext = createContext<{
  slots: TitleBarSlotElements
  register: (side: TitleBarSide, element: HTMLElement | null) => void
} | null>(null)
