import { Pause, Play, RefreshCw, Search, ShieldCheck } from 'lucide-react'
import {
  createContext,
  type ComponentProps,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'

import {
  actOnMemoryCandidate,
  classifyMemoryCandidate,
  getMemoryConsolidationState,
  listCanonicalMemories,
  listDerivedMemories,
  listMemoryCandidates,
  setMemoryConsolidationPaused,
  type MemoryCandidateAction,
} from '../api'
import type {
  AgentMemory,
  DerivedMemoryResponse,
  MemoryCandidate,
  MemoryCandidateActionResult,
  MemoryCandidateClassification,
  MemoryReviewPolicy,
} from '../types'
import { virtualRange } from '../virtualization'
import { useM7SurfacePrimitives } from './m7/M7SurfacePrimitives'
import { Button, type ButtonProps } from './ui/Button'

type QueueView =
  | 'all'
  | 'pending'
  | 'approved'
  | 'auto-retained'
  | 'rejected'
  | 'expired'
  | 'failed'
  | 'dead-letter'

const MemoryRendererContext = createContext<'legacy' | 'shadcn'>('legacy')

function MemoryButton({ variant = 'secondary', ...props }: ButtonProps) {
  const renderer = useContext(MemoryRendererContext)
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

function MemoryInput(props: ComponentProps<'input'>) {
  const renderer = useContext(MemoryRendererContext)
  const ShadcnInput = useM7SurfacePrimitives()?.Input
  return renderer === 'shadcn' && ShadcnInput ? <ShadcnInput {...props} /> : <input {...props} />
}

function MemoryTextarea(props: ComponentProps<'textarea'>) {
  const renderer = useContext(MemoryRendererContext)
  const ShadcnTextarea = useM7SurfacePrimitives()?.Textarea
  return renderer === 'shadcn' && ShadcnTextarea ? (
    <ShadcnTextarea {...props} />
  ) : (
    <textarea {...props} />
  )
}

function MemoryCard(props: ComponentProps<'div'>) {
  const renderer = useContext(MemoryRendererContext)
  const ShadcnCard = useM7SurfacePrimitives()?.Card
  return renderer === 'shadcn' && ShadcnCard ? (
    <ShadcnCard size="sm" {...props} />
  ) : (
    <div {...props} />
  )
}

function MemoryBadge(props: ComponentProps<'span'>) {
  const renderer = useContext(MemoryRendererContext)
  const ShadcnBadge = useM7SurfacePrimitives()?.Badge
  return renderer === 'shadcn' && ShadcnBadge ? (
    <ShadcnBadge variant="secondary" {...props} />
  ) : (
    <span {...props} />
  )
}

export type MemoryReviewClient = {
  listCandidates: (project?: string, query?: string, status?: string) => Promise<MemoryCandidate[]>
  classifyCandidate: (id: string) => Promise<MemoryCandidateClassification>
  listDerived: (project?: string) => Promise<DerivedMemoryResponse>
  listCanonical: (project?: string) => Promise<AgentMemory[]>
  act: (
    id: string,
    action: MemoryCandidateAction,
    policy: MemoryReviewPolicy,
    edit?: { title: string; content: string }
  ) => Promise<MemoryCandidateActionResult>
  setConsolidationPaused: (paused: boolean) => Promise<void>
  getConsolidationState: () => Promise<{ paused: boolean; canControl: boolean }>
}

function MemoryPolicy({
  policy,
  onChange,
}: {
  policy: MemoryReviewPolicy
  onChange: (policy: MemoryReviewPolicy) => void
}) {
  const patch = (next: Partial<MemoryReviewPolicy>) => onChange({ ...policy, ...next })
  return (
    <div className="memory-policy" aria-label="Memory retention policy">
      <label>
        Working ceiling (days)
        <MemoryInput
          type="number"
          min={1}
          max={7}
          value={policy.maxWorkingDays}
          onChange={(event) => patch({ maxWorkingDays: Number(event.target.value) })}
        />
      </label>
      <label>
        Durable ceiling (days)
        <MemoryInput
          type="number"
          min={1}
          max={3650}
          value={policy.maxDurableDays}
          onChange={(event) => patch({ maxDurableDays: Number(event.target.value) })}
        />
      </label>
      <label>
        Candidate expiry (days)
        <MemoryInput
          type="number"
          min={1}
          max={7}
          value={policy.candidateExpiryDays}
          onChange={(event) => patch({ candidateExpiryDays: Number(event.target.value) })}
        />
      </label>
      <p>
        Candidate processing is manual. Automatic retention and recurring processing remain
        disabled.
      </p>
    </div>
  )
}

type CandidateRange = ReturnType<typeof virtualRange>

function CandidateQueue({
  filtered,
  range,
  selectedId,
  selectedIds,
  loading,
  busy,
  onScroll,
  onSelect,
  onCheck,
  onBulk,
}: {
  filtered: MemoryCandidate[]
  range: CandidateRange
  selectedId: string
  selectedIds: Set<string>
  loading: boolean
  busy: boolean
  onScroll: (top: number) => void
  onSelect: (id: string) => void
  onCheck: (ids: Set<string>) => void
  onBulk: (action: MemoryCandidateAction, ids: string[]) => void
}) {
  const renderer = useContext(MemoryRendererContext)
  const ShadcnCheckbox = useM7SurfacePrimitives()?.Checkbox
  const updateSelection = (candidate: MemoryCandidate, checked: boolean) => {
    const next = new Set(selectedIds)
    if (checked && next.size < MAX_BULK_ACTIONS) next.add(candidate.id)
    else next.delete(candidate.id)
    onCheck(next)
  }

  return (
    <div>
      <div
        className="memory-candidate-list"
        role="list"
        aria-label="Memory candidate queue"
        aria-busy={loading}
        onScroll={(event) => onScroll(event.currentTarget.scrollTop)}
      >
        <div style={{ height: range.totalHeight }}>
          <div style={{ transform: `translateY(${range.offsetTop}px)` }}>
            {filtered.slice(range.start, range.end).map((candidate) => (
              <MemoryCard
                key={candidate.id}
                role="listitem"
                className={`memory-candidate-row ${selectedId === candidate.id ? 'selected' : ''}`}
              >
                {renderer === 'shadcn' && ShadcnCheckbox ? (
                  <ShadcnCheckbox
                    aria-label={`Select ${candidate.title}`}
                    checked={selectedIds.has(candidate.id)}
                    onCheckedChange={(checked) => updateSelection(candidate, checked)}
                  />
                ) : (
                  <input
                    type="checkbox"
                    aria-label={`Select ${candidate.title}`}
                    checked={selectedIds.has(candidate.id)}
                    onChange={(event) => updateSelection(candidate, event.target.checked)}
                  />
                )}
                <MemoryButton
                  variant="ghost"
                  type="button"
                  aria-current={selectedId === candidate.id}
                  aria-label={`${candidate.title}, ${queueStatus(candidate)}`}
                  onClick={() => onSelect(candidate.id)}
                >
                  <strong>{candidate.title}</strong>
                  <span>{candidate.content}</span>
                </MemoryButton>
                <MemoryBadge className={`memory-status status-${queueStatus(candidate)}`}>
                  {queueStatus(candidate)}
                </MemoryBadge>
              </MemoryCard>
            ))}
          </div>
        </div>
        {!loading && filtered.length === 0 && (
          <p className="empty-state">No candidates match this view.</p>
        )}
      </div>
      {selectedIds.size > 0 && (
        <div className="memory-bulk-actions" aria-label="Bulk-safe candidate actions">
          <span>
            {selectedIds.size}/{MAX_BULK_ACTIONS} selected
          </span>
          <MemoryButton
            type="button"
            variant="secondary"
            disabled={busy}
            onClick={() => onBulk('reject', [...selectedIds])}
          >
            Reject selected
          </MemoryButton>
          <MemoryButton
            type="button"
            variant="secondary"
            disabled={busy}
            onClick={() => onBulk('redact', [...selectedIds])}
          >
            Redact selected
          </MemoryButton>
        </div>
      )}
    </div>
  )
}

const defaultClient: MemoryReviewClient = {
  listCandidates: listMemoryCandidates,
  classifyCandidate: classifyMemoryCandidate,
  listDerived: listDerivedMemories,
  listCanonical: listCanonicalMemories,
  act: actOnMemoryCandidate,
  setConsolidationPaused: setMemoryConsolidationPaused,
  getConsolidationState: getMemoryConsolidationState,
}

const QUEUE_VIEWS: QueueView[] = [
  'all',
  'pending',
  'approved',
  'auto-retained',
  'rejected',
  'expired',
  'failed',
  'dead-letter',
]
const ROW_HEIGHT = 72
const MAX_BULK_ACTIONS = 25

const DEFAULT_POLICY: MemoryReviewPolicy = {
  autoCommit: false,
  maxWorkingDays: 7,
  maxDurableDays: 365,
  maxActive: 10_000,
  candidateExpiryDays: 7,
  schedule: 'manual',
}

export function MemoryReview({
  project,
  maxActive = DEFAULT_POLICY.maxActive,
  client = defaultClient,
  renderer = 'legacy',
}: {
  project?: string
  maxActive?: number
  client?: MemoryReviewClient
  renderer?: 'legacy' | 'shadcn'
}) {
  const primitives = useM7SurfacePrimitives()
  const ShadcnAlert = primitives?.Alert
  const ShadcnAlertDescription = primitives?.AlertDescription
  const ShadcnSpinner = primitives?.Spinner
  const ShadcnToggle = primitives?.Toggle
  const [candidates, setCandidates] = useState<MemoryCandidate[]>([])
  const [canonical, setCanonical] = useState<AgentMemory[]>([])
  const [derived, setDerived] = useState<DerivedMemoryResponse | null>(null)
  const [classification, setClassification] = useState<MemoryCandidateClassification | null>(null)
  const [selectedId, setSelectedId] = useState('')
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [view, setView] = useState<QueueView>('all')
  const [query, setQuery] = useState('')
  const [scrollTop, setScrollTop] = useState(0)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [paused, setPaused] = useState(false)
  const [canControl, setCanControl] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [policy, setPolicy] = useState({ ...DEFAULT_POLICY, maxActive })
  const [editing, setEditing] = useState(false)
  const [editTitle, setEditTitle] = useState('')
  const [editContent, setEditContent] = useState('')
  const refreshVersion = useRef(0)

  async function refresh() {
    const version = ++refreshVersion.current
    setLoading(true)
    setError('')
    try {
      const [nextCandidates, nextCanonical, nextDerived, consolidationState] = await Promise.all([
        client.listCandidates(
          project,
          query.trim() || undefined,
          view === 'all' ? undefined : view
        ),
        client.listCanonical(project),
        client.listDerived(project),
        client.getConsolidationState(),
      ])
      if (version !== refreshVersion.current) return
      setCandidates(nextCandidates)
      setCanonical(nextCanonical.slice(0, 100))
      setDerived(nextDerived)
      setPaused(consolidationState.paused)
      setCanControl(consolidationState.canControl)
      setSelectedId((current) =>
        nextCandidates.some((candidate) => candidate.id === current)
          ? current
          : (nextCandidates[0]?.id ?? '')
      )
    } catch (caught) {
      if (version !== refreshVersion.current) return
      setError(caught instanceof Error ? caught.message : 'Memory review failed')
    } finally {
      if (version === refreshVersion.current) setLoading(false)
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => void refresh(), 200)
    return () => window.clearTimeout(timer)
  }, [project, query, view])

  useEffect(() => {
    setPolicy((current) => ({ ...current, maxActive }))
  }, [maxActive])

  useEffect(() => {
    const selected = candidates.find((candidate) => candidate.id === selectedId)
    if (!selected) {
      setClassification(null)
      return
    }
    setEditTitle(selected.title)
    setEditContent(selected.content)
    setClassification(null)
    if (selected.status === 'pending') {
      client
        .classifyCandidate(selected.id)
        .then(setClassification)
        .catch(() => setClassification(null))
    }
  }, [candidates, client, selectedId])

  const filtered = useMemo(() => {
    return candidates
  }, [candidates])
  const range = virtualRange(filtered.length, scrollTop, 360, ROW_HEIGHT)
  const selected = candidates.find((candidate) => candidate.id === selectedId)

  async function runAction(
    action: MemoryCandidateAction,
    ids = selected ? [selected.id] : [],
    edit?: { title: string; content: string }
  ) {
    const boundedIds = ids.slice(0, MAX_BULK_ACTIONS)
    if (!boundedIds.length) return
    const canonicalWrite = ['approve', 'edit-approve', 'working', 'supersede', 'retry'].includes(
      action
    )
    if (
      canonicalWrite &&
      !window.confirm(
        `Confirm ${action.replace('-', ' ')} for ${boundedIds.length} candidate${boundedIds.length === 1 ? '' : 's'}? Canonical memory may change only if backend policy approves.`
      )
    ) {
      return
    }
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const results: MemoryCandidateActionResult[] = []
      for (const id of boundedIds) results.push(await client.act(id, action, policy, edit))
      const reviews = results.filter(
        (result) => result.status === 'review' || result.decision?.decision === 'review'
      ).length
      const writes = results.filter((result) => Boolean(result.memory_id)).length
      if (reviews) {
        setNotice(
          `${reviews} candidate(s) remain in review; no canonical memory changed for those records.`
        )
      } else if (writes) {
        setNotice(`Canonical memory updated for ${writes} candidate(s).`)
      } else {
        setNotice(`${action.replace('-', ' ')} recorded for ${boundedIds.length} candidate(s).`)
      }
      setEditing(false)
      setSelectedIds(new Set())
      await refresh()
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : `Memory candidate ${action} failed`)
    } finally {
      setBusy(false)
    }
  }

  async function togglePause() {
    setBusy(true)
    setError('')
    try {
      await client.setConsolidationPaused(!paused)
      setPaused(!paused)
      setNotice(`Consolidation ${paused ? 'resumed' : 'paused'}.`)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Consolidation control failed')
    } finally {
      setBusy(false)
    }
  }

  return (
    <MemoryRendererContext.Provider value={renderer}>
      <section
        className={`memory-review ${renderer === 'shadcn' ? 'm7-memory-review' : ''}`}
        aria-labelledby="memory-review-title"
        data-m7-memory-review={renderer === 'shadcn' ? '' : undefined}
      >
        <header className="memory-review-header">
          <div>
            <span className="eyebrow">Review before retention</span>
            <h3 id="memory-review-title">Memory control center</h3>
            <p>Inspect candidates, canonical recall, and derived reasoning as separate layers.</p>
          </div>
          <div className="memory-review-header-actions">
            <MemoryButton
              type="button"
              variant="secondary"
              disabled={busy || !canControl}
              title={canControl ? undefined : 'Owner authorization is required'}
              onClick={() => void togglePause()}
            >
              {paused ? <Play size={14} /> : <Pause size={14} />}
              {paused ? 'Resume consolidation' : 'Pause consolidation'}
            </MemoryButton>
            <MemoryButton
              type="button"
              variant="secondary"
              disabled={loading}
              onClick={() => void refresh()}
            >
              {renderer === 'shadcn' && loading && ShadcnSpinner ? (
                <ShadcnSpinner />
              ) : (
                <RefreshCw size={14} />
              )}{' '}
              Refresh
            </MemoryButton>
          </div>
        </header>

        <MemoryPolicy policy={policy} onChange={setPolicy} />
        <div className="memory-review-filters">
          <label className="memory-review-search">
            <Search size={14} aria-hidden="true" />
            <span className="sr-only">Search memory candidates</span>
            <MemoryInput
              type="search"
              aria-label="Search memory candidates"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search candidate content, project, or source"
            />
          </label>
          <div className="memory-status-tabs" role="group" aria-label="Candidate status views">
            {QUEUE_VIEWS.map((status) =>
              renderer === 'shadcn' && ShadcnToggle ? (
                <ShadcnToggle
                  key={status}
                  size="sm"
                  pressed={view === status}
                  onPressedChange={(pressed) => pressed && setView(status)}
                >
                  {status.replace('-', ' ')}
                </ShadcnToggle>
              ) : (
                <MemoryButton
                  key={status}
                  type="button"
                  variant={view === status ? 'primary' : 'ghost'}
                  aria-pressed={view === status}
                  onClick={() => setView(status)}
                >
                  {status.replace('-', ' ')}
                </MemoryButton>
              )
            )}
          </div>
        </div>

        {(error || notice) &&
          (renderer === 'shadcn' && error && ShadcnAlert && ShadcnAlertDescription ? (
            <ShadcnAlert variant="destructive">
              <ShadcnAlertDescription>{error}</ShadcnAlertDescription>
            </ShadcnAlert>
          ) : (
            <div
              role={error ? 'alert' : 'status'}
              className={`memory-review-message ${error ? 'error' : ''}`}
            >
              {error || notice}
            </div>
          ))}

        <div className="memory-review-layout">
          <CandidateQueue
            filtered={filtered}
            range={range}
            selectedId={selectedId}
            selectedIds={selectedIds}
            loading={loading}
            busy={busy}
            onScroll={setScrollTop}
            onSelect={setSelectedId}
            onCheck={setSelectedIds}
            onBulk={(action, ids) => void runAction(action, ids)}
          />
          <CandidateDetail
            selected={selected}
            classification={classification}
            busy={busy}
            editing={editing}
            editTitle={editTitle}
            editContent={editContent}
            onEditing={setEditing}
            onTitle={setEditTitle}
            onContent={setEditContent}
            onAction={(action, edit) => void runAction(action, undefined, edit)}
          />
        </div>

        <MemoryLayers canonical={canonical} derived={derived} />
      </section>
    </MemoryRendererContext.Provider>
  )
}

