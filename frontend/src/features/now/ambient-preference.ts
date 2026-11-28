import { useSyncExternalStore } from 'react'
import type { DaySegment } from './greeting'

export type NowAmbientMode = 'full' | 'static' | 'off'
export type NowConcreteScene = 'peaks' | 'sea' | 'window' | 'orbit'
export type NowAmbientScene = 'auto' | NowConcreteScene

export const NOW_AMBIENT_KEY = 'mailx.now.ambient'
export const NOW_AMBIENT_SCENE_KEY = 'mailx.now.ambientScene'
const MODE_EVENT = 'mailx:now-ambient-change'
const SCENE_EVENT = 'mailx:now-ambient-scene-change'

export const AMBIENT_SCENES: readonly NowAmbientScene[] = ['auto', 'peaks', 'window', 'sea', 'orbit']

/** Maps each part of the day to its natural matching scene in automatic mode. */
export function resolveAmbientScene(scene: NowAmbientScene, segment: DaySegment): NowConcreteScene {
  if (scene !== 'auto') return scene
  switch (segment) {
    case 'earlyMorning': return 'peaks'
    case 'morning':
    case 'noon':
    case 'afternoon': return 'window'
    case 'evening': return 'sea'
    case 'night': return 'orbit'
  }
}

function parseMode(raw: string | null): NowAmbientMode {
  return raw === 'static' || raw === 'off' ? raw : 'full'
}

function parseScene(raw: string | null): NowAmbientScene {
  return raw === 'peaks' || raw === 'sea' || raw === 'window' || raw === 'orbit' ? raw : 'auto'
}

export function readNowAmbient(): NowAmbientMode {
  try {
    return parseMode(globalThis.localStorage?.getItem(NOW_AMBIENT_KEY) ?? null)
  } catch {
    return 'full'
  }
}

export function writeNowAmbient(mode: NowAmbientMode) {
  try {
    globalThis.localStorage?.setItem(NOW_AMBIENT_KEY, mode)
  } catch {
    // Storage unavailable; keep working in memory for this session.
  }
  globalThis.dispatchEvent?.(new Event(MODE_EVENT))
}

export function readNowAmbientScene(): NowAmbientScene {
  try {
    return parseScene(globalThis.localStorage?.getItem(NOW_AMBIENT_SCENE_KEY) ?? null)
  } catch {
    return 'auto'
  }
}

export function writeNowAmbientScene(scene: NowAmbientScene) {
  try {
    globalThis.localStorage?.setItem(NOW_AMBIENT_SCENE_KEY, scene)
  } catch {
    // Storage unavailable; keep working in memory for this session.
  }
  globalThis.dispatchEvent?.(new Event(SCENE_EVENT))
}

function subscribe(key: string, eventName: string) {
  return (callback: () => void) => {
    if (typeof window === 'undefined') return () => undefined
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === key) callback()
    }
    window.addEventListener(eventName, callback)
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener(eventName, callback)
      window.removeEventListener('storage', onStorage)
    }
  }
}

const subscribeMode = subscribe(NOW_AMBIENT_KEY, MODE_EVENT)
const subscribeScene = subscribe(NOW_AMBIENT_SCENE_KEY, SCENE_EVENT)

/** Reactive device-local preference for the Now page's backdrop mode. */
export function useNowAmbientMode(): [NowAmbientMode, (mode: NowAmbientMode) => void] {
  const mode = useSyncExternalStore<NowAmbientMode>(subscribeMode, readNowAmbient, () => 'full')
  return [mode, writeNowAmbient]
}

/** Reactive device-local preference for which preset SVG scene the Now page displays. */
export function useNowAmbientScene(): [NowAmbientScene, (scene: NowAmbientScene) => void] {
  const scene = useSyncExternalStore<NowAmbientScene>(subscribeScene, readNowAmbientScene, () => 'auto')
  return [scene, writeNowAmbientScene]
}
