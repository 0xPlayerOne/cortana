import { FileText, History, Link2, Network, Sparkles, Star } from 'lucide-react'
import { type CSSProperties, useState } from 'react'

import type { AnswerResponse, Evidence } from '../types'

const tabs = [
  { id: 'answer', label: 'Answer', icon: Sparkles },
  { id: 'sources', label: 'Sources', icon: FileText },
  { id: 'graph', label: 'Graph', icon: Network },
  { id: 'timeline', label: 'Timeline', icon: History },
] as const

type Tab = (typeof tabs)[number]['id']

export function Workspace({
  query,
  answer,
  evidence,
  selected,
  loading,
  error,
  onSelect,
  onRetry,
}: {
  query: string
  answer: AnswerResponse | null
  evidence: Evidence[]
  selected: number
  loading: boolean
  error: string
  onSelect: (index: number) => void
  onRetry: () => void
}) {
  const [tab, setTab] = useState<Tab>('answer')
  const active = evidence[selected] ?? null

  return (
    <main className="workspace">
      <div className="workspace-tabs" role="tablist" aria-label="Result views">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? 'active' : ''}
            onClick={() => setTab(id)}
          >
            <Icon size={15} />
            {label}
            {id === 'sources' && <span className="count-pill">{evidence.length}</span>}
          </button>
        ))}
      </div>
      {error ? (
        <EmptyState
          title="Cortana could not reach the brain"
          detail={`${error}. Start the Rust API or add ?demo=1 to preview the workspace.`}
          action={onRetry}
        />
      ) : loading && evidence.length === 0 ? (
        <EmptyState
          title="Searching your brain"
          detail="Fusing semantic and exact-term evidence…"
        />
      ) : evidence.length === 0 ? (
        <EmptyState title="No evidence found" detail="Try a broader phrase or another source." />
      ) : tab === 'graph' ? (
        <GraphView evidence={evidence} onSelect={onSelect} />
      ) : tab === 'timeline' ? (
        <TimelineView evidence={evidence} onSelect={onSelect} />
      ) : tab === 'answer' ? (
        <AnswerView
          query={query}
          response={answer}
          evidence={evidence}
          onSelect={(index) => {
            onSelect(index)
            setTab('sources')
          }}
        />
      ) : (
        active && <DocumentView active={active} evidence={evidence} onSelect={onSelect} />
      )}
    </main>
  )
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
  return (
    <article className="document">
      <div className="breadcrumbs">
        <span>Brain</span> / <span>{active.source}</span> / <strong>{active.title}</strong>
        <div>
          <button aria-label="Favorite">
            <Star size={17} />
          </button>
          {active.uri && (
            <a href={active.uri} target="_blank" rel="noreferrer" aria-label="Open original source">
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
          {active.content.split(/\n{2,}/).map((paragraph, index) => (
            <p key={`${active.chunk_id}:${index}`}>{paragraph}</p>
          ))}
          <div className="evidence-footer">
            <h2>Related evidence</h2>
            {evidence.slice(0, 6).map((item, index) => (
              <button key={item.chunk_id} onClick={() => onSelect(index)}>
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
        <Sparkles size={15} /> Evidence brief
      </span>
      <h1>{query}</h1>
      {response && (
        <div className="answer-meta">
          <span>{response.mode}</span>
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
            {response.plan.queries.map((plannedQuery) => (
              <li key={plannedQuery}>{plannedQuery}</li>
            ))}
          </ol>
        </details>
      )}
      {response?.warnings.map((warning) => (
        <p className="answer-warning" key={warning}>
          {warning}
        </p>
      ))}
      <p className="lead">{evidence.length} cited passages</p>
      {evidence.slice(0, 4).map((item, index) => (
        <button className="answer-source" key={item.chunk_id} onClick={() => onSelect(index)}>
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
  evidence,
  onSelect,
}: {
  evidence: Evidence[]
  onSelect: (index: number) => void
}) {
  return (
    <div className="graph-view">
      <div className="graph-center">
        <Sparkles size={24} />
      </div>
      {evidence.slice(0, 8).map((item, index) => (
        <button
          key={item.chunk_id}
          style={
            {
              '--angle': `${(index / Math.min(evidence.length, 8)) * Math.PI * 2}rad`,
            } as CSSProperties
          }
          onClick={() => onSelect(index)}
        >
          <FileText size={17} />
          <span>{item.title}</span>
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
  onSelect: (index: number) => void
}) {
  return (
    <div className="timeline-view">
      <h1>Evidence timeline</h1>
      {evidence
        .map((item, index) => ({ item, index }))
        .sort((left, right) => right.item.updated_at.localeCompare(left.item.updated_at))
        .map(({ item, index }) => (
          <button key={item.chunk_id} onClick={() => onSelect(index)}>
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
      <Sparkles size={28} />
      <h1>{title}</h1>
      <p>{detail}</p>
      {action && <button onClick={action}>Try again</button>}
    </div>
  )
}
