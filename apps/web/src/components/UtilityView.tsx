import {
  AlertTriangle,
  BookOpen,
  Check,
  CheckCircle2,
  CircleStop,
  CircleX,
  Copy,
  ExternalLink,
  FileText,
  Inbox,
  LoaderCircle,
  MessageCircle,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  TerminalSquare,
} from 'lucide-react'
import { useState } from 'react'

import { openDesktopUrl } from '../api'
import type {
  AnswerResponse,
  BrainStatus,
  ContextBundle,
  DesktopSourceJob,
  Evidence,
} from '../types'
import { describeSourceJobProgress, recentCompletedJobs } from '../sourceJobs'
import { describeSyncRunProgress } from '../operations'
import { shortcutLabel } from '../shortcuts'
import { useClipboardCopy } from '../useClipboardCopy'

export type UtilityKind = 'inbox' | 'conversations' | 'agent-tools' | 'index' | 'help'

const TITLES: Record<UtilityKind, { eyebrow: string; title: string; description: string }> = {
  inbox: {
    eyebrow: 'Attention',
    title: 'Inbox',
    description: 'Current sync health and source-job activity. Nothing here is fabricated history.',
  },
  conversations: {
    eyebrow: 'Session',
    title: 'Conversations',
    description: 'The live query, answer, and evidence state of this workspace session.',
  },
  'agent-tools': {
    eyebrow: 'Retrieval',
    title: 'Agent tools',
    description: 'The token-bounded context bundle generated for the current conversation.',
  },
  index: {
    eyebrow: 'Brain',
    title: 'Index',
    description: 'Live document, chunk, source, and cache metrics reported by the brain.',
  },
  help: {
    eyebrow: 'Support',
    title: 'Help',
    description: 'Keyboard shortcuts and links to the project documentation.',
  },
}

export function UtilityView({
  kind,
  status,
  statusError = '',
  onRetryStatus,
  sourceJobs,
  query,
  answer,
  evidence,
  loading,
  error,
  contextBundle,
  contextLoading,
  contextError,
  contextTokens,
  desktopAvailable,
  sourceJobError,
  onRetrySourceJobs,
  onSearchFocus,
  onRetrieveContext,
  onOpenSettings,
  onOpenProject,
  onCancelSourceJob,
}: {
  kind: UtilityKind
  status: BrainStatus | null
  statusError?: string
  onRetryStatus?: () => void
  sourceJobs: DesktopSourceJob[]
  query: string
  answer: AnswerResponse | null
  evidence: Evidence[]
  loading: boolean
  error: string
  contextBundle: ContextBundle | null
  contextLoading: boolean
  contextError: string
  contextTokens: number
  desktopAvailable: boolean
  sourceJobError?: string
  onRetrySourceJobs?: () => void
  onSearchFocus: () => void
  onRetrieveContext: () => void
  onOpenSettings: () => void
  onOpenProject: () => void | Promise<void>
  onCancelSourceJob?: (id: string) => void
}) {
  const { eyebrow, title, description } = TITLES[kind]
  return (
    <main className="utility-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">{eyebrow}</span>
          <h1>{title}</h1>
          <p>{description}</p>
        </div>
      </header>
      <div className="utility-body">
        {kind === 'inbox' && (
          <InboxView
            status={status}
            statusError={statusError}
            sourceJobs={sourceJobs}
            sourceJobError={sourceJobError}
            onRetrySourceJobs={onRetrySourceJobs}
            onOpenSettings={onOpenSettings}
            onRetryStatus={onRetryStatus}
            onCancelSourceJob={onCancelSourceJob}
          />
        )}
        {kind === 'conversations' && (
          <ConversationsView
            query={query}
            answer={answer}
            evidence={evidence}
            loading={loading}
            error={error}
            onSearchFocus={onSearchFocus}
          />
        )}
        {kind === 'agent-tools' && (
          <AgentToolsView
            query={query}
            evidence={evidence}
            contextBundle={contextBundle}
            contextLoading={contextLoading}
            contextError={contextError}
            contextTokens={contextTokens}
            onRetrieveContext={onRetrieveContext}
          />
        )}
        {kind === 'index' && (
          <IndexView
            status={status}
            statusError={statusError}
            onOpenSettings={onOpenSettings}
            onRetryStatus={onRetryStatus}
          />
        )}
        {kind === 'help' && (
          <HelpView desktopAvailable={desktopAvailable} onOpenProject={onOpenProject} />
        )}
      </div>
    </main>
  )
}