function CandidateDetail({
  selected,
  classification,
  busy,
  editing,
  editTitle,
  editContent,
  onEditing,
  onTitle,
  onContent,
  onAction,
}: {
  selected?: MemoryCandidate
  classification: MemoryCandidateClassification | null
  busy: boolean
  editing: boolean
  editTitle: string
  editContent: string
  onEditing: (editing: boolean) => void
  onTitle: (title: string) => void
  onContent: (content: string) => void
  onAction: (action: MemoryCandidateAction, edit?: { title: string; content: string }) => void
}) {
  if (!selected) return <article className="memory-candidate-detail">Select a candidate.</article>
  return (
    <article className="memory-candidate-detail" aria-live="polite">
      <span className="eyebrow">Candidate · not canonical</span>
      <h4>{selected.title}</h4>
      {editing ? (
        <div className="memory-edit-fields">
          <label>
            Proposed title
            <MemoryInput value={editTitle} onChange={(event) => onTitle(event.target.value)} />
          </label>
          <label>
            Proposed content
            <MemoryTextarea
              value={editContent}
              onChange={(event) => onContent(event.target.value)}
            />
          </label>
        </div>
      ) : (
        <p>{selected.content}</p>
      )}
      <CandidateMetadata selected={selected} classification={classification} />
      {selected.status === 'pending' ? (
        <CandidateActions
          busy={busy}
          retryable={
            selected.consolidation?.status === 'dead-letter' ||
            selected.consolidation?.status === 'retry'
          }
          editing={editing}
          editTitle={editTitle}
          editContent={editContent}
          onEditing={onEditing}
          onAction={onAction}
        />
      ) : (
        <p className="memory-explanation">
          This candidate is terminal. Its stored outcome is shown above; no new classification or
          action was run.
        </p>
      )}
    </article>
  )
}

