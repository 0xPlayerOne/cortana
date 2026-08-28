import {
  ChevronDown,
  ChevronRight,
  CircleStop,
  Database,
  ExternalLink,
  KeyRound,
  Search,
  Settings,
  X,
} from 'lucide-react'
import { type ComponentProps, useMemo, useState } from 'react'

import { activeJobs, describeSourceJobProgress } from '../sourceJobs'
import { operationalSources, sourceHealth, type OperationalSource } from '../operations'
import { SourceIcon } from './sourceIcons'
import { sourceDisplayName } from './sourceIconData'
import { TooltipButton as Button } from './cortana/TooltipButton'
import { Input } from './shadcn/input'
import { Progress } from './shadcn/progress'
import { Skeleton } from './shadcn/skeleton'
import { Spinner } from './shadcn/spinner'
import { Switch } from './shadcn/switch'
import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from '../types'
import { VirtualDocumentList } from './VirtualDocumentList'

type ActionButtonProps = Omit<ComponentProps<typeof Button>, 'variant' | 'size'> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'ghost' | 'icon' | 'compact'
}

function ActionButton({ variant = 'secondary', ...props }: ActionButtonProps) {
  return (
    <Button
      {...props}
      variant={
        variant === 'primary'
          ? 'default'
          : variant === 'danger'
            ? 'destructive'
            : variant === 'ghost' || variant === 'icon'
              ? 'ghost'
              : 'secondary'
      }
      size={variant === 'icon' ? 'icon' : variant === 'compact' ? 'sm' : 'default'}
    />
  )
}

