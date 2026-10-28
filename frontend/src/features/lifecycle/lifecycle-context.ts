import { createContext, useContext } from 'react'
import type { LifecycleClient, LifecycleSnapshot } from './lifecycle-client'

export const LifecycleContext = createContext<{ client: LifecycleClient; snapshot: LifecycleSnapshot | null } | null>(null)
export const useLifecycle = () => useContext(LifecycleContext)
