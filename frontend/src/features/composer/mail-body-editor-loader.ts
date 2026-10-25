import { useEffect, useState } from 'react'

type EditorComponent = typeof import('./mail-body-editor')['default']

let loaded: EditorComponent | null = null
let pending: Promise<EditorComponent> | null = null

/** One import per WebView, started before draft IPC finishes in composer windows. */
export function preloadMailBodyEditor(): Promise<EditorComponent> {
  pending ??= import('./mail-body-editor').then(
    (module) => {
      loaded = module.default
      return loaded
    },
    (error: unknown) => {
      pending = null
      throw error
    },
  )
  return pending
}

export function useMailBodyEditor() {
  const [state, setState] = useState(() => ({ Component: loaded, failed: false }))
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let active = true
    void preloadMailBodyEditor().then(
      (Component) => {
        if (active) setState((current) => current.Component === Component ? current : { Component, failed: false })
      },
      () => {
        if (active) setState({ Component: null, failed: true })
      },
    )
    return () => { active = false }
  }, [attempt])

  return {
    ...state,
    retry: () => {
      setState({ Component: null, failed: false })
      setAttempt((current) => current + 1)
    },
  }
}
