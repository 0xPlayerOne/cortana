import { BookOpen, FileText, History, Link2, Network, Star } from 'lucide-react'
import { type CSSProperties, useEffect, useState } from 'react'

import { isDesktopApp, openDesktopUrl } from '../api'
import { isFavoriteDocument, toggleFavoriteDocument } from '../favoriteDocuments'
import { safeSourceLink } from '../sourceLinks'
import type { AnswerResponse, BrainDocument, BrainGraphPage, Evidence } from '../types'

const tabs = [
  { id: 'answer', label: 'Answer', icon: AppIcon },
  { id: 'document', label: 'Document', icon: BookOpen },
  { id: 'sources', label: 'Evidence', icon: FileText },
  { id: 'timeline', label: 'Timeline', icon: History },
] as const

export type WorkspaceTab = (typeof tabs)[number]['id'] | 'graph'

// Result-only views stay inert until a search returns an answer or evidence.
// The Document tab is the default primary view and Graph remains an explicit
// separate view, so neither is gated.
const resultGatedTabs = new Set<WorkspaceTab>(['answer', 'sources', 'timeline'])

async function openSourceLink(href: string): Promise<boolean> {
  if (!isDesktopApp) return false
  if (!safeSourceLink(href, { allowLocalFile: true })) return false
  try {
    await openDesktopUrl(href)
    return true
  } catch {
    // Desktop URL policy is enforced natively. Never fall back to a renderer
    // window, which could bypass the configured-root check for file links.
    return false
  }
}

export function Workspace({
  query,
  answer,
  evidence,
  selected,
  loading,
  error,
  document,
  documentLoading,
  graph,
  graphLoading,
  graphAppendLoading = false,
  graphError,
  onLoadMoreGraph,
  onRetryGraph,
  tab,
  onTabChange,
  onSelect,
  onSelectDocument,
  onRetry,
}: {
  query: string
  answer: AnswerResponse | null
  evidence: Evidence[]
  selected: number
  loading: boolean
  error: string
  document: BrainDocument | null
  documentLoading: boolean
  graph: BrainGraphPage | null
  graphLoading: boolean
  graphAppendLoading?: boolean
  graphError: string
  onLoadMoreGraph?: () => void
  onRetryGraph?: () => void
  tab: WorkspaceTab
  onTabChange: (tab: WorkspaceTab) => void
  onSelect: (index: number) => void
  onSelectDocument: (id: string) => void
  onRetry: () => void
}) {
  const active = evidence[selected] ?? null
  const hasResults = answer !== null || evidence.length > 0
  const selectEvidenceByChunkId = (chunkId: string) => {
    const next = evidence.findIndex((item) => item.chunk_id === chunkId)
    if (next >= 0) {
      onSelect(next)
      onTabChange('sources')
    }
  }

  useEffect(() => {
    if (document) onTabChange('document')
  }, [document, onTabChange])
  useEffect(() => {
    if (answer) onTabChange('answer')
  }, [answer, onTabChange])
  useEffect(() => {
    // Keep an explicitly submitted search visible while retrieval is in
    // flight. The result tab is hidden from the tab strip until evidence
    // arrives, but redirecting it immediately would replace the loading
    // state with the idle document view.
    if (!loading && !hasResults && resultGatedTabs.has(tab)) onTabChange('document')
  }, [hasResults, loading, onTabChange, tab])

  return (
    <main className="workspace">
      {tab !== 'graph' && (
        <div className="workspace-tabs" role="tablist" aria-label="Result views">
          {tabs
            .filter(({ id }) => id === 'document' || hasResults)
            .map(({ id, label, icon: Icon }) => (
              <button
                type="button"
                key={id}
                role="tab"
                aria-selected={tab === id}
                className={tab === id ? 'active' : ''}
                onClick={() => onTabChange(id)}
              >
                <Icon size={15} />
                {label}
                {id === 'document' && document && <span className="count-pill">1</span>}
                {id === 'sources' && evidence.length > 0 && (
                  <span className="count-pill">{evidence.length}</span>
                )}
              </button>
            ))}
        </div>
      )}
      {documentLoading ? (
        <EmptyState title="Opening document" detail="Loading the canonical indexed content…" />
      ) : tab === 'document' && document ? (
        <BrainDocumentView document={document} onSelectDocument={onSelectDocument} />
      ) : tab === 'graph' ? (
        <GraphView
          graph={graph}
          graphLoading={graphLoading}
          graphAppendLoading={graphAppendLoading}
          graphError={graphError}
          onLoadMore={onLoadMoreGraph}
          onRetry={onRetryGraph}
          evidence={evidence}
          onSelect={selectEvidenceByChunkId}
          onSelectDocument={onSelectDocument}
        />
      ) : error ? (
        <EmptyState
          title="Cortana could not reach the brain"
          detail={`${error}. Start the Rust API or add ?demo=1 to preview the workspace.`}
          action={onRetry}
        />
      ) : tab === 'document' ? (
        <EmptyState
          title="Choose a document"
          detail="Open a workspace and source in the sidebar, then select any indexed document."
        />
      ) : loading && evidence.length === 0 ? (
        <EmptyState
          title="Searching your brain"
          detail="Fusing semantic and exact-term evidence…"
        />
      ) : evidence.length === 0 ? (
        <EmptyState title="No evidence found" detail="Try a broader phrase or another source." />
      ) : tab === 'timeline' ? (
        <TimelineView evidence={evidence} onSelect={selectEvidenceByChunkId} />
      ) : tab === 'answer' ? (
        <AnswerView
          query={query}
          response={answer}
          evidence={evidence}
          onSelect={(index) => {
            onSelect(index)
            onTabChange('sources')
          }}
        />
      ) : (
        active && <DocumentView active={active} evidence={evidence} onSelect={onSelect} />
      )}
    </main>
  )
}