function InboxView({
  status,
  statusError,
  sourceJobs,
  sourceJobError,
  onRetrySourceJobs,
  onOpenSettings,
  onRetryStatus,
  onCancelSourceJob,
}: {
  status: BrainStatus | null
  statusError: string
  sourceJobs: DesktopSourceJob[]
  sourceJobError?: string
  onRetrySourceJobs?: () => void
  onOpenSettings: () => void
  onRetryStatus?: () => void
  onCancelSourceJob?: (id: string) => void
}) {
  const attention = (status?.sync_runs ?? []).filter((run) =>
    ['running', 'failed', 'cancelled', 'budget_exceeded'].includes(run.status)
  )
  const activeJobs = sourceJobs.filter(
    (job) => job.status === 'running' || job.status === 'cancelling'
  )
  const completedJobs = recentCompletedJobs(sourceJobs)
  if (
    !sourceJobError &&
    attention.length === 0 &&
    activeJobs.length === 0 &&
    completedJobs.length === 0
  ) {
    if (!status) {
      return (
        <UtilityEmpty
          icon={
            statusError ? <AlertTriangle size={26} /> : <LoaderCircle className="spin" size={26} />
          }
          title={statusError ? 'Sync health unavailable' : 'Loading sync health'}
          detail={
            statusError ||
            'Waiting for the runtime status snapshot before reporting source health or sync history.'
          }
          actions={[
            ...(onRetryStatus
              ? [{ label: 'Retry status', icon: <RefreshCw size={15} />, onClick: onRetryStatus }]
              : []),
            { label: 'Open settings', icon: <Settings size={15} />, onClick: onOpenSettings },
          ]}
        />
      )
    }
    return (
      <UtilityEmpty
        icon={<Inbox size={26} />}
        title="No sync attention"
        detail={
          statusError
            ? `${statusError} No attention is recorded in the last known snapshot.`
            : 'Every configured source is idle and the last sync of each source finished cleanly. New sync activity will appear here as it happens.'
        }
        actions={[
          { label: 'Open settings', icon: <Settings size={15} />, onClick: onOpenSettings },
        ]}
      />
    )
  }
  return (
    <>
      {sourceJobError && (
        <p className="utility-error" role="alert">
          {sourceJobError}
          {onRetrySourceJobs && (
            <>
              {' '}
              <button type="button" className="link-button" onClick={onRetrySourceJobs}>
                Retry source jobs
              </button>
            </>
          )}
        </p>
      )}
      {statusError && status && (
        <p className="utility-error" role="status">
          {statusError} Showing the last known sync snapshot.{' '}
          {onRetryStatus && (
            <button type="button" className="link-button" onClick={onRetryStatus}>
              Retry status
            </button>
          )}
        </p>
      )}
      {attention.length > 0 && (
        <section className="utility-section">
          <h2>Sync attention</h2>
          <div className="utility-list">
            {attention.map((run) => (
              <div className="utility-item" key={`${run.project}:${run.source}:${run.started_at}`}>
                <SyncIcon status={run.status} />
                <div className="utility-item-main">
                  <strong>{run.source}</strong>
                  <span>
                    {run.project} · started {new Date(run.started_at).toLocaleString()} ·{' '}
                    {describeSyncRunProgress(run)} · {run.documents ?? '—'} documents ·{' '}
                    {run.bytes ?? '—'} bytes
                  </span>
                </div>
                <StatusPill status={run.status} />
              </div>
            ))}
          </div>
        </section>
      )}
      {activeJobs.length > 0 && (
        <section className="utility-section">
          <h2>Active source jobs</h2>
          <div className="utility-list">
            {activeJobs.map((job) => (
              <div className="utility-item" key={job.id}>
                <LoaderCircle className="spin" size={16} />
                <div className="utility-item-main">
                  <strong>{job.source}</strong>
                  <span>
                    {job.project} · {job.operation} · {describeSourceJobProgress(job)} · started{' '}
                    {new Date(job.started_at_unix_seconds * 1000).toLocaleString()}
                  </span>
                </div>
                <StatusPill status={job.status} />
                {onCancelSourceJob && (
                  <button
                    type="button"
                    className="utility-cancel"
                    disabled={job.status === 'cancelling'}
                    aria-label={`Cancel ${job.project} ${job.source} ${job.operation}`}
                    onClick={() => onCancelSourceJob(job.id)}
                  >
                    <CircleStop size={14} /> Cancel
                  </button>
                )}
              </div>
            ))}
          </div>
        </section>
      )}
      {completedJobs.length > 0 && (
        <section className="utility-section">
          <h2>Recent source jobs</h2>
          <div className="utility-list">
            {completedJobs.map((job) => {
              const terminalStatus = job.status === 'cancelling' ? 'running' : job.status
              const started = new Date(job.started_at_unix_seconds * 1000)
              const completed = job.completed_at_unix_seconds
                ? new Date(job.completed_at_unix_seconds * 1000)
                : null
              const duration = completed
                ? `${Math.max(0, Math.round((completed.getTime() - started.getTime()) / 1000))}s`
                : 'duration unavailable'
              return (
                <div className="utility-item" key={job.id}>
                  <SyncIcon status={terminalStatus} />
                  <div className="utility-item-main">
                    <strong>
                      {job.source} · {job.operation}
                    </strong>
                    <span>
                      {job.project} · {job.summary} · started {started.toLocaleString()} ·{' '}
                      {duration}
                    </span>
                    {job.log && (
                      <details className="utility-job-log">
                        <summary>View job log</summary>
                        <pre>{job.log}</pre>
                      </details>
                    )}
                  </div>
                  <StatusPill status={terminalStatus} />
                </div>
              )
            })}
          </div>
        </section>
      )}
      <div className="utility-actions">
        <button type="button" className="secondary-button" onClick={onOpenSettings}>
          <Settings size={15} /> Manage ingestion in settings
        </button>
      </div>
    </>
  )
}

