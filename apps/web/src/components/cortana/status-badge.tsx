import { CircleAlert, CircleCheck, CircleX, CloudOff, LoaderCircle } from 'lucide-react'
import type { ReactNode } from 'react'

import { Badge } from '@/components/shadcn/badge'
import { cn } from '@/lib/utils'

export type StatusTone = 'success' | 'warning' | 'error' | 'offline' | 'busy'

const statusContract = {
  success: {
    icon: CircleCheck,
    className: 'border-success/40 bg-success/10 text-success',
  },
  warning: {
    icon: CircleAlert,
    className: 'border-warning/40 bg-warning/10 text-warning',
  },
  error: {
    icon: CircleX,
    className: 'border-destructive/40 bg-destructive/10 text-destructive',
  },
  offline: {
    icon: CloudOff,
    className: 'border-muted-foreground/40 bg-muted text-muted-foreground',
  },
  busy: {
    icon: LoaderCircle,
    className: 'border-primary/40 bg-primary/10 text-primary',
  },
} satisfies Record<StatusTone, { icon: typeof CircleCheck; className: string }>

export function StatusBadge({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  const { icon: Icon, className } = statusContract[tone]

  return (
    <Badge variant="outline" className={className} role="status">
      <Icon data-icon="inline-start" className={cn(tone === 'busy' && 'animate-spin')} />
      {children}
    </Badge>
  )
}
