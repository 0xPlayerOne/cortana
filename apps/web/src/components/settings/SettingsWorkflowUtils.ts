import { useEffect, useState } from 'react'

import { isDesktopApp } from '../../api'

export function useDesktopForeground(): boolean {
  const [foreground, setForeground] = useState(
    () => typeof document === 'undefined' || document.visibilityState !== 'hidden'
  )

  useEffect(() => {
    const visibility = { current: document.visibilityState !== 'hidden' }
    const focused = { current: true }
    const syncForeground = () => setForeground(visibility.current && focused.current)
    const markVisible = () => {
      visibility.current = document.visibilityState !== 'hidden'
      syncForeground()
    }
    const markFocused = () => {
      focused.current = true
      syncForeground()
    }
    const markBlurred = () => {
      focused.current = false
      syncForeground()
    }

    window.addEventListener('focus', markFocused)
    window.addEventListener('blur', markBlurred)
    document.addEventListener('visibilitychange', markVisible)
    let disposed = false
    let unlistenFocus: (() => void) | undefined
    if (isDesktopApp && '__TAURI_INTERNALS__' in window) {
      void import('@tauri-apps/api/window')
        .then(({ getCurrentWindow }) => {
          const currentWindow = getCurrentWindow()
          void currentWindow
            .isFocused()
            .then((payload) => {
              if (!disposed) {
                focused.current = payload
                syncForeground()
              }
            })
            .catch(() => undefined)
          return currentWindow.onFocusChanged(({ payload }) => {
            if (!disposed) {
              focused.current = payload
              syncForeground()
            }
          })
        })
        .then((unlisten) => {
          if (disposed) unlisten()
          else unlistenFocus = unlisten
        })
        .catch(() => undefined)
    }
    return () => {
      disposed = true
      window.removeEventListener('focus', markFocused)
      window.removeEventListener('blur', markBlurred)
      document.removeEventListener('visibilitychange', markVisible)
      unlistenFocus?.()
    }
  }, [])

  return foreground
}

export function applyConfirmed(decision: boolean | Promise<boolean>, action: () => void): void {
  if (typeof decision === 'boolean') {
    if (decision) action()
    return
  }
  void decision.then((confirmed) => confirmed && action())
}
