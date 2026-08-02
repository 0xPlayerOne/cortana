import { useCallback, useEffect, useRef, useState } from 'react'

import { writeClipboardText } from './clipboard'

export function useClipboardCopy(value: string | null) {
  const [copied, setCopied] = useState(false)
  const [copyError, setCopyError] = useState('')
  const resetTimer = useRef<number | null>(null)

  useEffect(() => {
    return () => {
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current)
    }
  }, [])

  const copy = useCallback(async () => {
    if (value === null) return
    setCopyError('')
    try {
      await writeClipboardText(value)
      setCopied(true)
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current)
      resetTimer.current = window.setTimeout(() => {
        resetTimer.current = null
        setCopied(false)
      }, 1800)
    } catch (caught) {
      setCopied(false)
      setCopyError(caught instanceof Error ? caught.message : 'Unable to copy context')
    }
  }, [value])

  return { copied, copyError, copy }
}
