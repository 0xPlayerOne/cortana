import {
  BookOpen,
  Database,
  FileText,
  FolderTree,
  History,
  Link2,
  Network,
  Search,
  Star,
} from 'lucide-react'
import { type ComponentProps, type CSSProperties, useEffect, useMemo, useState } from 'react'

import { isDesktopApp, openDesktopUrl } from '../api'
import { codeRevisionLabel } from '../codeEvidence'
import { isFavoriteDocument, toggleFavoriteDocument } from '../favoriteDocuments'
import { safeSourceLink } from '../sourceLinks'
import { Badge } from './shadcn/badge'
import { TooltipButton as Button } from './cortana/TooltipButton'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from './shadcn/empty'
import { Input } from './shadcn/input'
import { Spinner } from './shadcn/spinner'
import { Tabs, TabsList, TabsTrigger } from './shadcn/tabs'
import { Toggle } from './shadcn/toggle'
import type {
  AnswerResponse,
  BrainDocument,
  BrainGraphNode,
  BrainGraphPage,
  Evidence,
  ReflectResponse,
} from '../types'

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
type WorkspaceButtonProps = Omit<ComponentProps<typeof Button>, 'variant' | 'size'> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'ghost' | 'icon' | 'compact'
}

function WorkspaceButton({ variant = 'secondary', ...props }: WorkspaceButtonProps) {
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

function WorkspaceInteractive(props: ComponentProps<'button'>) {
  return <Button variant="ghost" {...props} />
}

function WorkspaceBadge(props: ComponentProps<'span'>) {
  return <Badge variant="secondary" {...props} />
}

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
  reflection,
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
  onFocusGraphNode,
  onRetry,
}: {
  query: string
  answer: AnswerResponse | null
  reflection: ReflectResponse | null
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
  onFocusGraphNode?: (node: BrainGraphNode) => void
  onRetry: () => void
}) {
  const active = evidence[selected] ?? null
  const hasResults = answer !== null || reflection !== null || evidence.length > 0
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
    if (answer || reflection) onTabChange('answer')
  }, [answer, reflection, onTabChange])
  useEffect(() => {
    // Keep an explicitly submitted search visible while retrieval is in
    // flight. The result tab is hidden from the tab strip until evidence
    // arrives, but redirecting it immediately would replace the loading
    // state with the idle document view.
    if (!loading && !hasResults && resultGatedTabs.has(tab)) onTabChange('document')
  }, [hasResults, loading, onTabChange, tab])

  const availableTabs = tabs.filter(({ id }) => id === 'document' || hasResults)
  return (
    <main
      id="main-content"
      className="workspace m7-knowledge-workspace"
      data-m7-knowledge-workspace=""
    >
      {tab !== 'graph' && (
        <Tabs
          className="shrink-0 border-b px-3 pt-2"
          value={tab}
          onValueChange={(value) => onTabChange(value as WorkspaceTab)}
        >
          <TabsList variant="line" aria-label="Result views">
            {availableTabs.map(({ id, label, icon: Icon }) => (
              <TabsTrigger key={id} value={id}>
                <Icon size={15} />
                {label}
                {id === 'document' && document && <Badge variant="secondary">1</Badge>}
                {id === 'sources' && evidence.length > 0 && (
                  <Badge variant="secondary">{evidence.length}</Badge>
                )}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      )}
      {documentLoading ? (
        <EmptyState title="Opening document" detail="Loading the canonical indexed content…" busy />
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
          onFocusGraphNode={onFocusGraphNode}
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
          busy
        />
      ) : evidence.length === 0 && !hasResults ? (
        <EmptyState title="No evidence found" detail="Try a broader phrase or another source." />
      ) : tab === 'timeline' ? (
        <TimelineView evidence={evidence} onSelect={selectEvidenceByChunkId} />
      ) : tab === 'answer' ? (
        reflection ? (
          <ReflectionView response={reflection} />
        ) : (
          <AnswerView
            query={query}
            response={answer}
            evidence={evidence}
            onSelect={(index) => {
              onSelect(index)
              onTabChange('sources')
            }}
          />
        )
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
          <WorkspaceInteractive
            type="button"
            aria-label={favorite ? 'Remove favorite' : 'Add favorite'}
            aria-pressed={favorite}
            title={favorite ? 'Remove favorite' : 'Add favorite'}
            className=""
            onClick={() => setFavorite(toggleFavoriteDocument(document.id))}
          >
            <Star size={17} fill={favorite ? 'currentColor' : 'none'} />
          </WorkspaceInteractive>
          {sourceHref && (
            <a
              href={sourceHref}
              target={isDesktopApp ? undefined : '_blank'}
              rel={isDesktopApp ? undefined : 'noreferrer'}
              aria-label="Open original source"
              title="Open original source"
              className=""
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
                    <WorkspaceInteractive
                      type="button"
                      key={related.id}
                      onClick={() => onSelectDocument(related.id)}
                    >
                      <Link2 size={14} />
                      <span>{related.title}</span>
                      <small>{related.source}</small>
                    </WorkspaceInteractive>
                  ))}
                </section>
              )}
              {document.surrounding.length > 0 && (
                <section>
                  <h2>Surrounding documents</h2>
                  {document.surrounding.map((related) => (
                    <WorkspaceInteractive
                      type="button"
                      key={related.id}
                      onClick={() => onSelectDocument(related.id)}
                    >
                      <FileText size={14} />
                      <span>{related.title}</span>
                      <small>{new Date(related.updated_at).toLocaleDateString()}</small>
                    </WorkspaceInteractive>
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
          <WorkspaceInteractive
            type="button"
            aria-label={favorite ? 'Remove favorite' : 'Add favorite'}
            aria-pressed={favorite}
            title={favorite ? 'Remove favorite' : 'Add favorite'}
            className=""
            onClick={() => setFavorite(toggleFavoriteDocument(active.chunk_id))}
          >
            <Star size={17} fill={favorite ? 'currentColor' : 'none'} />
          </WorkspaceInteractive>
          {sourceHref && (
            <a
              href={sourceHref}
              target={isDesktopApp ? undefined : '_blank'}
              rel={isDesktopApp ? undefined : 'noreferrer'}
              aria-label="Open original source"
              title="Open original source"
              className=""
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
              <WorkspaceInteractive
                type="button"
                key={item.chunk_id}
                onClick={() => onSelect(index)}
              >
                <span>{index + 1}</span> {item.title}
              </WorkspaceInteractive>
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

function ReflectionView({ response }: { response: ReflectResponse }) {
  const statements: Array<{ text: string; ids: string[]; evidenceIds?: string[] }> = [
    ...response.claims.map((item) => ({
      text: item.text,
      ids: item.supporting_memory_ids,
      evidenceIds: item.supporting_evidence_ids,
    })),
    ...response.patterns.map((item) => ({ text: item.statement, ids: item.supporting_memory_ids })),
    ...response.tensions.map((item) => ({ text: item.statement, ids: item.supporting_memory_ids })),
    ...response.recommendations.map((item) => ({
      text: item.statement,
      ids: item.supporting_memory_ids,
    })),
  ]
  return (
    <article className="answer-view" aria-label="Derived memory reflection">
      <span className="eyebrow">Derived reflection · not canonical memory</span>
      <h1>{response.objective}</h1>
      <p className="answer-warning">
        {response.status} · {response.provider.selected} · memory revision{' '}
        {response.memory_revision}
      </p>
      <div className="answer-copy">
        {statements.map((item, index) => (
          <section key={`${item.text}:${index}`} className="answer-memory-entry">
            <p>{item.text}</p>
            <small>
              Supporting memory: {item.ids.join(', ') || 'none'}
              {'evidenceIds' in item && item.evidenceIds?.length
                ? ` · evidence: ${item.evidenceIds.join(', ')}`
                : ''}
            </small>
          </section>
        ))}
      </div>
      {response.chronology.length > 0 && (
        <section className="answer-memory" aria-label="Reflection chronology">
          <h2>Chronology</h2>
          {response.chronology.map((item) => (
            <p key={`${item.memory_id}:${item.observed_at}`}>
              {item.observed_at} · {item.title} · supporting memory {item.memory_id}
            </p>
          ))}
        </section>
      )}
      {response.proposed_candidates.length > 0 && (
        <section className="answer-memory" aria-label="Review-only proposed memories">
          <h2>Proposed memories requiring approval</h2>
          {response.proposed_candidates.map((item) => (
            <article key={`${item.project}:${item.title}`} className="answer-memory-entry">
              <h3>{item.title}</h3>
              <p>{item.content}</p>
              <small>
                {item.content_type} · {item.retention_tier} · {item.scope} · support{' '}
                {item.supporting_memory_ids.join(', ')}
              </small>
            </article>
          ))}
        </section>
      )}
      <p className="lead">
        {response.metrics.memories_included} memories included · canonical memory unchanged
      </p>
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
          <WorkspaceBadge>{response.mode}</WorkspaceBadge>
          <WorkspaceBadge>
            {response.retrieval_degraded
              ? 'lexical fallback'
              : response.retrieval_mode || 'hybrid retrieval'}
          </WorkspaceBadge>
          <WorkspaceBadge>
            {response.cached ? 'cache hit' : `${response.latency_ms} ms`}
          </WorkspaceBadge>
          <WorkspaceBadge>
            {response.plan.queries.length}{' '}
            {response.plan.queries.length === 1 ? 'retrieval' : 'retrievals'}
          </WorkspaceBadge>
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
      {response?.memories && response.memories.length > 0 && (
        <section className="answer-memory" aria-label="Native agent memory">
          <p className="lead">{response.memories.length} native memory entries</p>
          {response.memories.slice(0, 4).map((memory) => (
            <article className="answer-memory-entry" key={memory.id}>
              <h2>{memory.title}</h2>
              <p>{memory.content}</p>
              <small>
                {memory.content_type ?? memory.kind} · {memory.retention_tier ?? 'durable'} ·{' '}
                {memory.scope ?? 'workspace'} · {memory.project} · confidence{' '}
                {memory.confidence.toFixed(2)}
                {memory.valid_until
                  ? ` · expires ${new Date(memory.valid_until).toLocaleDateString()}`
                  : ''}
              </small>
            </article>
          ))}
        </section>
      )}
      {evidence.slice(0, 4).map((item, index) => (
        <WorkspaceInteractive
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
        </WorkspaceInteractive>
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
  onFocusGraphNode,
}: {
  graph: BrainGraphPage | null
  graphLoading: boolean
  graphAppendLoading: boolean
  graphError: string
  onRetry?: () => void
  onLoadMore?: () => void
  evidence: Evidence[]
  onSelect?: (chunkId: string) => void
  onSelectDocument: (id: string) => void
  onFocusGraphNode?: (node: BrainGraphNode) => void
}) {
  const [visibleCount, setVisibleCount] = useState(12)
  const [filter, setFilter] = useState('')
  const [kindFilter, setKindFilter] = useState<BrainGraphNode['kind'] | 'all'>('all')
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const graphNodes = graph?.nodes ?? []
  const usingEvidenceFallback = graph === null
  const normalizedFilter = filter.trim().toLocaleLowerCase()
  const filteredNodes = useMemo(
    () =>
      normalizedFilter
        ? graphNodes.filter(
            (node) =>
              (kindFilter === 'all' || node.kind === kindFilter) &&
              [node.label, node.project, node.source ?? '', node.kind].some((value) =>
                value.toLocaleLowerCase().includes(normalizedFilter)
              )
          )
        : graphNodes.filter((node) => kindFilter === 'all' || node.kind === kindFilter),
    [graphNodes, kindFilter, normalizedFilter]
  )
  useEffect(() => {
    setVisibleCount(12)
    setSelectedNodeId(null)
  }, [graph?.nodes[0]?.id, kindFilter, normalizedFilter])
  const nodes = filteredNodes.length
    ? filteredNodes.slice(0, visibleCount)
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
      ? graph.edges.filter(
          (edge) => visibleNodeIds.has(edge.target) || visibleNodeIds.has(edge.source)
        )
      : []
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null
  const selectedEdges = selectedNode
    ? visibleEdges.filter(
        (edge) => edge.source === selectedNode.id || edge.target === selectedNode.id
      )
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
  if (!graphLoading && nodes.length === 0 && normalizedFilter) {
    return (
      <div className="graph-empty-filter">
        <Search size={24} aria-hidden="true" />
        <h1>No matching graph nodes</h1>
        <p>Try a workspace, source, or document name.</p>
        <WorkspaceButton variant="secondary" onClick={() => setFilter('')}>
          Clear filter
        </WorkspaceButton>
      </div>
    )
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
      <div className="graph-toolbar" role="search">
        <Search size={14} aria-hidden="true" />
        <Input
          type="search"
          aria-label="Filter graph nodes"
          placeholder="Filter nodes…"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        {filter && (
          <WorkspaceButton
            variant="ghost"
            type="button"
            className="link-button"
            onClick={() => setFilter('')}
          >
            Clear
          </WorkspaceButton>
        )}
      </div>
      {graph && !usingEvidenceFallback && (
        <div className="graph-kind-filter" role="group" aria-label="Filter graph node types">
          {(['all', 'workspace', 'source', 'document'] as const).map((kind) => (
            <Toggle
              key={kind}
              size="sm"
              variant="outline"
              pressed={kindFilter === kind}
              onPressedChange={(pressed) => pressed && setKindFilter(kind)}
            >
              {kind === 'all'
                ? 'All'
                : kind === 'workspace'
                  ? 'Workspaces'
                  : kind === 'source'
                    ? 'Sources'
                    : 'Documents'}
            </Toggle>
          ))}
        </div>
      )}
      <div className="graph-summary" role="status">
        <span>
          {graph && !usingEvidenceFallback
            ? graphNodes.every((node) => node.kind === 'document')
              ? `Showing ${nodes.length} of ${filteredNodes.length}${normalizedFilter ? ` matching ${graphNodes.length}` : ''} document${filteredNodes.length === 1 ? '' : 's'} · ${visibleEdges.length} link${visibleEdges.length === 1 ? '' : 's'}`
              : `Showing ${nodes.length} of ${filteredNodes.length}${normalizedFilter ? ` matching ${graphNodes.length}` : ''} node${filteredNodes.length === 1 ? '' : 's'} · ${visibleEdges.length} link${visibleEdges.length === 1 ? '' : 's'}`
            : graphLoading
              ? 'Loading indexed graph…'
              : 'Retrieved evidence'}
          {graphError ? ` · ${graphError}` : ''}
        </span>
      </div>
      {graphError && onRetry && (
        <div className="graph-actions">
          <WorkspaceButton variant="ghost" type="button" className="link-button" onClick={onRetry}>
            Retry graph
          </WorkspaceButton>
        </div>
      )}
      {graph?.next_cursor && onLoadMore && (
        <div className="graph-pagination">
          <WorkspaceButton variant="secondary" onClick={onLoadMore} disabled={graphAppendLoading}>
            {graphAppendLoading ? 'Loading more nodes…' : 'Load more nodes'}
          </WorkspaceButton>
          <span>More nodes remain outside this bounded view.</span>
        </div>
      )}
      {!graph?.next_cursor && filteredNodes.length > visibleCount && (
        <div className="graph-pagination">
          <WorkspaceButton
            variant="secondary"
            onClick={() => setVisibleCount((count) => Math.min(count + 12, filteredNodes.length))}
          >
            Show more nodes
          </WorkspaceButton>
          <span>Showing a bounded window for responsive rendering.</span>
        </div>
      )}
      {nodes.map((node, index) => (
        <button
          type="button"
          key={node.id}
          aria-label={`${node.document_id ? 'Open document' : node.kind === 'workspace' ? 'Focus workspace' : node.kind === 'source' ? 'Focus source' : 'Open evidence'}: ${node.label}`}
          title={
            node.document_id
              ? 'Open document'
              : node.kind === 'workspace'
                ? 'Focus workspace'
                : node.kind === 'source'
                  ? 'Focus source'
                  : 'Open retrieved evidence'
          }
          className={` graph-node graph-node--${node.kind}`}
          data-kind={node.kind}
          style={
            {
              '--angle': `${(index / Math.max(nodes.length, 1)) * Math.PI * 2}rad`,
            } as CSSProperties
          }
          onClick={() => {
            setSelectedNodeId(node.id)
            if (node.document_id) return
            if (node.kind === 'workspace' || node.kind === 'source') {
              onFocusGraphNode?.(node)
              return
            }
            // The API-backed graph uses workspace/source nodes for navigation,
            // while the offline evidence fallback uses chunk IDs. Preserve the
            // fallback's evidence selection without passing synthetic graph IDs
            // into the workspace/source focus handler.
            onSelect?.(node.id)
          }}
        >
          {node.kind === 'workspace' ? (
            <FolderTree size={17} aria-hidden="true" />
          ) : node.kind === 'source' ? (
            <Database size={17} aria-hidden="true" />
          ) : (
            <FileText size={17} aria-hidden="true" />
          )}
          <span>{node.label}</span>
        </button>
      ))}
      {selectedNode && (
        <aside className="graph-selection" aria-label="Selected graph node">
          <strong>{selectedNode.label}</strong>
          <span>
            {selectedNode.kind === 'workspace'
              ? 'Workspace'
              : selectedNode.kind === 'source'
                ? `Source in ${selectedNode.project || 'Unscoped'}`
                : `${selectedNode.project || 'Unscoped'} · ${selectedNode.source || 'Unknown source'}`}
          </span>
          <small>
            {selectedEdges.length} related link{selectedEdges.length === 1 ? '' : 's'}
          </small>
          {selectedEdges.length > 0 && (
            <ul>
              {selectedEdges.map((edge) => (
                <li key={`${edge.source}:${edge.target}:${edge.kind}`}>
                  <span>
                    {edge.kind === 'contains' ? 'Contained by its workspace or source' : edge.kind}
                  </span>
                  {edge.origin && (
                    <small>
                      {edge.origin === 'inferred'
                        ? `Inferred relationship${edge.confidence == null ? '' : ` · ${Math.round(edge.confidence * 100)}% confidence`}`
                        : `${edge.origin[0].toUpperCase()}${edge.origin.slice(1)} relationship`}
                      {edge.support
                        ? ` · ${edge.support.record_ids.length} supporting record${edge.support.record_ids.length === 1 ? '' : 's'}`
                        : ''}
                      {edge.citation_authority ? ' · citation-capable' : ' · not citation evidence'}
                    </small>
                  )}
                </li>
              ))}
            </ul>
          )}
          {selectedNode.document_id && (
            <WorkspaceButton
              variant="secondary"
              onClick={() => onSelectDocument(selectedNode.document_id!)}
            >
              Open document
            </WorkspaceButton>
          )}
        </aside>
      )}
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
          <WorkspaceInteractive
            type="button"
            key={item.chunk_id}
            aria-label={`Timeline evidence: ${item.title}`}
            onClick={() => onSelect(item.chunk_id)}
          >
            <time>{new Date(item.updated_at).toLocaleDateString()}</time>
            <i />
            <div>
              <strong>{item.title}</strong>
              <span>{codeRevisionLabel(item) ?? item.source}</span>
            </div>
          </WorkspaceInteractive>
        ))}
    </div>
  )
}

function EmptyState({
  title,
  detail,
  action,
  busy = false,
}: {
  title: string
  detail: string
  action?: () => void
  busy?: boolean
}) {
  return (
    <Empty className="m-4 min-h-64 border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          {busy ? <Spinner aria-label="Loading" /> : <Search aria-hidden="true" />}
        </EmptyMedia>
        <EmptyTitle role="heading" aria-level={1}>
          {title}
        </EmptyTitle>
        <EmptyDescription>{detail}</EmptyDescription>
      </EmptyHeader>
      {action && (
        <EmptyContent>
          <Button onClick={action}>Try again</Button>
        </EmptyContent>
      )}
    </Empty>
  )
}

function AppIcon({ size = 16 }: { size?: number }) {
  return <img src="/app-icon.svg" alt="" width={size} height={size} aria-hidden="true" />
}
