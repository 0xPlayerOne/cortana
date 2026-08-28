import {
  AlertTriangle,
  CheckCircle2,
  CircleStop,
  CircleX,
  Inbox,
  LoaderCircle,
  RefreshCw,
  Settings,
} from 'lucide-react'

import { describeSyncRunProgress } from '@/operations'
import { describeSourceJobProgress, recentCompletedJobs } from '@/sourceJobs'
import type { BrainStatus, DesktopSourceJob, SourceSyncSummary } from '@/types'
import { Alert, AlertAction, AlertDescription, AlertTitle } from '@/components/shadcn/alert'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/shadcn/card'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/shadcn/empty'
import { Progress } from '@/components/shadcn/progress'

export type M7ActivityInboxProps = {
  status: BrainStatus | null
  statusError?: string
  sourceJobs: DesktopSourceJob[]
  sourceJobError?: string
  onRetrySourceJobs?: () => void
  onOpenSettings: () => void
  onRetryStatus?: () => void
  onCancelSourceJob?: (id: string) => void
}

function statusBadge(status: SourceSyncSummary['status'] | DesktopSourceJob['status']) {
  const label =
    status === 'budget_exceeded'
      ? 'Budget exceeded'
      : status === 'cancelling'
        ? 'Cancelling…'
        : status[0].toUpperCase() + status.slice(1)
  const variant =
    status === 'failed' || status === 'cancelled' || status === 'budget_exceeded'
      ? 'destructive'
      : status === 'succeeded'
        ? 'secondary'
        : 'outline'
  return <Badge variant={variant}>{label}</Badge>
}

function statusIcon(status: SourceSyncSummary['status'] | DesktopSourceJob['status']) {
  if (status === 'running' || status === 'cancelling') {
    return <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
  }
  if (status === 'succeeded') return <CheckCircle2 className="size-4" aria-hidden="true" />
  if (status === 'cancelled') return <CircleX className="size-4" aria-hidden="true" />
  return <AlertTriangle className="size-4" aria-hidden="true" />
}

function ActivityEmpty({
  loading,
  error,
  onRetryStatus,
  onOpenSettings,
}: {
  loading: boolean
  error: string
  onRetryStatus?: () => void
  onOpenSettings: () => void
}) {
  return (
    <Empty className="min-h-72 border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          {loading ? (
            <LoaderCircle className="animate-spin" aria-hidden="true" />
          ) : error ? (
            <AlertTriangle aria-hidden="true" />
          ) : (
            <Inbox aria-hidden="true" />
          )}
        </EmptyMedia>
        <EmptyTitle>
          {loading
            ? 'Loading sync health'
            : error
              ? 'Sync health unavailable'
              : 'No sync attention'}
        </EmptyTitle>
        <EmptyDescription>
          {error ||
            (loading
              ? 'Waiting for the runtime status snapshot before reporting source health or sync history.'
              : 'Every configured source is idle and the latest syncs finished cleanly. New activity appears here as it happens.')}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <div className="flex flex-wrap justify-center gap-2">
          {error && onRetryStatus ? (
            <Button variant="outline" onClick={onRetryStatus}>
              <RefreshCw aria-hidden="true" /> Retry status
            </Button>
          ) : null}
          <Button variant="outline" onClick={onOpenSettings}>
            <Settings aria-hidden="true" /> Open settings
          </Button>
        </div>
      </EmptyContent>
    </Empty>
  )
}

function SyncActivityCard({ run }: { run: SourceSyncSummary }) {
  const documents = run.progress_documents ?? run.documents ?? 0
  const progress = run.budget_documents
    ? Math.min(100, Math.round((documents / run.budget_documents) * 100))
    : null
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {statusIcon(run.status)} {run.source}
        </CardTitle>
        <CardDescription>
          {run.project} · started {new Date(run.started_at).toLocaleString()}
        </CardDescription>
        <CardAction>{statusBadge(run.status)}</CardAction>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{describeSyncRunProgress(run)}</p>
        {run.status === 'running' ? (
          <Progress value={progress} aria-label={`${run.source} sync progress`} />
        ) : null}
        <p className="text-xs text-muted-foreground">
          {documents.toLocaleString()} documents ·{' '}
          {(run.progress_bytes ?? run.bytes ?? 0).toLocaleString()} bytes
        </p>
      </CardContent>
    </Card>
  )
}