function CandidateMetadata({
  selected,
  classification,
}: {
  selected: MemoryCandidate
  classification: MemoryCandidateClassification | null
}) {
  return (
    <>
      <div className="memory-metadata">
        <div>
          <span>Content type</span>
          <strong>{selected.content_type}</strong>
        </div>
        <div>
          <span>Retention</span>
          <strong>{selected.retention_tier}</strong>
        </div>
        <div>
          <span>Scope</span>
          <strong>{selected.scope}</strong>
        </div>
        <div>
          <span>Confidence</span>
          <strong>{Math.round(selected.confidence * 100)}%</strong>
        </div>
        <div>
          <span>Sensitivity</span>
          <strong>{selected.sensitivity}</strong>
        </div>
        <div>
          <span>Expires</span>
          <strong>{selected.expires_at}</strong>
        </div>
        <div>
          <span>Classification</span>
          <strong>
            {selected.consolidation?.classification ??
              classification?.classification ??
              'Not evaluated'}
          </strong>
        </div>
        <div>
          <span>Policy version</span>
          <strong>{selected.consolidation?.policy_version ?? 'Not evaluated'}</strong>
        </div>
      </div>
      {selected.consolidation && (
        <div className="memory-metadata">
          <div>
            <span>Decision</span>
            <strong>{selected.consolidation.decision}</strong>
          </div>
          <div>
            <span>Job status</span>
            <strong>{selected.consolidation.status}</strong>
          </div>
          <div>
            <span>Attempts</span>
            <strong>{selected.consolidation.attempts}</strong>
          </div>
          <div>
            <span>Canonical memory</span>
            <strong>{selected.consolidation.memory_id ?? 'None'}</strong>
          </div>
          <div>
            <span>Last error</span>
            <strong>{selected.consolidation.last_error ?? 'None'}</strong>
          </div>
          <div>
            <span>Evaluated</span>
            <strong>{selected.consolidation.updated_at}</strong>
          </div>
        </div>
      )}
      {classification && <p className="memory-explanation">{classification.explanation}</p>}
      {!classification && selected.consolidation && (
        <p className="memory-explanation">
          {selected.consolidation.explanation ??
            `Stored policy decision ${selected.consolidation.decision} ended as ${selected.consolidation.status}`}
          {selected.consolidation.memory_id
            ? ` and created canonical memory ${selected.consolidation.memory_id}`
            : ' without creating canonical memory'}
          {selected.consolidation.reason_code
            ? ` (reason: ${selected.consolidation.reason_code})`
            : ''}
          .
        </p>
      )}
      <details>
        <summary>Provenance and support</summary>
        <pre>{JSON.stringify(selected.provenance, null, 2)}</pre>
        <p>
          Supporting memories:{' '}
          {(
            classification?.supporting_memory_ids ??
            selected.consolidation?.supporting_memory_ids ??
            []
          ).join(', ') || 'None'}
        </p>
      </details>
    </>
  )
}

