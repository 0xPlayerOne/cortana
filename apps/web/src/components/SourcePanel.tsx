import {
  Bot,
  ChevronDown,
  ChevronRight,
  Code2,
  Database,
  FileText,
  Folder,
  Mail,
  MessageCircle,
  Settings,
  StickyNote,
  X,
} from 'lucide-react'
import { useMemo, useState } from 'react'

import { operationalSources, sourceHealth, type OperationalSource } from '../operations'
import type { BrainDocumentSummary, BrainStatus } from '../types'

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
  selected,
  documents,
  selectedDocument,
  documentsLoading,
  documentsError,
  hasMoreDocuments,
  onSelect,
  onSelectDocument,
  onLoadMoreDocuments,
  onOpenSettings,
  onClose,
}: {
  open: boolean
  status: BrainStatus | null
  workspace: string
  selected: string
  documents: BrainDocumentSummary[]
  selectedDocument: string
  documentsLoading: boolean
  documentsError: string
  hasMoreDocuments: boolean
  onSelect: (source: string) => void
  onSelectDocument: (id: string) => void
  onLoadMoreDocuments: () => void
  onOpenSettings: () => void
  onClose: () => void
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
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
              <div className="project-row">
                <ChevronDown size={14} />
                <Folder size={17} />
                <strong>{project[0]?.toUpperCase() + project.slice(1)}</strong>
                <span>{items.reduce((sum, item) => sum + item.documents, 0).toLocaleString()}</span>
              </div>
              {items.map((item) => {
                const Icon = sourceIcon(item.source)
                const health = sourceHealth(item)
                const key = `${item.project}:${item.source}`
                const itemDocuments = documents.filter(
                  (document) => document.project === item.project && document.source === item.source
                )
                const isCollapsed = collapsed.has(key)
                return (
                  <div className="source-node" key={key}>
                    <div className="source-row">
                      <button
                        className="tree-toggle"
                        aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${item.name}`}
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
                        onClick={() => onSelect(item.source)}
                        title={health.label}
                      >
                        <Icon size={17} />
                        <span>{item.name}</span>
                        <i className={`source-health ${health.state}`} />
                        <small>{item.documents.toLocaleString()}</small>
                      </button>
                    </div>
                    {!isCollapsed && (
                      <div className="document-tree">
                        {itemDocuments.map((document) => (
                          <button
                            key={document.id}
                            className={selectedDocument === document.id ? 'selected-document' : ''}
                            onClick={() => onSelectDocument(document.id)}
                            title={document.title}
                          >
                            <FileText size={14} />
                            <span>{document.title}</span>
                          </button>
                        ))}
                        {!documentsLoading && !documentsError && itemDocuments.length === 0 && (
                          <span className="document-tree-empty">
                            {selected && selected !== item.source
                              ? 'Select source to load documents'
                              : 'No documents in this page'}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                )
              })}
            </section>
          ))}
          {documentsError && <p className="document-list-error">{documentsError}</p>}
          {documentsLoading && <p className="document-list-state">Loading documents…</p>}
          {hasMoreDocuments && !documentsLoading && (
            <button className="load-more-documents" onClick={onLoadMoreDocuments}>
              Load more documents
            </button>
          )}
        </div>
      )}
    </aside>
  )
}