function BrainDocumentView({
  document,
  onSelectDocument,
}: {
  document: BrainDocument
  onSelectDocument: (id: string) => void
}) {
  const [favorite, setFavorite] = useState(() => isFavoriteDocument(document.id))
  const [sourceOpenError, setSourceOpenError] = useState(false)
  useEffect(() => setFavorite(isFavoriteDocument(document.id)), [document.id])
  useEffect(() => setSourceOpenError(false), [document.id])
  const metadata = Object.entries(document.metadata).slice(0, 24)
  const sourceHref = document.uri
    ? safeSourceLink(document.uri, { allowLocalFile: isDesktopApp })
    : null
  return (
    <article className="document canonical-document">
      <div className="breadcrumbs">
        <span>Brain</span> / <span>{document.project}</span> / <span>{document.source}</span> /{' '}
        <strong>{document.title}</strong>
        <div>
          <button
            type="button"
            aria-label={favorite ? 'Remove favorite' : 'Add favorite'}
            aria-pressed={favorite}
            title={favorite ? 'Remove favorite' : 'Add favorite'}
            data-tooltip={favorite ? 'Remove favorite' : 'Add favorite'}
            className="quick-tooltip"
            onClick={() => setFavorite(toggleFavoriteDocument(document.id))}
          >
            <Star size={17} fill={favorite ? 'currentColor' : 'none'} />
          </button>
          {sourceHref && (
            <a
              href={sourceHref}
              target={isDesktopApp ? undefined : '_blank'}
              rel={isDesktopApp ? undefined : 'noreferrer'}
              aria-label="Open original source"
              title="Open original source"
              data-tooltip="Open original source"
              className="quick-tooltip"
              onClick={(event) => {
                if (!isDesktopApp) return
                const uri = sourceHref
                event.preventDefault()
                setSourceOpenError(false)
                void openSourceLink(uri).then((opened) => {
                  if (!opened) setSourceOpenError(true)
                })
              }}
            >
              <Link2 size={17} />
            </a>
          )}
        </div>
      </div>
      {sourceOpenError && (
        <p className="answer-warning source-link-error" role="alert">
          Cortana could not open the original source. Check that the source app is installed and try
          again.
        </p>
      )}
      <div className="document-grid">
        <div className="document-body">
          <h1>{document.title}</h1>
          <p className="byline">
            {document.project} · {document.source} ·{' '}
            {new Date(document.updated_at).toLocaleString()} · {document.chunk_count} indexed chunks
          </p>
          <div className="document-labels" aria-label="Document security and provenance">
            <span>Workspace: {document.project}</span>
            <span>Source ID: {document.source_id}</span>
            {(document.acl.length ? document.acl : ['public']).map((label) => (
              <span key={label}>ACL: {label}</span>
            ))}
          </div>
          <div className="rule" />
          <div className="canonical-content">
            {document.content.split(/\n{2,}/).map((paragraph, index) => (
              <p key={`${document.id}:${index}`}>{paragraph}</p>
            ))}
          </div>
          {document.truncated && (
            <p className="answer-warning">
              This unusually large document was safely truncated at the desktop display limit. Open
              the original source for the complete content.
            </p>
          )}
          {(document.backlinks.length > 0 || document.surrounding.length > 0) && (
            <div className="document-relations">
              {document.backlinks.length > 0 && (
                <section>
                  <h2>Backlinks</h2>
                  {document.backlinks.map((related) => (
                    <button
                      type="button"
                      key={related.id}
                      onClick={() => onSelectDocument(related.id)}
                    >
                      <Link2 size={14} />
                      <span>{related.title}</span>
                      <small>{related.source}</small>
                    </button>
                  ))}
                </section>
              )}
              {document.surrounding.length > 0 && (
                <section>
                  <h2>Surrounding documents</h2>
                  {document.surrounding.map((related) => (
                    <button
                      type="button"
                      key={related.id}
                      onClick={() => onSelectDocument(related.id)}
                    >
                      <FileText size={14} />
                      <span>{related.title}</span>
                      <small>{new Date(related.updated_at).toLocaleDateString()}</small>
                    </button>
                  ))}
                </section>
              )}
            </div>
          )}
        </div>
        <aside className="document-outline">
          <strong>Indexed document</strong>
          <span>{document.content_chars.toLocaleString()} characters</span>
          <span>{document.chunk_count.toLocaleString()} retrieval chunks</span>
          <span>{document.source}</span>
          <span title={document.source_id}>{document.source_id}</span>
          {metadata.length > 0 && (
            <details className="document-metadata">
              <summary>Metadata ({metadata.length})</summary>
              <dl>
                {metadata.map(([key, value]) => (
                  <div key={key}>
                    <dt>{key}</dt>
                    <dd>{formatMetadata(value)}</dd>
                  </div>
                ))}
              </dl>
            </details>
          )}
          <BookOpen size={56} />
          <small>Canonical content protected by workspace ACLs</small>
        </aside>
      </div>
    </article>
  )
}