function ConversationsView({
  query,
  answer,
  evidence,
  loading,
  error,
  onSearchFocus,
}: {
  query: string
  answer: AnswerResponse | null
  evidence: Evidence[]
  loading: boolean
  error: string
  onSearchFocus: () => void
}) {
  if (loading) {
    return (
      <UtilityEmpty
        icon={<LoaderCircle className="spin" size={26} />}
        title="Searching the brain"
        detail={`Fusing semantic and exact-term evidence for “${query}”.`}
      />
    )
  }
  if (error && !answer) {
    return (
      <UtilityEmpty
        icon={<AlertTriangle size={26} />}
        title="The brain is unreachable"
        detail={`${error} Start the Rust API or add ?demo=1 to preview the workspace.`}
        actions={[
          { label: 'Search the brain', icon: <Search size={15} />, onClick: onSearchFocus },
        ]}
      />
    )
  }
  if (!answer) {
    return (
      <UtilityEmpty
        icon={<MessageCircle size={26} />}
        title="No conversation yet"
        detail="Ask a question in the search bar above. The current query, answer, and cited evidence will be tracked here."
        actions={[
          { label: 'Search the brain', icon: <Search size={15} />, onClick: onSearchFocus },
        ]}
      />
    )
  }
  return (
    <>
      <section className="utility-section">
        <h2>Current conversation</h2>
        <div className="utility-card">
          <span className="utility-card-eyebrow">
            <Sparkles size={14} /> Query
          </span>
          <h3>{query}</h3>
          <div className="utility-meta">
            <span>{answer.mode}</span>
            <span>
              {answer.retrieval_degraded
                ? 'lexical fallback'
                : answer.retrieval_mode || 'hybrid retrieval'}
            </span>
            <span>{answer.cached ? 'cache hit' : `${answer.latency_ms} ms`}</span>
            <span>
              {answer.plan.queries.length}{' '}
              {answer.plan.queries.length === 1 ? 'retrieval' : 'retrievals'}
            </span>
            <span>{evidence.length} cited passages</span>
          </div>
          <p className="utility-answer">{answer.answer}</p>
          {answer.warnings.map((warning, index) => (
            <p className="answer-warning" key={`${warning}:${index}`}>
              {warning}
            </p>
          ))}
        </div>
      </section>
      {evidence.length > 0 && (
        <section className="utility-section">
          <h2>Cited evidence</h2>
          <div className="utility-list">
            {evidence.slice(0, 4).map((item, index) => (
              <div className="utility-item" key={item.chunk_id}>
                <span className="utility-index">{index + 1}</span>
                <div className="utility-item-main">
                  <strong>{item.title}</strong>
                  <span>
                    {item.source} · updated {new Date(item.updated_at).toLocaleDateString()}
                  </span>
                </div>
              </div>
            ))}
          </div>
        </section>
      )}
      <div className="utility-actions">
        <button type="button" className="secondary-button" onClick={onSearchFocus}>
          <Search size={15} /> Search the brain
        </button>
      </div>
    </>
  )
}

