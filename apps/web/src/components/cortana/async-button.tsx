import type { ComponentProps } from 'react'

import { Button } from '@/components/shadcn/button'
import { Spinner } from '@/components/shadcn/spinner'

export function AsyncButton({
  busy = false,
  busyLabel = 'Working',
  children,
  disabled,
  ...props
}: ComponentProps<typeof Button> & { busy?: boolean; busyLabel?: string }) {
  return (
    <Button aria-busy={busy || undefined} disabled={busy || disabled} {...props}>
      {busy ? <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" /> : null}
      {busy ? busyLabel : children}
    </Button>
  )
}
