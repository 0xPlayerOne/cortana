import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'

import { useM7SurfacePrimitives } from '../m7/M7SurfacePrimitives'

type ConfirmSettingsAction = (description: string) => boolean | Promise<boolean>

const SettingsConfirmContext = createContext<ConfirmSettingsAction>((description) =>
  window.confirm(description)
)

type PendingConfirmation = {
  description: string
  resolve: (confirmed: boolean) => void
  trigger: HTMLElement | null
  scope: HTMLElement | null
}

export function SettingsConfirmProvider({
  renderer,
  children,
}: {
  renderer: 'legacy' | 'shadcn'
  children: ReactNode
}) {
  const primitives = useM7SurfacePrimitives()
  const [pending, setPending] = useState<PendingConfirmation | null>(null)
  const pendingRef = useRef<PendingConfirmation | null>(null)
  const restoreRef = useRef<(PendingConfirmation & { confirmed: boolean }) | null>(null)

  const restoreFocus = useCallback(() => {
    const current = restoreRef.current
    if (!current) return
    restoreRef.current = null
    current.resolve(current.confirmed)
    // Resolve first because a confirmed action may remove its trigger. Wait
    // until React commits that action before choosing the surviving target.
    window.setTimeout(() => {
      if (current.trigger?.isConnected) {
        current.trigger.focus()
        return
      }
      const fallback =
        current.scope?.querySelector<HTMLElement>('.settings-nav-item.active') ??
        current.scope?.querySelector<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])'
        )
      if (fallback?.isConnected) fallback.focus()
    }, 50)
  }, [])

  const settle = useCallback((confirmed: boolean) => {
    const current = pendingRef.current
    if (!current) return
    pendingRef.current = null
    restoreRef.current = { ...current, confirmed }
    setPending(null)
  }, [])

  useEffect(() => () => settle(false), [settle])

  const confirm = useCallback<ConfirmSettingsAction>(
    (description) => {
      if (renderer === 'legacy') {
        return window.confirm(description)
      }
      if (!primitives?.AlertDialog) {
        throw new Error('The shadcn settings renderer requires AlertDialog primitives.')
      }
      settle(false)
      return new Promise<boolean>((resolve) => {
        const next = {
          description,
          resolve,
          trigger: document.activeElement instanceof HTMLElement ? document.activeElement : null,
          scope:
            document.activeElement instanceof HTMLElement
              ? document.activeElement.closest<HTMLElement>('.settings-view')
              : null,
        }
        pendingRef.current = next
        setPending(next)
      })
    },
    [primitives?.AlertDialog, renderer, settle]
  )

  if (renderer === 'legacy') {
    return (
      <SettingsConfirmContext.Provider value={confirm}>{children}</SettingsConfirmContext.Provider>
    )
  }

  if (!primitives?.AlertDialog) {
    throw new Error('The shadcn settings renderer requires AlertDialog primitives.')
  }

  const AlertDialog = primitives.AlertDialog
  const AlertDialogAction = primitives.AlertDialogAction
  const AlertDialogCancel = primitives.AlertDialogCancel
  const AlertDialogContent = primitives.AlertDialogContent
  const AlertDialogDescription = primitives.AlertDialogDescription
  const AlertDialogFooter = primitives.AlertDialogFooter
  const AlertDialogHeader = primitives.AlertDialogHeader
  const AlertDialogTitle = primitives.AlertDialogTitle

  return (
    <SettingsConfirmContext.Provider value={confirm}>
      {children}
      <AlertDialog
        open={Boolean(pending)}
        onOpenChange={(open) => !open && settle(false)}
        onOpenChangeComplete={(open) => !open && restoreFocus()}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Confirm this action</AlertDialogTitle>
            <AlertDialogDescription className="whitespace-pre-line">
              {pending?.description}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => settle(false)}>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => settle(true)}>
              Continue
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsConfirmContext.Provider>
  )
}

// The hook and provider intentionally share one module-scoped confirmation context.
// eslint-disable-next-line react-refresh/only-export-components
export function useSettingsConfirm() {
  return useContext(SettingsConfirmContext)
}