function AgentToolsView({
  query,
  evidence,
  contextBundle,
  contextLoading,
  contextError,
  contextTokens,
  onRetrieveContext,
}: {
  query: string
  evidence: Evidence[]
  contextBundle: ContextBundle | null
  contextLoading: boolean
  contextError: string
  contextTokens: number
  onRetrieveContext: () => void
}) {
  const { copied, copyError, copy } = useClipboardCopy(contextBundle?.context ?? null)

  return (
    <>
      <section className="utility-section">
        <h2>Generated context</h2>
        {contextLoading ? (
          <UtilityEmpty
            icon={<LoaderCircle className="spin" size={26} />}
            title="Retrieving context"
            detail={`Building a token-bounded bundle for “${query}”.`}
          />
        ) : contextBundle ? (
          <>
            <div className="utility-metrics">
              <Metric label="Retrieval" value={contextBundle.retrieval_mode || 'hybrid'} />
              <Metric label="Retrieved" value={contextBundle.metrics.retrieved.toLocaleString()} />
              <Metric label="Included" value={contextBundle.metrics.included.toLocaleString()} />
              <Metric label="Omitted" value={contextBundle.metrics.omitted.toLocaleString()} />
              <Metric
                label="Native memory"
                value={(contextBundle.metrics.memories_included ?? contextBundle.memories?.length ?? 0).toLocaleString()}
              />
              <Metric
                label="Estimated tokens"
                value={contextBundle.metrics.estimated_tokens.toLocaleString()}
              />
              <Metric
                label="Max tokens"
                value={contextBundle.metrics.max_tokens.toLocaleString()}
              />
            </div>
            {contextBundle.retrieval_warning && (
              <p className="answer-warning" role="status">
                {contextBundle.retrieval_warning}
              </p>
            )}
            {contextBundle.evidence.length > 0 && (
              <div className="utility-list utility-list-spaced">
                {contextBundle.evidence.map((item) => (
                  <div className="utility-item" key={item.chunk_id}>
                    <FileText size={16} />
                    <div className="utility-item-main">
                      <strong>{item.title}</strong>
                      <span>
                        {item.source} · score {item.score.toFixed(2)}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <div className="utility-actions">
              <button
                type="button"
                className="secondary-button"
                aria-label="Copy MCP-equivalent context"
                onClick={() => void copy()}
              >
                {copied ? <Check size={15} /> : <Copy size={15} />}
                {copied ? 'Context copied' : 'Copy MCP-equivalent context'}
              </button>
              {copyError && (
                <p className="utility-error" role="alert">
                  {copyError}
                </p>
              )}
            </div>
          </>
        ) : (
          <UtilityEmpty
            icon={<TerminalSquare size={26} />}
            title="No context generated yet"
            detail="Retrieve the token-bounded context bundle for the current conversation. It is the same citation-ready surface the agent integrations receive."
            actions={[
              {
                label: 'Retrieve context',
                icon: <Sparkles size={15} />,
                onClick: onRetrieveContext,
              },
            ]}
          />
        )}
        {contextError && (
          <p className="utility-error" role="alert">
            {contextError}
          </p>
        )}
      </section>
      <section className="utility-section">
        <h2>Agent context window</h2>
        <div className="utility-card">
          <p className="utility-answer">
            ~{contextTokens.toLocaleString()} tokens assembled from the active query and{' '}
            {evidence.length} cited {evidence.length === 1 ? 'passage' : 'passages'}.
          </p>
          <p className="utility-note">
            The window is rebuilt locally from the current session state and never leaves this
            machine.
          </p>
        </div>
      </section>
    </>
  )
}

function IndexView({
  status,
  statusError,
  onOpenSettings,
  onRetryStatus,
}: {
  status: BrainStatus | null
  statusError: string
  onOpenSettings: () => void
  onRetryStatus?: () => void
}) {
  if (!status) {
    return (
      <UtilityEmpty
        icon={
          statusError ? <AlertTriangle size={26} /> : <LoaderCircle className="spin" size={26} />
        }
        title={statusError ? 'Index unavailable' : 'Loading index'}
        detail={
          statusError ||
          'Waiting for the runtime status snapshot before reporting live index metrics.'
        }
        actions={[
          ...(onRetryStatus
            ? [{ label: 'Retry status', icon: <RefreshCw size={15} />, onClick: onRetryStatus }]
            : []),
          { label: 'Open settings', icon: <Settings size={15} />, onClick: onOpenSettings },
        ]}
      />
    )
  }
  return (
    <>
      {status.stats_stale && (
        <p className="utility-warning" role="status">
          {status.stats_warning ?? 'Live database statistics are temporarily stale.'}
          {typeof status.stats_age_seconds === 'number'
            ? ` Snapshot age: ${status.stats_age_seconds.toLocaleString()} seconds.`
            : ''}
        </p>
      )}
      <section className="utility-section">
        <h2>Live metrics</h2>
        <div className="utility-metrics">
          <Metric label="Documents" value={status.documents.toLocaleString()} />
          <Metric label="Chunks" value={status.chunks.toLocaleString()} />
          <Metric label="Sources" value={status.sources.length.toLocaleString()} />
          <Metric
            label="Embedding cache"
            value={`${status.embedding_cache_entries.toLocaleString()} entries`}
          />
          <Metric
            label="Embedding cache hits"
            value={status.embedding_cache_hits.toLocaleString()}
          />
          <Metric
            label="Query cache"
            value={`${status.query_cache_entries.toLocaleString()} entries`}
          />
          <Metric label="Query cache hits" value={status.query_cache_hits.toLocaleString()} />
          <Metric
            label="Retrieval fallbacks"
            value={(status.retrieval_fallbacks_total ?? 0).toLocaleString()}
          />
          <Metric label="Answers total" value={status.answers_total.toLocaleString()} />
        </div>
      </section>
      <section className="utility-section">
        <h2>Configuration</h2>
        <div className="utility-card">
          <div className="utility-line">
            <span>Embedding</span>
            <strong>{status.embedding_fingerprint ?? '—'}</strong>
          </div>
          <div className="utility-line">
            <span>Query mode</span>
            <strong>{status.query.mode}</strong>
          </div>
          <div className="utility-line">
            <span>Ingestion</span>
            <strong>
              {status.ingestion.mode}
              {status.ingestion.scheduled ? ' · scheduled' : ' · manual'}
            </strong>
          </div>
          <div className="utility-line">
            <span>Workspaces</span>
            <strong>{status.workspaces.length}</strong>
          </div>
        </div>
      </section>
      <div className="utility-actions">
        <button type="button" className="secondary-button" onClick={onOpenSettings}>
          <Settings size={15} /> Open settings
        </button>
      </div>
    </>
  )
}

function HelpView({
  desktopAvailable,
  onOpenProject,
}: {
  desktopAvailable: boolean
  onOpenProject: () => void | Promise<void>
}) {
  const [projectError, setProjectError] = useState('')
  const shortcuts = [
    { keys: shortcutLabel('MOD K'), action: 'Focus the search bar' },
    { keys: shortcutLabel('MOD P'), action: 'Toggle the command palette' },
    { keys: shortcutLabel('MOD ⇧ F'), action: 'Open the document filter' },
    { keys: 'Esc', action: 'Close panels and the palette' },
  ]
  const links = [
    {
      label: 'GitHub project',
      href: 'https://github.com/0xPlayerOne/cortana',
      detail: 'Source, releases, and issues.',
    },
    {
      label: 'Documentation',
      href: 'https://github.com/0xPlayerOne/cortana/tree/main/docs',
      detail: 'Architecture, ingestion, query, and operations guides.',
    },
  ]
  return (
    <>
      <section className="utility-section">
        <h2>Keyboard shortcuts</h2>
        <div className="utility-list">
          {shortcuts.map(({ keys, action }) => (
            <div className="utility-shortcut" key={keys}>
              <kbd>{keys}</kbd>
              <span>{action}</span>
            </div>
          ))}
        </div>
      </section>
      <section className="utility-section">
        <h2>Project and docs</h2>
        <div className="utility-list">
          {links.map(({ label, href, detail }) => (
            <a
              className="utility-link"
              href={href}
              target="_blank"
              rel="noreferrer"
              key={href}
              onClick={(event) => {
                if (!desktopAvailable) return
                event.preventDefault()
                setProjectError('')
                void openDesktopUrl(href).catch((caught: unknown) => {
                  setProjectError(
                    caught instanceof Error
                      ? caught.message
                      : `Unable to open ${label.toLowerCase()} in the system browser`
                  )
                })
              }}
            >
              <BookOpen size={16} />
              <span>
                <strong>{label}</strong>
                <small>{detail}</small>
              </span>
              <ExternalLink size={14} />
            </a>
          ))}
        </div>
        {desktopAvailable && (
          <div className="utility-actions">
            <button
              type="button"
              className="secondary-button"
              onClick={() => {
                setProjectError('')
                void Promise.resolve(onOpenProject()).catch((caught: unknown) => {
                  setProjectError(
                    caught instanceof Error
                      ? caught.message
                      : 'Unable to open the Cortana project page'
                  )
                })
              }}
            >
              <ExternalLink size={15} /> Open project page
            </button>
          </div>
        )}
        {projectError && (
          <p className="utility-error" role="alert">
            {projectError}
          </p>
        )}
        <p className="utility-note">
          Cortana is local-first: your index, context bundles, and settings stay on this machine.
        </p>
      </section>
    </>
  )
}

function SyncIcon({ status }: { status: string }) {
  if (status === 'running' || status === 'cancelling') {
    return <LoaderCircle className="spin" size={16} />
  }
  if (status === 'succeeded') {
    return <CheckCircle2 size={16} />
  }
  if (status === 'cancelled') {
    return <CircleX size={16} />
  }
  return <AlertTriangle size={16} />
}

function StatusPill({
  status,
}: {
  status: 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled' | 'budget_exceeded'
}) {
  const label =
    status === 'budget_exceeded'
      ? 'Budget exceeded'
      : status === 'cancelling'
        ? 'Cancelling…'
        : status === 'succeeded'
          ? 'Succeeded'
          : status[0].toUpperCase() + status.slice(1)
  const tone =
    status === 'running' || status === 'cancelling'
      ? 'running'
      : status === 'succeeded'
        ? 'healthy'
        : 'warning'
  return <span className={`status-pill ${tone}`}>{label}</span>
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="utility-metric">
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  )
}

function UtilityEmpty({
  icon,
  title,
  detail,
  actions = [],
}: {
  icon: React.ReactNode
  title: string
  detail: string
  actions?: Array<{ label: string; icon: React.ReactNode; onClick: () => void }>
}) {
  return (
    <div className="utility-empty">
      {icon}
      <strong>{title}</strong>
      <p>{detail}</p>
      {actions.length > 0 && (
        <div className="utility-actions utility-actions-center">
          {actions.map(({ label, icon: actionIcon, onClick }) => (
            <button type="button" className="secondary-button" key={label} onClick={onClick}>
              {actionIcon} {label}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
