import type { QueryClient } from '@tanstack/react-query'
import type { AppSettings, SettingsSnapshot } from '@/generated/accounts'
import { settingsKeys } from './query-client'

type Edit = { section: keyof AppSettings; change: object }
type State = { committed?: SettingsSnapshot; edits: Map<symbol, Edit> }
const states = new WeakMap<QueryClient, State>()

function stateFor(client: QueryClient): State {
  let state = states.get(client)
  if (!state) {
    state = { committed: client.getQueryData(settingsKeys.all), edits: new Map() }
    states.set(client, state)
  }
  return state
}

function project(state: State): SettingsSnapshot | undefined {
  if (!state.committed) return undefined
  const settings = { ...state.committed.settings }
  for (const { section, change } of state.edits.values()) {
    Object.assign(settings, { [section]: { ...settings[section], ...change } })
  }
  return { ...state.committed, settings }
}

function publish(client: QueryClient, state: State): SettingsSnapshot | undefined {
  const snapshot = project(state)
  if (snapshot) client.setQueryData(settingsKeys.all, snapshot)
  return snapshot
}

/** Recheck at TanStack's final cache write, after the query promise settles. */
export function latestSettingsSnapshot(client: QueryClient): SettingsSnapshot | undefined {
  return project(stateFor(client))
}

/** Accept authoritative state before reapplying only edits still awaiting a result. */
export function acceptSettingsSnapshot(client: QueryClient, snapshot: SettingsSnapshot): SettingsSnapshot {
  const state = stateFor(client)
  if (!state.committed || snapshot.revision > state.committed.revision) state.committed = snapshot
  return publish(client, state)!
}

export function beginSettingsEdit(client: QueryClient, section: keyof AppSettings, change: object): symbol {
  const token = Symbol(section)
  const state = stateFor(client)
  state.edits.set(token, { section, change })
  publish(client, state)
  return token
}

/** Pending UI edits must never be included in a different save request. */
export function committedSettings(client: QueryClient): AppSettings | undefined {
  return stateFor(client).committed?.settings
}

export function finishSettingsEdit(client: QueryClient, token: symbol | undefined, snapshot?: SettingsSnapshot) {
  const state = stateFor(client)
  if (token) state.edits.delete(token)
  if (snapshot) acceptSettingsSnapshot(client, snapshot)
  else publish(client, state)
}