function CandidateActions({
  busy,
  retryable,
  editing,
  editTitle,
  editContent,
  onEditing,
  onAction,
}: {
  busy: boolean
  retryable: boolean
  editing: boolean
  editTitle: string
  editContent: string
  onEditing: (editing: boolean) => void
  onAction: (action: MemoryCandidateAction, edit?: { title: string; content: string }) => void
}) {
  return (
    <div className="memory-candidate-actions">
      <MemoryButton type="button" disabled={busy} onClick={() => onAction('approve')}>
        <ShieldCheck size={14} /> Approve canonical memory
      </MemoryButton>
      {editing ? (
        <MemoryButton
          type="button"
          disabled={busy || !editTitle.trim() || !editContent.trim()}
          onClick={() => onAction('edit-approve', { title: editTitle, content: editContent })}
        >
          Confirm edit and approve
        </MemoryButton>
      ) : (
        <MemoryButton type="button" variant="secondary" onClick={() => onEditing(true)}>
          Edit and approve
        </MemoryButton>
      )}
      <MemoryButton
        type="button"
        variant="secondary"
        disabled={busy}
        onClick={() => onAction('working')}
      >
        Keep working
      </MemoryButton>
      <MemoryButton
        type="button"
        variant="secondary"
        disabled={busy}
        onClick={() => onAction('supersede')}
      >
        Review and supersede
      </MemoryButton>
      {retryable && (
        <MemoryButton
          type="button"
          variant="secondary"
          disabled={busy}
          onClick={() => onAction('retry')}
        >
          Retry
        </MemoryButton>
      )}
      <MemoryButton
        type="button"
        variant="secondary"
        disabled={busy}
        onClick={() => onAction('reject')}
      >
        Reject
      </MemoryButton>
      <MemoryButton
        type="button"
        variant="secondary"
        disabled={busy}
        onClick={() => onAction('redact')}
      >
        Redact
      </MemoryButton>
    </div>
  )
}

