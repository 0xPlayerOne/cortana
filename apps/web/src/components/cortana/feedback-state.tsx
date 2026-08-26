import { CircleAlert, CircleCheck, Inbox, RotateCcw } from 'lucide-react'

import { Alert, AlertDescription, AlertTitle } from '@/components/shadcn/alert'
import { Button } from '@/components/shadcn/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/shadcn/empty'
import { Skeleton } from '@/components/shadcn/skeleton'

type FeedbackStateProps = {
  kind: 'loading' | 'empty' | 'error' | 'success' | 'warning'
  title: string
  description: string
  onRetry?: () => void
}

export function FeedbackState({ kind, title, description, onRetry }: FeedbackStateProps) {
  if (kind === 'loading') {
    return (
      <div className="flex flex-col gap-3" aria-busy="true" aria-label={title}>
        <Skeleton className="h-5 w-2/5" />
        <Skeleton className="h-20 w-full" />
      </div>
    )
  }

  if (kind === 'empty') {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Inbox aria-hidden="true" />
          </EmptyMedia>
          <EmptyTitle>{title}</EmptyTitle>
          <EmptyDescription>{description}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const Icon = kind === 'success' ? CircleCheck : CircleAlert
  return (
    <Alert
      className={
        kind === 'success'
          ? 'border-success/40 text-success'
          : kind === 'warning'
            ? 'border-warning/40 text-warning'
            : 'border-destructive/40 text-destructive'
      }
    >
      <Icon aria-hidden="true" />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{description}</AlertDescription>
      {kind === 'error' && onRetry ? (
        <EmptyContent className="mt-3 items-start">
          <Button variant="outline" size="sm" onClick={onRetry}>
            <RotateCcw data-icon="inline-start" />
            Retry
          </Button>
        </EmptyContent>
      ) : null}
    </Alert>
  )
}
