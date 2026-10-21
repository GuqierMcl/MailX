import { createContext, useContext } from 'react'

export type ShellControls = {
  navOpen: boolean
  toggleNav: () => void
}

export const ShellContext = createContext<ShellControls | null>(null)

export function useShell() {
  return useContext(ShellContext)
}
