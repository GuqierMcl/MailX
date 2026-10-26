import { createContext, useContext } from 'react'
import type { UpdateSnapshot } from './types'

export type UpdaterContextValue = {
  snapshot: UpdateSnapshot
  /** A check has finished in this run, so "up to date" is a fact, not a guess. */
  checked: boolean
  /** Checks now and, when a version is found, opens the modal. Never a toast. */
  check(): Promise<void>
  openDialog(): void
}

export const UpdaterContext = createContext<UpdaterContextValue | null>(null)

/** The update state for the main window, its modal and its notices. */
export function useUpdater(): UpdaterContextValue {
  const value = useContext(UpdaterContext)
  if (!value) throw new Error('useUpdater outside UpdaterProvider')
  return value
}
