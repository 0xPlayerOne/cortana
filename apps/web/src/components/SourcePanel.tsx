import {
  ChevronDown,
  ChevronRight,
  CircleStop,
  Database,
  ExternalLink,
  KeyRound,
  LoaderCircle,
  Search,
  Settings,
  X,
} from 'lucide-react'
import { createContext, useContext, useMemo, useState } from 'react'

import { activeJobs, describeSourceJobProgress } from '../sourceJobs'
import { operationalSources, sourceHealth, type OperationalSource } from '../operations'
import { WorkspaceLogo } from '../workspaceLogos'
import { useM7SurfacePrimitives } from './m7/M7SurfacePrimitives'
import { SourceIcon } from './sourceIcons'
import { sourceDisplayName } from './sourceIconData'
import { Button, type ButtonProps } from './ui/Button'
import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from '../types'
import { VirtualDocumentList } from './VirtualDocumentList'

const SourceRendererContext = createContext<'legacy' | 'shadcn'>('legacy')

function ActionButton({ variant = 'secondary', ...props }: ButtonProps) {
  const renderer = useContext(SourceRendererContext)
  const ShadcnButton = useM7SurfacePrimitives()?.Button
  if (renderer === 'legacy' || !ShadcnButton) return <Button variant={variant} {...props} />
  return (
    <ShadcnButton
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
  onSelectWorkspace,
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
  renderer = 'legacy',
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
  onSelectWorkspace: (workspace: string) => void
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
  renderer?: 'legacy' | 'shadcn'
}) {
  const primitives = useM7SurfacePrimitives()
  const ShadcnButton = primitives?.Button
  const ShadcnInput = primitives?.Input
  const ShadcnProgress = primitives?.Progress
  const ShadcnSelect = primitives?.Select
  const ShadcnSelectContent = primitives?.SelectContent
  const ShadcnSelectItem = primitives?.SelectItem
  const ShadcnSelectTrigger = primitives?.SelectTrigger
  const ShadcnSelectValue = primitives?.SelectValue
  const ShadcnSkeleton = primitives?.Skeleton
  const ShadcnSpinner = primitives?.Spinner
  const ShadcnSwitch = primitives?.Switch
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
    <SourceRendererContext.Provider value={renderer}>
      <aside
        className={`source-panel ${open ? 'mobile-open' : ''} ${renderer === 'shadcn' ? 'm7-source-panel' : ''}`}
        data-m7-source-panel={renderer === 'shadcn' ? '' : undefined}
      >
        <div className="panel-heading">
          <strong>Sources</strong>
          <ActionButton
            variant="icon"
            className="mobile-close quick-tooltip"
            aria-label="Close sources"
            data-tooltip="Close sources"
            onClick={onClose}
          >
            <X size={17} />
          </ActionButton>
          <ActionButton
            variant="icon"
            className="quick-tooltip"
            aria-label="Add source"
            data-tooltip="Add source"
            onClick={onOpenSourcesSettings}
          >
            +
          </ActionButton>
          <ActionButton
            variant="icon"
            className="quick-tooltip"
            aria-label="Source settings"
            data-tooltip="Source settings"
            onClick={onOpenSourcesSettings}
          >
            <Settings size={16} />
          </ActionButton>
        </div>
        <label className="sidebar-workspace-select">
          <span>Workspace</span>
          <div className="workspace-picker">
            {selectedWorkspace && <WorkspaceLogo workspace={selectedWorkspace} size="small" />}
            {renderer === 'shadcn' &&
            ShadcnSelect &&
            ShadcnSelectContent &&
            ShadcnSelectItem &&
            ShadcnSelectTrigger &&
            ShadcnSelectValue ? (
              <ShadcnSelect
                value={selectedWorkspaceId}
                onValueChange={(value) => value && onSelectWorkspace(value)}
              >
                <ShadcnSelectTrigger className="w-full" aria-label="Workspace">
                  <ShadcnSelectValue />
                </ShadcnSelectTrigger>
                <ShadcnSelectContent align="start">
                  {workspaces.map((item) => (
                    <ShadcnSelectItem value={item.id} key={item.id}>
                      {item.name}
                    </ShadcnSelectItem>
                  ))}
                </ShadcnSelectContent>
              </ShadcnSelect>
            ) : (
              <select
                value={selectedWorkspaceId}
                onChange={(event) => onSelectWorkspace(event.target.value)}
                aria-label="Workspace"
              >
                {workspaces.map((item) => (
                  <option value={item.id} key={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
            )}
          </div>
        </label>
        <div className={`source-mode ${sourceModeClass}`}>
          <i />
          Ingestion {statusLoading ? 'loading status…' : sourceModeLabel}
        </div>
        {active.length > 0 && (
          <div className="source-jobs-strip" aria-label="Active source jobs">
            {active.map((job) => (
              <div className="source-job-item" key={job.id}>
                {renderer === 'shadcn' && ShadcnSpinner ? (
                  <ShadcnSpinner />
                ) : (
                  <LoaderCircle className="spin" size={12} />
                )}
                <span>
                  {job.project} · {job.source} · {job.operation} · {job.status} ·{' '}
                  {describeSourceJobProgress(job)}
                </span>
                {onCancelSourceJob && (
                  <ActionButton
                    variant="icon"
                    type="button"
                    className="source-job-cancel quick-tooltip"
                    aria-label={`Cancel ${job.project} ${job.source} ${job.operation}`}
                    data-tooltip={`Cancel ${job.project} ${job.source} ${job.operation}`}
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
          renderer === 'shadcn' && ShadcnSkeleton ? (
            <div
              className="space-y-2 p-3"
              role="status"
              aria-label="Loading source index and health"
            >
              <ShadcnSkeleton className="h-8 w-full" />
              <ShadcnSkeleton className="h-8 w-5/6" />
              <span className="sr-only">Loading source index and health…</span>
            </div>
          ) : (
            <p className="document-list-state" role="status">
              Loading source index and health…
            </p>
          )
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
                  const isSelected =
                    selected === item.source && selectedWorkspaceId === item.project
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
                          className="tree-toggle quick-tooltip"
                          aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
                          data-tooltip={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
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
                        {renderer === 'shadcn' && ShadcnButton ? (
                          <ShadcnButton
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
                          </ShadcnButton>
                        ) : (
                          <button
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
                          </button>
                        )}
                        {onOpenSourceSetup && needsProviderSetup && (
                          <ActionButton
                            variant="icon"
                            type="button"
                            className="source-action quick-tooltip"
                            aria-label={`Open ${item.name} setup`}
                            data-tooltip={
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
                            className="source-action quick-tooltip"
                            aria-label={`Authorize ${item.name}`}
                            data-tooltip={
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
                        {onToggleSource &&
                          item.kind !== 'indexed' &&
                          (renderer === 'shadcn' && ShadcnSwitch ? (
                            <ShadcnSwitch
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
                          ) : (
                            <button
                              type="button"
                              role="switch"
                              className={`source-enable-toggle ${item.enabled ? 'enabled' : ''} quick-tooltip`}
                              aria-checked={item.enabled}
                              aria-busy={sourceToggleBusy === key}
                              aria-label={`${item.enabled ? 'Disable' : 'Enable'} ${item.name}`}
                              data-tooltip={
                                sourceJobActive
                                  ? 'Wait for the active source job to finish'
                                  : sourceToggleBusy !== null
                                    ? 'Saving source setting…'
                                    : sourceToggleDisabled
                                      ? 'Save or discard settings changes before toggling a source'
                                      : `${item.enabled ? 'Disable' : 'Enable'} ${item.name}`
                              }
                              disabled={
                                sourceToggleDisabled || sourceToggleBusy !== null || sourceJobActive
                              }
                              onClick={(event) => {
                                event.stopPropagation()
                                onToggleSource(item.source, item.project, !item.enabled)
                              }}
                            >
                              <span />
                            </button>
                          ))}
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
            {renderer === 'shadcn' && ShadcnInput ? (
              <ShadcnInput
                id="document-filter"
                value={documentQuery}
                onChange={(event) => onDocumentQueryChange(event.target.value)}
                placeholder="Filter documents"
                aria-label="Filter documents"
              />
            ) : (
              <input
                id="document-filter"
                value={documentQuery}
                onChange={(event) => onDocumentQueryChange(event.target.value)}
                placeholder="Filter documents"
                aria-label="Filter documents"
              />
            )}
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
              renderer={renderer}
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
          {documentsLoading &&
            documents.length > 0 &&
            (renderer === 'shadcn' && ShadcnProgress ? (
              <ShadcnProgress value={null} aria-label="Loading more documents" />
            ) : (
              <p className="document-list-state" aria-live="polite">
                Loading more…
              </p>
            ))}
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
    </SourceRendererContext.Provider>
  )
}
