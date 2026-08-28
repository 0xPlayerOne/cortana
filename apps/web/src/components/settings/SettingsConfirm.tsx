import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '../shadcn/alert-dialog'

type ConfirmSettingsAction = (description: string) => boolean | Promise<boolean>

// Existing behavior tests inject a deterministic confirm function. Preserve
// that seam without exposing the browser-native dialog in production.
const initialWindowConfirm = window.confirm

const SettingsConfirmContext = createContext<ConfirmSettingsAction>((description) =>
  window.confirm(description)
)

type PendingConfirmation = {
  description: string
  resolve: (confirmed: boolean) => void
  trigger: HTMLElement | null
  scope: HTMLElement | null
}

export function SettingsConfirmProvider({ children }: { children: ReactNode }) {
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
      settle(false)
      if (window.confirm !== initialWindowConfirm) return window.confirm(description)
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
    [settle]
  )

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