function formatMetadata(value: unknown) {
  if (value === null) return 'null'
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  const serialized = JSON.stringify(value)
  return serialized.length > 300 ? `${serialized.slice(0, 297)}…` : serialized
}

function DocumentView({
  active,
  evidence,
  onSelect,
}: {
  active: Evidence
  evidence: Evidence[]
  onSelect: (index: number) => void
}) {
  const [favorite, setFavorite] = useState(() => isFavoriteDocument(active.chunk_id))
  const [sourceOpenError, setSourceOpenError] = useState(false)
  useEffect(() => setFavorite(isFavoriteDocument(active.chunk_id)), [active.chunk_id])
  useEffect(() => setSourceOpenError(false), [active.chunk_id])
  const sourceHref = active.uri
    ? safeSourceLink(active.uri, { allowLocalFile: isDesktopApp })
    : null

  return (
    <article className="document">
      <div className="breadcrumbs">
        <span>Brain</span> / <span>{active.source}</span> / <strong>{active.title}</strong>
        <div>
          <button
            type="button"
            aria-label={favorite ? 'Remove favorite' : 'Add favorite'}
            aria-pressed={favorite}
            title={favorite ? 'Remove favorite' : 'Add favorite'}
            data-tooltip={favorite ? 'Remove favorite' : 'Add favorite'}
            className="quick-tooltip"
            onClick={() => setFavorite(toggleFavoriteDocument(active.chunk_id))}
          >
            <Star size={17} fill={favorite ? 'currentColor' : 'none'} />
          </button>
          {sourceHref && (
            <a
              href={sourceHref}
              target={isDesktopApp ? undefined : '_blank'}
              rel={isDesktopApp ? undefined : 'noreferrer'}
              aria-label="Open original source"
              title="Open original source"
              data-tooltip="Open original source"
              className="quick-tooltip"
              onClick={(event) => {
                if (!isDesktopApp) return
                event.preventDefault()
                setSourceOpenError(false)
                void openSourceLink(sourceHref).then((opened) => {
                  if (!opened) setSourceOpenError(true)
                })
              }}
            >
              <Link2 size={17} />
            </a>
          )}
        </div>
      </div>
      <div className="document-grid">
        <div className="document-body">
          <h1>{active.title}</h1>
          <p className="byline">
            Retrieved from {active.source} · {new Date(active.updated_at).toLocaleString()}
          </p>
          <div className="rule" />
          <div id="passage">
            {active.content.split(/\n{2,}/).map((paragraph, index) => (
              <p key={`${active.chunk_id}:${index}`}>{paragraph}</p>
            ))}
          </div>
          <div id="related" className="evidence-footer">
            <h2>Related evidence</h2>
            {evidence.slice(0, 6).map((item, index) => (
              <button type="button" key={item.chunk_id} onClick={() => onSelect(index)}>
                <span>{index + 1}</span> {item.title}
              </button>
            ))}
          </div>
        </div>
        <aside className="document-outline">
          <strong>In this evidence</strong>
          <a href="#passage">Retrieved passage</a>
          <a href="#related">Related evidence</a>
          <Network size={56} />
          <small>{evidence.length} linked results</small>
        </aside>
      </div>
      {sourceOpenError && (
        <p className="answer-warning source-link-error" role="alert">
          Cortana could not open the original source. Check that the source app is installed and try
          again.
        </p>
      )}
    </article>
  )
}

