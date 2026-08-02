import {
  Bot,
  CalendarDays,
  ChevronDown,
  ChevronRight,
  CircleStop,
  Code2,
  Database,
  Folder,
  LoaderCircle,
  Mail,
  MessageCircle,
  Search,
  Settings,
  StickyNote,
  X,
} from 'lucide-react'
import { useMemo, useState } from 'react'

import { activeJobs, describeSourceJobProgress } from '../sourceJobs'
import { operationalSources, sourceHealth, type OperationalSource } from '../operations'
import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from '../types'
import { VirtualDocumentList } from './VirtualDocumentList'

const sourceIcons: Record<string, typeof Folder> = {
  filesystem: Code2,
  'google-drive': Folder,
  'google-calendar': CalendarDays,
  gmail: Mail,
  'apple-notes': StickyNote,
  discord: MessageCircle,
  slack: MessageCircle,
  buzz: Bot,
}

export function sourceIconForKind(kind: string) {
  return sourceIcons[kind] || Database
}

export function SourcePanel({
  open,
  status,
  statusError,
  sourceJobError = '',
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
  onOpenSourcesSettings,
  onClose,
  onCancelSourceJob,
  jobs = [],
}: {
  open: boolean
  status: BrainStatus | null
  statusError: string
  sourceJobError?: string
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
  onOpenSourcesSettings: () => void
  onClose: () => void
  onCancelSourceJob?: (id: string) => void
  jobs?: DesktopSourceJob[]
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(new Set())
  const sources = useMemo(
    () => operationalSources(status).filter((item) => !workspace || item.project === workspace),
    [status, workspace]
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
    <aside className={`source-panel ${open ? 'mobile-open' : ''}`}>
      <div className="panel-heading">
        <strong>Sources</strong>
        <button type="button" className="mobile-close" aria-label="Close sources" onClick={onClose}>
          <X size={17} />
        </button>
        <button type="button" aria-label="Add source" onClick={onOpenSourcesSettings}>
          +
        </button>
        <button type="button" aria-label="Source settings" onClick={onOpenSourcesSettings}>
          <Settings size={16} />
        </button>
      </div>
      <label className="sidebar-workspace-select">
        <span>Workspace</span>
        <select value={workspace} onChange={(event) => onSelectWorkspace(event.target.value)}>
          <option value="">All workspaces</option>
          {workspaces.map((item) => (
            <option value={item.id} key={item.id}>
              {item.name}
              {item.account_label ? ` · ${item.account_label}` : ''}
            </option>
          ))}
        </select>
      </label>
      <div className={`source-mode ${sourceModeClass}`}>
        <i />
        Ingestion {statusLoading ? 'loading status…' : sourceModeLabel}
      </div>
      {active.length > 0 && (
        <div className="source-jobs-strip" aria-label="Active source jobs">
          {active.map((job) => (
            <div className="source-job-item" key={job.id}>
              <LoaderCircle className="spin" size={12} />
              <span>
                {job.project} · {job.source} · {job.operation} · {job.status} ·{' '}
                {describeSourceJobProgress(job)}
              </span>
              {onCancelSourceJob && (
                <button
                  type="button"
                  className="source-job-cancel"
                  aria-label={`Cancel ${job.project} ${job.source} ${job.operation}`}
                  disabled={job.status === 'cancelling'}
                  onClick={() => onCancelSourceJob(job.id)}
                >
                  <CircleStop size={12} />
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {sourceJobError && (
        <p className="document-list-error source-job-error" role="alert">
          {sourceJobError}
        </p>
      )}
      {statusError && status && (
        <p className="document-list-error" role="status">
          {statusError} Showing the last known source index.
        </p>
      )}
      {statusLoading ? (
        <p className="document-list-state" role="status">
          Loading source index and health…
        </p>
      ) : statusError && !status ? (
        <p className="document-list-error" role="status">
          {statusError}
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
              <button
                type="button"
                className="project-row"
                aria-expanded={!collapsedProjects.has(project)}
                onClick={() =>
                  setCollapsedProjects((current) => {
                    const next = new Set(current)
                    if (next.has(project)) next.delete(project)
                    else next.add(project)
                    return next
                  })
                }
              >
                {collapsedProjects.has(project) ? (
                  <ChevronRight size={14} />
                ) : (
                  <ChevronDown size={14} />
                )}
                <Folder size={17} />
                <strong>{project[0]?.toUpperCase() + project.slice(1)}</strong>
                <span>{items.reduce((sum, item) => sum + item.documents, 0).toLocaleString()}</span>
              </button>
              {!collapsedProjects.has(project) &&
                items.map((item) => {
                  const Icon = sourceIconForKind(item.kind)
                  const health = sourceHealth(item)
                  const key = `${item.project}:${item.source}`
                  const isCollapsed = collapsed.has(key)
                  // Source names are only unique inside a workspace. When the
                  // panel shows all workspaces, matching by name alone would
                  // highlight every same-named connector and make a click
                  // appear to select the wrong account.
                  const isSelected = selected === item.source && workspace === item.project
                  return (
                    <div className="source-node" key={key}>
                      <div className="source-row">
                        <button
                          type="button"
                          className="tree-toggle"
                          aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
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
                        </button>
                        <button
                          type="button"
                          className={`source-select ${isSelected ? 'selected' : ''}`}
                          aria-pressed={isSelected}
                          onClick={() => onSelect(item.source, item.project)}
                          title={health.label}
                        >
                          <Icon size={17} />
                          <span>{item.name}</span>
                          <i className={`source-health ${health.state}`} />
                          <small>{item.documents.toLocaleString()}</small>
                        </button>
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
      <section className="document-explorer">
        <div className="document-explorer-heading">
          <strong>{selected || workspace || 'All documents'}</strong>
          <span>{documents.length.toLocaleString()} loaded</span>
        </div>
        <label className="document-filter">
          <Search size={14} />
          <input
            id="document-filter"
            value={documentQuery}
            onChange={(event) => onDocumentQueryChange(event.target.value)}
            placeholder="Filter documents"
            aria-label="Filter documents"
          />
          {documentQuery !== '' && (
            <button
              type="button"
              className="document-filter-clear"
              aria-label="Clear document filter"
              onClick={() => onDocumentQueryChange('')}
            >
              <X size={14} />
            </button>
          )}
        </label>
        {documentsError ? (
          <p className="document-list-error" role="alert">
            {documentsError}
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
          <p className="document-list-state" aria-live="polite">
            Loading more…
          </p>
        )}
        {hasMoreDocuments && !documentsLoading && (
          <button type="button" className="load-more-documents" onClick={onLoadMoreDocuments}>
            Load next page
          </button>
        )}
      </section>
    </aside>
  )
}