export function SourcePanel({
  open,
  status,
  statusError,
  onRetryStatus,
  sourceJobError = '',
  onRetrySourceJobs,
  workspace,
  workspaces,
  documentQuery,
  selected,
  documents,
  selectedDocument,
  documentsLoading,
  documentsError,
  hasMoreDocuments,
  onSelect,
  onDocumentQueryChange,
  onSelectDocument,
  onLoadMoreDocuments,
  onRetryDocuments,
  onOpenSourcesSettings,
  onOpenSourceSetup,
  onAuthorizeSource,
  onToggleSource,
  sourceToggleBusy = null,
  sourceToggleDisabled = false,
  sourceToggleError = '',
  sourceToggleNotice = '',
  onClose,
  onCancelSourceJob,
  jobs = [],
}: {
  open: boolean
  status: BrainStatus | null
  statusError: string
  onRetryStatus?: () => void
  sourceJobError?: string
  onRetrySourceJobs?: () => void
  workspace: string
  workspaces: WorkspaceSettings[]
  documentQuery: string
  selected: string
  documents: BrainDocumentSummary[]
  selectedDocument: string
  documentsLoading: boolean
  documentsError: string
  hasMoreDocuments: boolean
  onSelect: (source: string, project: string) => void
  onDocumentQueryChange: (query: string) => void
  onSelectDocument: (id: string) => void
  onLoadMoreDocuments: () => void
  onRetryDocuments?: () => void
  onOpenSourcesSettings: () => void
  onOpenSourceSetup?: (source: string, project: string) => void
  onAuthorizeSource?: (source: string, project: string) => void
  onToggleSource?: (source: string, project: string, enabled: boolean) => void
  sourceToggleBusy?: string | null
  sourceToggleDisabled?: boolean
  sourceToggleError?: string
  sourceToggleNotice?: string
  onClose: () => void
  onCancelSourceJob?: (id: string) => void
  jobs?: DesktopSourceJob[]
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const selectedWorkspaceId = workspace || workspaces[0]?.id || ''
  const sources = useMemo(
    () => operationalSources(status).filter((item) => item.project === selectedWorkspaceId),
    [selectedWorkspaceId, status]
  )
  const projects = useMemo(
    () =>
      Object.entries(
        sources.reduce<Record<string, OperationalSource[]>>((groups, source) => {
          ;(groups[source.project] ??= []).push(source)
          return groups
        }, {})
      ),
    [sources]
  )
  const active = activeJobs(jobs)
  const selectedWorkspace = workspaces.find((item) => item.id === workspace) ?? workspaces[0]
  const selectedSource = sources.find((item) => item.source === selected)
  const statusLoading = status === null && statusError === ''
  const sourceModeClass = status
    ? status.ingestion.scheduled
      ? 'scheduled'
      : 'manual'
    : statusError
      ? 'unavailable'
      : 'manual'
  const sourceModeLabel = status
    ? status.ingestion.scheduled
      ? 'scheduled'
      : 'paused · manual only'
    : statusError
      ? 'status unavailable'
      : 'loading status…'

  return (
    <aside
      className={`source-panel ${open ? 'mobile-open' : ''} m7-source-panel`}
      data-m7-source-panel=""
    >
      <div className="panel-heading">
        <strong>Sources</strong>
        <ActionButton
          variant="icon"
          className="mobile-close "
          aria-label="Close sources"
          tooltip="Close sources"
          onClick={onClose}
        >
          <X size={17} />
        </ActionButton>
        <ActionButton
          variant="icon"
          className=""
          aria-label="Add source"
          tooltip="Add source"
          onClick={onOpenSourcesSettings}
        >
          +
        </ActionButton>
        <ActionButton
          variant="icon"
          className=""
          aria-label="Source settings"
          tooltip="Source settings"
          onClick={onOpenSourcesSettings}
        >
          <Settings size={16} />
        </ActionButton>
      </div>
      <div className={`source-mode ${sourceModeClass}`}>
        <i />
        Ingestion {statusLoading ? 'loading status…' : sourceModeLabel}
      </div>
      {active.length > 0 && (
        <div className="source-jobs-strip" aria-label="Active source jobs">
          {active.map((job) => (
            <div className="source-job-item" key={job.id}>
              <Spinner />
              <span>
                {job.project} · {job.source} · {job.operation} · {job.status} ·{' '}
                {describeSourceJobProgress(job)}
              </span>
              {onCancelSourceJob && (
                <ActionButton
                  variant="icon"
                  type="button"
                  className="source-job-cancel "
                  aria-label={`Cancel ${job.project} ${job.source} ${job.operation}`}
                  tooltip={`Cancel ${job.project} ${job.source} ${job.operation}`}
                  disabled={job.status === 'cancelling'}
                  onClick={() => onCancelSourceJob(job.id)}
                >
                  <CircleStop size={12} />
                </ActionButton>
              )}
            </div>
          ))}
        </div>
      )}
      {sourceJobError && (
        <p className="document-list-error source-job-error" role="alert">
          {sourceJobError}
          {onRetrySourceJobs && (
            <>
              {' '}
              <ActionButton
                variant="ghost"
                type="button"
                className="link-button"
                onClick={onRetrySourceJobs}
              >
                Retry source jobs
              </ActionButton>
            </>
          )}
        </p>
      )}
      {sourceToggleError && (
        <p className="document-list-error source-job-error" role="alert">
          {sourceToggleError}
        </p>
      )}
      {sourceToggleNotice && (
        <p className="document-list-state source-toggle-notice" role="status">
          {sourceToggleNotice}
        </p>
      )}
      {statusError && status && (
        <p className="document-list-error" role="status">
          {statusError} Showing the last known source index.{' '}
          {onRetryStatus && (
            <ActionButton
              variant="ghost"
              type="button"
              className="link-button"
              onClick={onRetryStatus}
            >
              Retry status
            </ActionButton>
          )}
        </p>
      )}
      {statusLoading ? (
        <div className="space-y-2 p-3" role="status" aria-label="Loading source index and health">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-5/6" />
          <span className="sr-only">Loading source index and health…</span>
        </div>
      ) : statusError && !status ? (
        <p className="document-list-error" role="status">
          {statusError}{' '}
          {onRetryStatus && (
            <ActionButton
              variant="ghost"
              type="button"
              className="link-button"
              onClick={onRetryStatus}
            >
              Retry status
            </ActionButton>
          )}
        </p>
      ) : !projects.length ? (
        <div className="source-empty">
          <Database size={20} />
          <p>No indexed sources yet.</p>
          <span>Configure a source, then run cortana sync.</span>
        </div>
      ) : (
        <div className="source-tree">
          {projects.map(([project, items]) => (
            <section key={project}>
              {items.map((item) => {
                const health = sourceHealth(item)
                const key = `${item.project}:${item.source}`
                const isCollapsed = collapsed.has(key)
                // Source names are only unique inside a workspace. When the
                // panel shows all workspaces, matching by name alone would
                // highlight every same-named connector and make a click
                // appear to select the wrong account.
                const isSelected = selected === item.source && selectedWorkspaceId === item.project
                const auth = item.authorization
                const needsProviderSetup = Boolean(auth?.setup_required)
                const needsBrowserAuthorization =
                  (auth?.method === 'google_oauth' ||
                    auth?.method === 'github_oauth' ||
                    auth?.method === 'discord_rpc') &&
                  !auth.authorized &&
                  !needsProviderSetup
                const sourceJobActive = active.some(
                  (job) =>
                    job.project === item.project &&
                    (job.source === item.source || job.source === item.name)
                )
                return (
                  <div className="source-node" key={key}>
                    <div className="source-row">
                      <ActionButton
                        variant="icon"
                        type="button"
                        className="tree-toggle "
                        aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
                        tooltip={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
                        aria-expanded={!isCollapsed}
                        onClick={() => {
                          setCollapsed((current) => {
                            const next = new Set(current)
                            if (next.has(key)) next.delete(key)
                            else next.add(key)
                            return next
                          })
                        }}
                      >
                        {isCollapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
                      </ActionButton>
                      <Button
                        variant="ghost"
                        type="button"
                        className={`source-select ${isSelected ? 'selected' : ''}`}
                        aria-pressed={isSelected}
                        aria-label={`${item.source} ${item.documents.toLocaleString()}`}
                        onClick={() => onSelect(item.source, item.project)}
                        title={health.label}
                      >
                        <SourceIcon kind={item.kind} size={17} />
                        <span>{sourceDisplayName(item.kind, item.name)}</span>
                        <i className={`source-health ${health.state}`} />
                        <small>{item.documents.toLocaleString()}</small>
                      </Button>
                      {onOpenSourceSetup && needsProviderSetup && (
                        <ActionButton
                          variant="icon"
                          type="button"
                          className="source-action "
                          aria-label={`Open ${item.name} setup`}
                          tooltip={
                            sourceJobActive
                              ? 'Wait for the active source job to finish'
                              : auth?.method === 'google_oauth'
                                ? 'Open Google source settings'
                                : auth?.method === 'github_oauth'
                                  ? 'Open GitHub source settings'
                                  : auth?.method === 'discord_rpc'
                                    ? 'Open Discord source settings'
                                    : 'Open the provider setup page'
                          }
                          disabled={
                            sourceToggleBusy !== null || sourceToggleDisabled || sourceJobActive
                          }
                          onClick={(event) => {
                            event.stopPropagation()
                            onOpenSourceSetup(item.source, item.project)
                          }}
                        >
                          <ExternalLink size={13} />
                        </ActionButton>
                      )}
                      {onAuthorizeSource && needsBrowserAuthorization && (
                        <ActionButton
                          variant="icon"
                          type="button"
                          className="source-action "
                          aria-label={`Authorize ${item.name}`}
                          tooltip={
                            sourceJobActive
                              ? 'Wait for the active source job to finish'
                              : auth?.method === 'github_oauth'
                                ? 'Authorize this GitHub source in your browser'
                                : auth?.method === 'discord_rpc'
                                  ? 'Approve this Discord source in the running Discord Desktop client'
                                  : 'Authorize this Google source in your browser'
                          }
                          disabled={
                            sourceToggleBusy !== null || sourceToggleDisabled || sourceJobActive
                          }
                          onClick={(event) => {
                            event.stopPropagation()
                            onAuthorizeSource(item.source, item.project)
                          }}
                        >
                          <KeyRound size={13} />
                        </ActionButton>
                      )}
                      {onToggleSource && item.kind !== 'indexed' && (
                        <Switch
                          size="sm"
                          checked={item.enabled}
                          aria-busy={sourceToggleBusy === key}
                          aria-label={`${item.enabled ? 'Disable' : 'Enable'} ${item.name}`}
                          disabled={
                            sourceToggleDisabled || sourceToggleBusy !== null || sourceJobActive
                          }
                          onClick={(event) => event.stopPropagation()}
                          onCheckedChange={(checked) =>
                            onToggleSource(item.source, item.project, checked)
                          }
                        />
                      )}
                    </div>
                    {!isCollapsed && (
                      <span className="source-node-hint">
                        {item.chunks.toLocaleString()} chunks · {health.label}
                      </span>
                    )}
                  </div>
                )
              })}
            </section>
          ))}
        </div>
      )}
      <section className="document-explorer" aria-label="Document explorer">
        <div className="document-explorer-heading">
          <strong
            aria-label={`Documents in ${
              selectedWorkspace?.name || selectedWorkspaceId || 'Documents'
            } / ${
              selectedSource
                ? sourceDisplayName(selectedSource.kind, selectedSource.source)
                : 'All sources'
            }`}
          >
            <span className="explorer-workspace">
              {selectedWorkspace?.name || selectedWorkspaceId || 'Documents'}
            </span>
            <span className="explorer-separator" aria-hidden="true">
              /
            </span>
            <span className="explorer-scope">
              {selectedSource
                ? sourceDisplayName(selectedSource.kind, selectedSource.source)
                : 'All sources'}
            </span>
          </strong>
          <span>{documents.length.toLocaleString()} loaded</span>
        </div>
        <label className="document-filter">
          <Search size={14} />
          <Input
            id="document-filter"
            value={documentQuery}
            onChange={(event) => onDocumentQueryChange(event.target.value)}
            placeholder="Filter documents"
            aria-label="Filter documents"
          />
          {documentQuery !== '' && (
            <ActionButton
              variant="icon"
              type="button"
              className="document-filter-clear"
              aria-label="Clear document filter"
              onClick={() => onDocumentQueryChange('')}
            >
              <X size={14} />
            </ActionButton>
          )}
        </label>
        {documentsError ? (
          <p className="document-list-error" role="alert">
            {documentsError}{' '}
            {onRetryDocuments && (
              <ActionButton
                variant="ghost"
                type="button"
                className="link-button"
                onClick={onRetryDocuments}
              >
                Retry documents
              </ActionButton>
            )}
          </p>
        ) : documents.length ? (
          <VirtualDocumentList
            documents={documents}
            selectedDocument={selectedDocument}
            loading={documentsLoading}
            hasMore={hasMoreDocuments}
            onSelect={onSelectDocument}
            onLoadMore={onLoadMoreDocuments}
          />
        ) : (
          <p className="document-list-state">
            {documentsLoading ? 'Loading documents…' : 'No documents match this scope.'}
          </p>
        )}
        {documentsLoading && documents.length > 0 && (
          <Progress value={null} aria-label="Loading more documents" />
        )}
        {hasMoreDocuments && !documentsLoading && (
          <ActionButton
            variant="secondary"
            className="load-more-documents"
            onClick={onLoadMoreDocuments}
          >
            Load next page
          </ActionButton>
        )}
      </section>
    </aside>
  )
}