function AnswerView({
  query,
  response,
  evidence,
  onSelect,
}: {
  query: string
  response: AnswerResponse | null
  evidence: Evidence[]
  onSelect: (index: number) => void
}) {
  return (
    <article className="answer-view">
      <span className="eyebrow">
        <AppIcon size={14} /> Evidence brief
      </span>
      <h1>{query}</h1>
      {response && (
        <div className="answer-meta">
          <span>{response.mode}</span>
          <span>
            {response.retrieval_degraded
              ? 'lexical fallback'
              : response.retrieval_mode || 'hybrid retrieval'}
          </span>
          <span>{response.cached ? 'cache hit' : `${response.latency_ms} ms`}</span>
          <span>
            {response.plan.queries.length}{' '}
            {response.plan.queries.length === 1 ? 'retrieval' : 'retrievals'}
          </span>
        </div>
      )}
      <div className="answer-copy">
        {(response?.answer ?? 'Cortana found relevant evidence below.')
          .split(/\n{2,}/)
          .map((paragraph, index) => (
            <p key={`${paragraph.slice(0, 24)}:${index}`}>{paragraph}</p>
          ))}
      </div>
      {response && response.plan.queries.length > 1 && (
        <details className="answer-plan">
          <summary>Retrieval plan</summary>
          <ol>
            {response.plan.queries.map((plannedQuery, index) => (
              <li key={`${plannedQuery}:${index}`}>{plannedQuery}</li>
            ))}
          </ol>
        </details>
      )}
      {response?.warnings.map((warning, index) => (
        <p className="answer-warning" key={`${warning}:${index}`}>
          {warning}
        </p>
      ))}
      {response?.retrieval_degraded && (
        <p className="answer-warning" role="status">
          Embedding retrieval is temporarily unavailable; these citations came from exact-term
          search.
        </p>
      )}
      <p className="lead">{evidence.length} cited passages</p>
      {evidence.slice(0, 4).map((item, index) => (
        <button
          type="button"
          className="answer-source"
          key={item.chunk_id}
          onClick={() => onSelect(index)}
        >
          <span>[{index + 1}]</span>
          <div>
            <h2>{item.title}</h2>
            <p>{item.content}</p>
          </div>
        </button>
      ))}
      <p className="answer-note">
        {response?.mode === 'synthesized'
          ? 'Synthesized from the cited passages. Open a source to inspect the original evidence.'
          : 'Extractive mode keeps citations stable when no synthesis model is configured.'}
      </p>
    </article>
  )
}

