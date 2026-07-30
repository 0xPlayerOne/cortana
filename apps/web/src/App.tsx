import { FileText, LoaderCircle, Search } from 'lucide-react'
import {
  type CSSProperties,
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'

import {
  getAnswer,
  getDesktopInfo,
  getDesktopSettings,
  getDocument,
  getDocuments,
  getContext,
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
  ContextBundle,
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
  const [documentQuery, setDocumentQuery] = useState('')
  const [debouncedDocumentQuery, setDebouncedDocumentQuery] = useState('')
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false)
  const [sourceWidth, setSourceWidth] = useState(270)
  const [contextWidth, setContextWidth] = useState(350)
  const [contextBundle, setContextBundle] = useState<ContextBundle | null>(null)
  const [contextLoading, setContextLoading] = useState(false)
  const [contextError, setContextError] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const documentScope = `${workspace}\u0000${source}\u0000${debouncedDocumentQuery}`
  const documentScopeRef = useRef(documentScope)
  const documentPageLoadingRef = useRef(false)
  const sourceWidthRef = useRef(sourceWidth)
  const contextWidthRef = useRef(contextWidth)
  documentScopeRef.current = documentScope
  sourceWidthRef.current = sourceWidth
  contextWidthRef.current = contextWidth

  useEffect(() => {
    const timeout = window.setTimeout(() => setDebouncedDocumentQuery(documentQuery.trim()), 250)
    return () => window.clearTimeout(timeout)
  }, [documentQuery])

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
    const requestedScope = documentScopeRef.current
    documentPageLoadingRef.current = true
    setDocumentsLoading(true)
    setDocumentsError('')
    setActiveDocument(null)
    void getDocuments(
      workspace || undefined,
      source || undefined,
      debouncedDocumentQuery || undefined,
      undefined,
      controller.signal
    )
      .then((page) => {
        if (documentScopeRef.current !== requestedScope) return
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
        if (!controller.signal.aborted && documentScopeRef.current === requestedScope) {
          documentPageLoadingRef.current = false
          setDocumentsLoading(false)
        }
      })
    return () => {
      controller.abort()
      if (documentScopeRef.current === requestedScope) documentPageLoadingRef.current = false
    }
  }, [debouncedDocumentQuery, source, workspace])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const modifier = event.metaKey || event.ctrlKey
      if (modifier && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        searchRef.current?.focus()
        searchRef.current?.select()
      } else if (modifier && event.key.toLowerCase() === 'p') {
        event.preventDefault()
        setCommandPaletteOpen((open) => !open)
      } else if (modifier && event.shiftKey && event.key.toLowerCase() === 'f') {
        event.preventDefault()
        setLeftOpen(true)
        window.setTimeout(() => document.getElementById('document-filter')?.focus(), 0)
      } else if (event.key === 'Escape') {
        setCommandPaletteOpen(false)
        setLeftOpen(false)
        setRightOpen(false)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  useEffect(() => {
    function clampPaneWidths() {
      if (window.innerWidth <= 1280) return
      const available = window.innerWidth - 72 - 520
      const nextSource = Math.min(sourceWidthRef.current, Math.max(220, available - 280))
      const nextContext = Math.min(contextWidthRef.current, Math.max(280, available - nextSource))
      setSourceWidth(nextSource)
      setContextWidth(nextContext)
    }
    window.addEventListener('resize', clampPaneWidths)
    clampPaneWidths()
    return () => window.removeEventListener('resize', clampPaneWidths)
  }, [])

  useEffect(() => {
    if (!isDesktopApp) return
    void getDesktopSettings()
      .then((next) => {
        setDesktopSettings(next)
        if (next.needs_setup) setView('settings')
      })
      .catch(() => {
        setView('settings')
      })
    void getDesktopInfo()
      .then(setDesktopInfo)
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
      setContextBundle(null)
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

  function chooseSource(next: string, project?: string) {
    const sameScope = source === next && (!project || workspace === project)
    const value = sameScope ? '' : next
    if (project) setWorkspace(project)
    setSource(value)
    setLeftOpen(false)
  }

  function chooseWorkspace(next: string) {
    setWorkspace(next)
    setSource('')
  }

  async function loadMoreDocuments() {
    if (!documentCursor || documentsLoading || documentPageLoadingRef.current) return
    const requestedScope = documentScopeRef.current
    documentPageLoadingRef.current = true
    setDocumentsLoading(true)
    setDocumentsError('')
    try {
      const page = await getDocuments(
        workspace || undefined,
        source || undefined,
        debouncedDocumentQuery || undefined,
        documentCursor
      )
      if (documentScopeRef.current !== requestedScope) return
      setDocuments((current) => [
        ...current,
        ...page.documents.filter((item) => !current.some((existing) => existing.id === item.id)),
      ])
      setDocumentCursor(page.next_cursor)
    } catch (caught) {
      if (documentScopeRef.current === requestedScope) {
        setDocumentsError(caught instanceof Error ? caught.message : 'Documents unavailable')
      }
    } finally {
      if (documentScopeRef.current === requestedScope) {
        documentPageLoadingRef.current = false
        setDocumentsLoading(false)
      }
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

  async function retrieveAgentContext() {
    setContextLoading(true)
    setContextError('')
    try {
      setContextBundle(await getContext(activeQuery, workspace || undefined, source || undefined))
    } catch (caught) {
      setContextError(caught instanceof Error ? caught.message : 'Context retrieval failed')
    } finally {
      setContextLoading(false)
    }
  }

  function maximumPaneWidth(side: 'source' | 'context') {
    if (window.innerWidth <= 1280) return 520
    return Math.max(
      side === 'source' ? 220 : 280,
      Math.min(520, window.innerWidth - 72 - 520 - (side === 'source' ? contextWidth : sourceWidth))
    )
  }

  function beginResize(side: 'source' | 'context', event: ReactPointerEvent<HTMLDivElement>) {
    event.preventDefault()
    const startX = event.clientX
    const startWidth = side === 'source' ? sourceWidth : contextWidth
    const move = (moveEvent: PointerEvent) => {
      const delta = moveEvent.clientX - startX
      const width = side === 'source' ? startWidth + delta : startWidth - delta
      const minimum = side === 'source' ? 220 : 280
      const bounded = Math.max(minimum, Math.min(width, maximumPaneWidth(side)))
      if (side === 'source') setSourceWidth(bounded)
      else setContextWidth(bounded)
    }
    const stop = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', stop)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', stop)
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
    <div
      className="shell"
      style={
        {
          '--source-width': `${sourceWidth}px`,
          '--context-width': `${contextWidth}px`,
        } as CSSProperties
      }
    >
      <header className="titlebar">
        <TitleActions onOpenSources={() => setLeftOpen(true)} />
        <form className="search-form" onSubmit={submit}>
          <Search size={18} />
          <input
            ref={searchRef}
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
            workspaces={workspaces}
            documentQuery={documentQuery}
            selected={source}
            documents={documents}
            selectedDocument={activeDocument?.id ?? ''}
            documentsLoading={documentsLoading}
            documentsError={documentsError}
            hasMoreDocuments={Boolean(documentCursor)}
            onSelect={chooseSource}
            onSelectWorkspace={chooseWorkspace}
            onDocumentQueryChange={setDocumentQuery}
            onSelectDocument={(id) => void chooseDocument(id)}
            onLoadMoreDocuments={() => void loadMoreDocuments()}
            onOpenSettings={() => setView('settings')}
            onClose={() => setLeftOpen(false)}
          />
          <div
            className="pane-resizer source-resizer"
            role="separator"
            aria-label="Resize sources panel"
            aria-orientation="vertical"
            aria-valuemin={220}
            aria-valuemax={maximumPaneWidth('source')}
            aria-valuenow={sourceWidth}
            tabIndex={0}
            onPointerDown={(event) => beginResize('source', event)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowLeft') setSourceWidth((width) => Math.max(220, width - 16))
              else if (event.key === 'ArrowRight')
                setSourceWidth((width) => Math.min(maximumPaneWidth('source'), width + 16))
              else return
              event.preventDefault()
            }}
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
            onSelectDocument={(id) => void chooseDocument(id)}
            onRetry={() => void runSearch(query)}
          />
          <ContextPanel
            open={rightOpen}
            query={activeQuery}
            evidence={evidence}
            answer={answer}
            selected={selected}
            status={status}
            context={agentContext}
            contextTokens={estimateTokens(agentContext)}
            serverContext={contextBundle}
            contextLoading={contextLoading}
            contextError={contextError}
            onRetrieveContext={() => void retrieveAgentContext()}
            onSelect={setSelected}
            onClose={() => setRightOpen(false)}
          />
          <div
            className="pane-resizer context-resizer"
            role="separator"
            aria-label="Resize context panel"
            aria-orientation="vertical"
            aria-valuemin={280}
            aria-valuemax={maximumPaneWidth('context')}
            aria-valuenow={contextWidth}
            tabIndex={0}
            onPointerDown={(event) => beginResize('context', event)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowLeft')
                setContextWidth((width) => Math.min(maximumPaneWidth('context'), width + 16))
              else if (event.key === 'ArrowRight')
                setContextWidth((width) => Math.max(280, width - 16))
              else return
              event.preventDefault()
            }}
          />
        </>
      )}
      {commandPaletteOpen && (
        <div
          className="command-palette-backdrop"
          role="presentation"
          onMouseDown={() => setCommandPaletteOpen(false)}
        >
          <div
            className="command-palette"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <strong>Commands</strong>
            <button
              autoFocus
              onClick={() => {
                setCommandPaletteOpen(false)
                searchRef.current?.focus()
              }}
            >
              Search the brain <kbd>⌘ K</kbd>
            </button>
            <button
              onClick={() => {
                setCommandPaletteOpen(false)
                setLeftOpen(true)
                window.setTimeout(() => document.getElementById('document-filter')?.focus(), 0)
              }}
            >
              Filter documents <kbd>⌘ ⇧ F</kbd>
            </button>
            <button
              onClick={() => {
                setCommandPaletteOpen(false)
                setSource('')
                setWorkspace('')
                setDocumentQuery('')
              }}
            >
              Clear workspace scope
            </button>
            {workspaces.map((item) => (
              <button
                key={item.id}
                onClick={() => {
                  chooseWorkspace(item.id)
                  setCommandPaletteOpen(false)
                }}
              >
                Switch to {item.name}
              </button>
            ))}
            <button
              onClick={() => {
                setCommandPaletteOpen(false)
                setView('settings')
              }}
            >
              Open settings
            </button>
          </div>
        </div>
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