function MemoryLayers({
  canonical,
  derived,
}: {
  canonical: AgentMemory[]
  derived: DerivedMemoryResponse | null
}) {
  return (
    <div className="memory-layer-grid">
      <section aria-labelledby="canonical-memory-title">
        <span className="eyebrow">Recall</span>
        <h4 id="canonical-memory-title">Canonical memory</h4>
        <p>Durable records eligible for recall and evidence-backed answers.</p>
        <ul>
          {canonical.slice(0, 20).map((memory) => (
            <li key={memory.id}>
              <strong>{memory.title}</strong>
              <span>{memory.content}</span>
              <span>
                {memory.status ?? 'active'}
                {memory.supersedes_id ? ` · supersedes ${memory.supersedes_id}` : ''}
                {memory.source ? ` · from ${memory.source}` : ''}
              </span>
            </li>
          ))}
        </ul>
      </section>
      <section aria-labelledby="derived-memory-title">
        <span className="eyebrow">Reflect</span>
        <h4 id="derived-memory-title">Derived · not canonical</h4>
        <p>Recomputed interpretations are never source evidence or citation authority.</p>
        <ul>
          {derived?.representations.slice(0, 20).map((item) => (
            <li key={item.id}>
              <strong>
                {item.kind}: {item.statement}
              </strong>
              <span>Supports: {item.supporting_memory_ids.join(', ') || 'None'}</span>
              <span>Opposes: {item.contradicting_memory_ids.join(', ') || 'None'}</span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  )
}

function queueStatus(candidate: MemoryCandidate): QueueView {
  const job = candidate.consolidation
  if (candidate.status === 'expired') return 'expired'
  if (candidate.status === 'accepted' && job?.decision === 'auto-retain') return 'auto-retained'
  if (candidate.status === 'accepted') return 'approved'
  if (['cancelled', 'rejected', 'redacted'].includes(candidate.status)) return 'rejected'
  if (job?.status === 'dead-letter') return 'dead-letter'
  if (job?.status === 'retry') return 'failed'
  return 'pending'
}