function GraphView({
  graph,
  graphLoading,
  graphAppendLoading = false,
  graphError,
  onRetry,
  onLoadMore,
  evidence,
  onSelect,
  onSelectDocument,
}: {
  graph: BrainGraphPage | null
  graphLoading: boolean
  graphAppendLoading: boolean
  graphError: string
  onRetry?: () => void
  onLoadMore?: () => void
  evidence: Evidence[]
  onSelect: (chunkId: string) => void
  onSelectDocument: (id: string) => void
}) {
  const [visibleCount, setVisibleCount] = useState(12)
  const graphDocuments = graph?.nodes.filter((node) => node.kind === 'document') ?? []
  const usingEvidenceFallback = graph === null
  useEffect(() => {
    setVisibleCount(12)
  }, [graph?.nodes[0]?.id])
  const nodes = graphDocuments.length
    ? graphDocuments.slice(0, visibleCount)
    : usingEvidenceFallback
      ? evidence.slice(0, 8).map((item) => ({
          id: item.chunk_id,
          kind: 'document' as const,
          label: item.title,
          project: '',
          source: item.source,
          document_id: null,
        }))
      : []
  const visibleNodeIds = new Set(nodes.map((node) => node.id))
  const visibleEdges =
    graph && !usingEvidenceFallback
      ? graph.edges.filter((edge) => visibleNodeIds.has(edge.target))
      : []
  if (graphLoading && nodes.length === 0) {
    return (
      <EmptyState
        title="Loading knowledge graph"
        detail="Mapping indexed workspaces and documents…"
      />
    )
  }
  if (graphError && nodes.length === 0) {
    return <EmptyState title="Graph unavailable" detail={graphError} action={onRetry} />
  }
  if (!graphLoading && nodes.length === 0) {
    return (
      <EmptyState title="No graph data" detail="Index a source to build linked workspace nodes." />
    )
  }
  return (
    <div className="graph-view">
      {visibleEdges.length > 0 && (
        <svg className="graph-links" viewBox="0 0 100 100" aria-hidden="true" focusable="false">
          {nodes.map((node, index) => {
            if (!visibleEdges.some((edge) => edge.target === node.id)) return null
            const angle = (index / Math.max(nodes.length, 1)) * Math.PI * 2 - Math.PI / 2
            return (
              <line
                key={`edge:${node.id}`}
                x1="50"
                y1="50"
                x2={50 + 31 * Math.cos(angle)}
                y2={50 + 31 * Math.sin(angle)}
              />
            )
          })}
        </svg>
      )}
      <div className="graph-center">
        <AppIcon size={24} />
      </div>
      <div className="graph-summary" role="status">
        <span>
          {graph && !usingEvidenceFallback
            ? `Showing ${nodes.length} of ${graphDocuments.length} document${graphDocuments.length === 1 ? '' : 's'} · ${visibleEdges.length} link${visibleEdges.length === 1 ? '' : 's'}`
            : graphLoading
              ? 'Loading indexed graph…'
              : 'Retrieved evidence'}
          {graphError ? ` · ${graphError}` : ''}
        </span>
      </div>
      {graphError && onRetry && (
        <div className="graph-actions">
          <button type="button" className="link-button" onClick={onRetry}>
            Retry graph
          </button>
        </div>
      )}
      {graph?.next_cursor && onLoadMore && (
        <div className="graph-pagination">
          <button
            type="button"
            className="secondary-button"
            onClick={onLoadMore}
            disabled={graphAppendLoading}
          >
            {graphAppendLoading ? 'Loading more nodes…' : 'Load more nodes'}
          </button>
          <span>More nodes remain outside this bounded view.</span>
        </div>
      )}
      {!graph?.next_cursor && graphDocuments.length > visibleCount && (
        <div className="graph-pagination">
          <button
            type="button"
            className="secondary-button"
            onClick={() => setVisibleCount((count) => Math.min(count + 12, graphDocuments.length))}
          >
            Show more nodes
          </button>
          <span>Showing a bounded window for responsive rendering.</span>
        </div>
      )}
      {nodes.map((node, index) => (
        <button
          type="button"
          key={node.id}
          aria-label={`${node.document_id ? 'Open document' : 'Open evidence'}: ${node.label}`}
          title={node.document_id ? 'Open document' : 'Open retrieved evidence'}
          data-tooltip={node.document_id ? 'Open document' : 'Open retrieved evidence'}
          className="quick-tooltip"
          style={
            {
              '--angle': `${(index / Math.max(nodes.length, 1)) * Math.PI * 2}rad`,
            } as CSSProperties
          }
          onClick={() => {
            if (node.document_id) onSelectDocument(node.document_id)
            else onSelect(node.id)
          }}
        >
          <FileText size={17} />
          <span>{node.label}</span>
        </button>
      ))}
    </div>
  )
}

function TimelineView({
  evidence,
  onSelect,
}: {
  evidence: Evidence[]
  onSelect: (chunkId: string) => void
}) {
  return (
    <div className="timeline-view">
      <h1>Evidence timeline</h1>
      {evidence
        .map((item) => item)
        .sort((left, right) => right.updated_at.localeCompare(left.updated_at))
        .map((item) => (
          <button
            type="button"
            key={item.chunk_id}
            aria-label={`Timeline evidence: ${item.title}`}
            onClick={() => onSelect(item.chunk_id)}
          >
            <time>{new Date(item.updated_at).toLocaleDateString()}</time>
            <i />
            <div>
              <strong>{item.title}</strong>
              <span>{item.source}</span>
            </div>
          </button>
        ))}
    </div>
  )
}

function EmptyState({
  title,
  detail,
  action,
}: {
  title: string
  detail: string
  action?: () => void
}) {
  return (
    <div className="empty-state">
      <AppIcon size={28} />
      <h1>{title}</h1>
      <p>{detail}</p>
      {action && (
        <button type="button" onClick={action}>
          Try again
        </button>
      )}
    </div>
  )
}

function AppIcon({ size = 16 }: { size?: number }) {
  return <img src="/app-icon.svg" alt="" width={size} height={size} aria-hidden="true" />
}
