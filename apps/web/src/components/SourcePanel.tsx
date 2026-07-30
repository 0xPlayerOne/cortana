import {
  Bot,
  ChevronDown,
  ChevronRight,
  Code2,
  Database,
  Folder,
  Mail,
  MessageCircle,
  Search,
  Settings,
  StickyNote,
  X,
} from 'lucide-react'
import { useMemo, useState } from 'react'

import { operationalSources, sourceHealth, type OperationalSource } from '../operations'
import type { BrainDocumentSummary, BrainStatus, WorkspaceSettings } from '../types'
import { VirtualDocumentList } from './VirtualDocumentList'

const sourceIcons: Record<string, typeof Folder> = {
  code: Code2,
  drive: Folder,
  gmail: Mail,
  notes: StickyNote,
  discord: MessageCircle,
  slack: MessageCircle,
  buzz: Bot,
}

function sourceIcon(source: string) {
  const key = Object.keys(sourceIcons).find((name) => source.includes(name))
  return key ? sourceIcons[key] : Database
}

export function SourcePanel({
  open,
  status,
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
  onOpenSettings,
  onClose,
}: {
  open: boolean
  status: BrainStatus | null
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
  onOpenSettings: () => void
  onClose: () => void
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

  return (
    <aside className={`source-panel ${open ? 'mobile-open' : ''}`}>
      <div className="panel-heading">
        <strong>Sources</strong>
        <button className="mobile-close" aria-label="Close sources" onClick={onClose}>
          <X size={17} />
        </button>
        <button aria-label="Add source" onClick={onOpenSettings}>
          +
        </button>
        <button aria-label="Source settings" onClick={onOpenSettings}>
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
      <div className={`source-mode ${status?.ingestion.scheduled ? 'scheduled' : 'manual'}`}>
        <i />
        Ingestion {status?.ingestion.scheduled ? 'scheduled' : 'paused · manual only'}
      </div>
      {!projects.length ? (
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
                  const Icon = sourceIcon(item.source)
                  const health = sourceHealth(item)
                  const key = `${item.project}:${item.source}`
                  const isCollapsed = collapsed.has(key)
                  return (
                    <div className="source-node" key={key}>
                      <div className="source-row">
                        <button
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
                          className={`source-select ${selected === item.source ? 'selected' : ''}`}
                          aria-pressed={selected === item.source}
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
          <button className="load-more-documents" onClick={onLoadMoreDocuments}>
            Load next page
          </button>
        )}
      </section>
    </aside>
  )
}
