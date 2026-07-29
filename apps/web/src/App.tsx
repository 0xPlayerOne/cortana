import { FileText, LoaderCircle, Search } from 'lucide-react'
import { type FormEvent, useEffect, useMemo, useState } from 'react'

import { getStatus, isDemoMode, searchEvidence } from './api'
import { ContextPanel } from './components/ContextPanel'
import { Navigation, TitleActions } from './components/Navigation'
import { SourcePanel } from './components/SourcePanel'
import { Workspace } from './components/Workspace'
import { buildAgentContext } from './context'
import type { BrainStatus, Evidence } from './types'

export function App() {
  const [query, setQuery] = useState('How do releases work?')
  const [activeQuery, setActiveQuery] = useState(query)
  const [status, setStatus] = useState<BrainStatus | null>(null)
  const [evidence, setEvidence] = useState<Evidence[]>([])
  const [selected, setSelected] = useState(0)
  const [source, setSource] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [leftOpen, setLeftOpen] = useState(false)
  const [rightOpen, setRightOpen] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    void Promise.all([
      getStatus(controller.signal).then(setStatus),
      searchEvidence(query, undefined, undefined, controller.signal).then(setEvidence),
    ])
      .catch((caught: unknown) => {
        if (controller.signal.aborted) return
        setError(caught instanceof Error ? caught.message : 'Cortana is unavailable')
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false)
      })
    return () => controller.abort()
  }, [])

  async function runSearch(value: string, nextSource = source) {
    setLoading(true)
    setError('')
    try {
      setEvidence(await searchEvidence(value, undefined, nextSource || undefined))
      setActiveQuery(value)
      setSelected(0)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Search failed')
      setEvidence([])
    } finally {
      setLoading(false)
    }
  }

  function submit(event: FormEvent) {
    event.preventDefault()
    if (query.trim()) void runSearch(query.trim())
  }

  function chooseSource(next: string) {
    const value = source === next ? '' : next
    setSource(value)
    setLeftOpen(false)
    void runSearch(query.trim() || activeQuery, value)
  }

  const context = useMemo(() => buildAgentContext(activeQuery, evidence), [activeQuery, evidence])

  return (
    <div className="shell">
      <header className="titlebar">
        <TitleActions onOpenSources={() => setLeftOpen(true)} />
        <form className="search-form" onSubmit={submit}>
          <Search size={18} />
          <input
            aria-label="Search your knowledge"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          {loading ? <LoaderCircle className="spin" size={16} /> : <kbd>⌘ K</kbd>}
        </form>
        <TitleActions context onOpenContext={() => setRightOpen(true)} />
      </header>
      <Navigation />
      <SourcePanel
        open={leftOpen}
        sources={status?.sources ?? []}
        selected={source}
        onSelect={chooseSource}
        onClose={() => setLeftOpen(false)}
      />
      <Workspace
        query={activeQuery}
        evidence={evidence}
        selected={selected}
        loading={loading}
        error={error}
        onSelect={setSelected}
        onRetry={() => void runSearch(query)}
      />
      <ContextPanel
        open={rightOpen}
        query={activeQuery}
        evidence={evidence}
        selected={selected}
        status={status}
        context={context}
        onSelect={setSelected}
        onClose={() => setRightOpen(false)}
      />
      <footer className="statusbar">
        <span className={error ? 'health error' : 'health'}>
          <i /> Index {error ? 'offline' : 'healthy'}
        </span>
        <span>Embedding: {status?.embedding_fingerprint?.split(':')[1] ?? '—'}</span>
        <span>
          <FileText size={13} /> Docs: {(status?.documents ?? 0).toLocaleString()}
        </span>
        <span className="status-spacer" />
        {isDemoMode && <span className="demo-badge">Demo data</span>}
        <span>Workspace: All</span>
      </footer>
    </div>
  )
}
