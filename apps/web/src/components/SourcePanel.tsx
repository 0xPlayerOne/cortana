import {
  Bot,
  ChevronDown,
  ChevronRight,
  Code2,
  Database,
  Folder,
  Mail,
  MessageCircle,
  Settings,
  StickyNote,
  X,
} from 'lucide-react'
import { useMemo } from 'react'

import type { SourceSummary } from '../types'

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
  sources,
  selected,
  onSelect,
  onClose,
}: {
  open: boolean
  sources: SourceSummary[]
  selected: string
  onSelect: (source: string) => void
  onClose: () => void
}) {
  const projects = useMemo(
    () =>
      Object.entries(
        sources.reduce<Record<string, SourceSummary[]>>((groups, source) => {
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
        <button aria-label="Add source">+</button>
        <button aria-label="Source settings">
          <Settings size={16} />
        </button>
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
                return (
                  <button
                    key={item.source}
                    className={selected === item.source ? 'selected' : ''}
                    onClick={() => onSelect(item.source)}
                  >
                    <ChevronRight size={13} />
                    <Icon size={17} />
                    <span>{item.source}</span>
                    <small>{item.documents.toLocaleString()}</small>
                  </button>
                )
              })}
            </section>
          ))}
        </div>
      )}
    </aside>
  )
}
