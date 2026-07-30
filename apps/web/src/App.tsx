import { FileText, LoaderCircle, Search } from 'lucide-react'
import { type FormEvent, useEffect, useMemo, useState } from 'react'

import {
  getAnswer,
  getDesktopInfo,
  getDesktopSettings,
  getDocument,
  getDocuments,
  getStatus,
  isDemoMode,
  isDesktopApp,
} from './api'
import { ContextPanel } from './components/ContextPanel'
import { type AppView, Navigation, TitleActions } from './components/Navigation'
import { SettingsView } from './components/SettingsView'
import { SourcePanel } from './components/SourcePanel'
import { Workspace } from './components/Workspace'
import { buildAgentContext, estimateTokens } from './context'
import type {
  AnswerResponse,
  BrainDocument,
  BrainDocumentSummary,
  BrainStatus,
  DesktopSettings,
  DesktopInfo,
  Evidence,
} from './types'

export function App() {
  const [query, setQuery] = useState('How do releases work?')
  const [activeQuery, setActiveQuery] = useState(query)
  const [status, setStatus] = useState<BrainStatus | null>(null)
  const [evidence, setEvidence] = useState<Evidence[]>([])
  const [answer, setAnswer] = useState<AnswerResponse | null>(null)
  const [selected, setSelected] = useState(0)
  const [source, setSource] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [statusError, setStatusError] = useState('')
  const [leftOpen, setLeftOpen] = useState(false)
  const [rightOpen, setRightOpen] = useState(false)
  const [view, setView] = useState<AppView>('knowledge')
  const [workspace, setWorkspace] = useState('')
  const [desktopSettings, setDesktopSettings] = useState<DesktopSettings | null>(null)
  const [desktopInfo, setDesktopInfo] = useState<DesktopInfo | null>(null)
  const [settingsSection, setSettingsSection] = useState<'readiness' | 'updates'>('readiness')
  const [documents, setDocuments] = useState<BrainDocumentSummary[]>([])
  const [documentCursor, setDocumentCursor] = useState<string | null>(null)
  const [documentsLoading, setDocumentsLoading] = useState(true)
  const [documentsError, setDocumentsError] = useState('')
  const [activeDocument, setActiveDocument] = useState<BrainDocument | null>(null)
  const [documentLoading, setDocumentLoading] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    const statusRequest = getStatus(controller.signal)
      .then(setStatus)
      .catch((caught: unknown) => {
        if (controller.signal.aborted) return
        setStatusError(caught instanceof Error ? caught.message : 'Status unavailable')
      })
    void statusRequest.finally(() => {
      if (!controller.signal.aborted) setLoading(false)
    })
    return () => controller.abort()
  }, [])

  useEffect(() => {
    const controller = new AbortController()
    setDocumentsLoading(true)
    setDocumentsError('')
    setActiveDocument(null)
    void getDocuments(workspace || undefined, source || undefined, undefined, controller.signal)
      .then((page) => {
        setDocuments(page.documents)
        setDocumentCursor(page.next_cursor)
      })
      .catch((caught: unknown) => {
        if (controller.signal.aborted) return
        setDocuments([])
        setDocumentCursor(null)
        setDocumentsError(caught instanceof Error ? caught.message : 'Documents unavailable')
      })
      .finally(() => {
        if (!controller.signal.aborted) setDocumentsLoading(false)
      })
    return () => controller.abort()
  }, [source, workspace])

  useEffect(() => {
    if (!isDesktopApp) return
    void getDesktopSettings().then((next) => {
      setDesktopSettings(next)
      if (next.needs_setup) setView('settings')
    })
    void getDesktopInfo()
      .then(setDesktopInfo)
      .catch(() => {})
      .catch(() => {
        // The settings view will surface the local configuration error.
      })
  }, [])

  const agentContext = useMemo(
    () => buildAgentContext(activeQuery, evidence),
    [activeQuery, evidence]
  )

  useEffect(() => {
    const refresh = () => {
      void getStatus()
        .then((next) => {
          setStatus(next)
          setStatusError('')
        })
        .catch((caught: unknown) => {
          setStatusError(caught instanceof Error ? caught.message : 'Status unavailable')
        })
    }
    const interval = window.setInterval(refresh, 15_000)
    return () => window.clearInterval(interval)
  }, [])

  async function runSearch(value: string, nextSource = source, nextWorkspace = workspace) {
    setLoading(true)
    setError('')
    try {
      const next = await getAnswer(value, nextWorkspace || undefined, nextSource || undefined)
      setAnswer(next)
      setEvidence(next.evidence)
      setActiveQuery(value)
      setSelected(0)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Search failed')
      setAnswer(null)
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
  }

  function chooseWorkspace(next: string) {
    setWorkspace(next)
    setSource('')
  }

  async function loadMoreDocuments() {
    if (!documentCursor || documentsLoading) return
    setDocumentsLoading(true)
    setDocumentsError('')
    try {
      const page = await getDocuments(workspace || undefined, source || undefined, documentCursor)
      setDocuments((current) => [
        ...current,
        ...page.documents.filter((item) => !current.some((existing) => existing.id === item.id)),
      ])
      setDocumentCursor(page.next_cursor)
    } catch (caught) {
      setDocumentsError(caught instanceof Error ? caught.message : 'Documents unavailable')
    } finally {
      setDocumentsLoading(false)
    }
  }

  async function chooseDocument(id: string) {
    setDocumentLoading(true)
    setDocumentsError('')
    try {
      setActiveDocument(await getDocument(id))
      setLeftOpen(false)
    } catch (caught) {
      setDocumentsError(caught instanceof Error ? caught.message : 'Document unavailable')
    } finally {
      setDocumentLoading(false)
    }
  }

  const workspaces = desktopSettings?.workspaces.length
    ? desktopSettings.workspaces
    : status?.workspaces.length
      ? status.workspaces
      : Array.from(new Set(status?.sources.map((item) => item.project) ?? [])).map((id) => ({
          id,
          name: id[0]?.toUpperCase() + id.slice(1),
          account_label: null,
          color: null,
        }))

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
      <Navigation view={view} onNavigate={setView} />
      {view === 'settings' ? (
        <SettingsView
          initialSection={settingsSection}
          onSaved={(next) => {
            setDesktopSettings(next)
            if (workspace && !next.workspaces.some((item) => item.id === workspace)) {
              setWorkspace('')
            }
          }}
        />
      ) : (
        <>
          <SourcePanel
            open={leftOpen}
            status={status}
            workspace={workspace}
            selected={source}
            documents={documents}
            selectedDocument={activeDocument?.id ?? ''}
            documentsLoading={documentsLoading}
            documentsError={documentsError}
            hasMoreDocuments={Boolean(documentCursor)}
            onSelect={chooseSource}
            onSelectDocument={(id) => void chooseDocument(id)}
            onLoadMoreDocuments={() => void loadMoreDocuments()}
            onOpenSettings={() => setView('settings')}
            onClose={() => setLeftOpen(false)}
          />
          <Workspace
            query={activeQuery}
            answer={answer}
            evidence={evidence}
            selected={selected}
            loading={loading}
            error={error}
            document={activeDocument}
            documentLoading={documentLoading}
            onSelect={setSelected}
            onRetry={() => void runSearch(query)}
          />
          <ContextPanel
            open={rightOpen}
            query={activeQuery}
            evidence={evidence}
            selected={selected}
            status={status}
            context={agentContext}
            contextTokens={estimateTokens(agentContext)}
            onSelect={setSelected}
            onClose={() => setRightOpen(false)}
          />
        </>
      )}
      <footer className="statusbar">
        <span className={statusError ? 'health error' : 'health'}>
          <i /> Index {statusError ? 'offline' : status ? 'online' : 'checking'}
        </span>
        <span>Embedding: {status?.embedding_fingerprint?.split(':')[1] ?? '—'}</span>
        <span>Query: {status?.query.mode ?? '—'}</span>
        <span>
          <FileText size={13} /> Docs: {(status?.documents ?? 0).toLocaleString()}
        </span>
        <IngestionIndicator status={status} />
        <span className="status-spacer" />
        {isDemoMode && <span className="demo-badge">Demo data</span>}
        {isDesktopApp && (
          <button
            type="button"
            className="status-link"
            onClick={() => {
              setSettingsSection('updates')
              setView('settings')
            }}
          >
            Cortana {desktopInfo?.desktop_version || '—'} · Updates
          </button>
        )}
        <label className="workspace-select">
          Workspace:
          <select value={workspace} onChange={(event) => chooseWorkspace(event.target.value)}>
            <option value="">All</option>
            {workspaces.map((item) => (
              <option value={item.id} key={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
      </footer>
    </div>
  )
}

function IngestionIndicator({ status }: { status: BrainStatus | null }) {
  const runs = status?.sync_runs ?? []
  const running = runs.filter((run) => run.status === 'running').length
  const failed = runs.filter((run) =>
    ['failed', 'cancelled', 'budget_exceeded'].includes(run.status)
  ).length
  const state = running
    ? 'running'
    : failed
      ? 'warning'
      : status?.ingestion.scheduled
        ? 'healthy'
        : 'manual'
  const label = running
    ? `${running} running`
    : failed
      ? `${failed} need attention`
      : status?.ingestion.scheduled
        ? 'scheduled'
        : 'paused · manual'
  return (
    <span className={`ingestion-health ${state}`}>
      <i /> Ingestion: {label}
    </span>
  )
}