function SourceJobCard({
  job,
  onCancel,
}: {
  job: DesktopSourceJob
  onCancel?: (id: string) => void
}) {
  const completed = job.completed_at_unix_seconds
    ? new Date(job.completed_at_unix_seconds * 1000)
    : null
  const started = new Date(job.started_at_unix_seconds * 1000)
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {statusIcon(job.status)} {job.source} · {job.operation}
        </CardTitle>
        <CardDescription>
          {job.project} · started {started.toLocaleString()}
        </CardDescription>
        <CardAction>{statusBadge(job.status)}</CardAction>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{describeSourceJobProgress(job)}</p>
        {job.status === 'running' || job.status === 'cancelling' ? (
          <Progress value={null} aria-label={`${job.source} ${job.operation} in progress`} />
        ) : null}
        {completed ? (
          <p className="text-xs text-muted-foreground">
            Completed in {Math.max(0, Math.round((completed.getTime() - started.getTime()) / 1000))}{' '}
            seconds
          </p>
        ) : null}
        {job.log ? (
          <details className="rounded-md border p-2 text-xs">
            <summary className="cursor-pointer font-medium">View job log</summary>
            <pre className="mt-2 overflow-auto whitespace-pre-wrap text-muted-foreground">
              {job.log}
            </pre>
          </details>
        ) : null}
        {onCancel && (job.status === 'running' || job.status === 'cancelling') ? (
          <Button
            variant="outline"
            size="sm"
            disabled={job.status === 'cancelling'}
            onClick={() => onCancel(job.id)}
          >
            <CircleStop aria-hidden="true" /> Cancel
          </Button>
        ) : null}
      </CardContent>
    </Card>
  )
}

export function M7ActivityInbox({
  status,
  statusError = '',
  sourceJobs,
  sourceJobError = '',
  onRetrySourceJobs,
  onOpenSettings,
  onRetryStatus,
  onCancelSourceJob,
}: M7ActivityInboxProps) {
  const attention = (status?.sync_runs ?? []).filter((run) =>
    ['running', 'failed', 'cancelled', 'budget_exceeded'].includes(run.status)
  )
  const activeJobs = sourceJobs.filter(
    (job) => job.status === 'running' || job.status === 'cancelling'
  )
  const completedJobs = recentCompletedJobs(sourceJobs)
  const empty = attention.length === 0 && activeJobs.length === 0 && completedJobs.length === 0

  return (
    <main id="main-content" className="utility-view m7-utility-view" data-m7-activity-inbox>
      <header className="utility-header">
        <div>
          <span className="eyebrow">Attention</span>
          <h1>Inbox</h1>
          <p>Current sync health and source-job activity. Nothing here is fabricated history.</p>
        </div>
      </header>
      <div className="utility-body" data-m7-activity-body>
        {sourceJobError ? (
          <Alert variant="destructive">
            <AlertTriangle aria-hidden="true" />
            <AlertTitle>Source jobs unavailable</AlertTitle>
            <AlertDescription>{sourceJobError}</AlertDescription>
            {onRetrySourceJobs ? (
              <AlertAction>
                <Button variant="outline" size="sm" onClick={onRetrySourceJobs}>
                  Retry
                </Button>
              </AlertAction>
            ) : null}
          </Alert>
        ) : null}
        {statusError && status ? (
          <Alert>
            <AlertTriangle aria-hidden="true" />
            <AlertTitle>Showing the last known sync snapshot</AlertTitle>
            <AlertDescription>{statusError}</AlertDescription>
            {onRetryStatus ? (
              <AlertAction>
                <Button variant="outline" size="sm" onClick={onRetryStatus}>
                  Retry
                </Button>
              </AlertAction>
            ) : null}
          </Alert>
        ) : null}
        {empty ? (
          <ActivityEmpty
            loading={!status && !statusError}
            error={statusError}
            onRetryStatus={onRetryStatus}
            onOpenSettings={onOpenSettings}
          />
        ) : (
          <div className="space-y-6">
            {attention.length ? (
              <section className="space-y-3" aria-labelledby="m7-sync-attention">
                <h2 id="m7-sync-attention" className="font-heading text-base font-medium">
                  Sync attention
                </h2>
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                  {attention.map((run) => (
                    <SyncActivityCard
                      key={`${run.project}:${run.source}:${run.started_at}`}
                      run={run}
                    />
                  ))}
                </div>
              </section>
            ) : null}
            {activeJobs.length ? (
              <section className="space-y-3" aria-labelledby="m7-active-source-jobs">
                <h2 id="m7-active-source-jobs" className="font-heading text-base font-medium">
                  Active source jobs
                </h2>
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                  {activeJobs.map((job) => (
                    <SourceJobCard key={job.id} job={job} onCancel={onCancelSourceJob} />
                  ))}
                </div>
              </section>
            ) : null}
            {completedJobs.length ? (
              <section className="space-y-3" aria-labelledby="m7-recent-source-jobs">
                <h2 id="m7-recent-source-jobs" className="font-heading text-base font-medium">
                  Recent source jobs
                </h2>
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                  {completedJobs.map((job) => (
                    <SourceJobCard key={job.id} job={job} />
                  ))}
                </div>
              </section>
            ) : null}
          </div>
        )}
        <div className="utility-actions">
          <Button variant="outline" onClick={onOpenSettings}>
            <Settings aria-hidden="true" /> Manage ingestion in settings
          </Button>
        </div>
      </div>
    </main>
  )
}
