import { FileText, LoaderCircle, Search } from 'lucide-react'
import {
  type CSSProperties,
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'

import {
  getAnswer,
  getDesktopInfo,
  getDesktopInstaller,
  getDesktopSettings,
  getDesktopUpdate,
  getDocument,
  getDocuments,
  getContext,
  getGraph,
  getStatus,
  cancelDesktopSourceValidation,
  scanDesktopReadiness,
  isDemoMode,
  isDesktopApp,
  openDesktopProject,
} from './api'
import { ContextPanel } from './components/ContextPanel'
import { type AppView, Navigation, TitleActions } from './components/Navigation'
import { SettingsView } from './components/SettingsView'
import { SourcePanel } from './components/SourcePanel'
import { UtilityView } from './components/UtilityView'
import { Workspace, type WorkspaceTab } from './components/Workspace'
import { buildAgentContext, estimateTokens } from './context'
import { embeddingLabel } from './operations'
import { activeJobs, describeSourceJobProgress, useSourceJobs } from './sourceJobs'
import type {
  AnswerResponse,
  BrainDocument,
  BrainDocumentSummary,
  BrainGraphPage,
  BrainStatus,
  ContextBundle,
  DesktopSettings,
  DesktopInfo,
  DesktopInstallJob,
  DesktopReadiness,
  DesktopReadinessActivity,
  DesktopServiceActivity,
  DesktopSourceJob,
  DesktopUpdate,
  Evidence,
} from './types'

const STATUS_REFRESH_MS = 15_000
const INSTALLER_POLL_MS = 1_000

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
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>('document')
  const [desktopSettings, setDesktopSettings] = useState<DesktopSettings | null>(null)
  const [desktopInfo, setDesktopInfo] = useState<DesktopInfo | null>(null)
  const [settingsSection, setSettingsSection] = useState<
    'readiness' | 'services' | 'updates' | 'sources' | 'hindsight' | 'honcho'
  >('readiness')
  const [settingsDirty, setSettingsDirty] = useState(false)
  const [installerJob, setInstallerJob] = useState<DesktopInstallJob | null>(null)
  const [desktopUpdate, setDesktopUpdate] = useState<DesktopUpdate | null>(null)
  const [desktopReadiness, setDesktopReadiness] = useState<DesktopReadiness | null>(null)
  const [readinessActivity, setReadinessActivity] = useState<DesktopReadinessActivity | null>(null)
  const [serviceActivity, setServiceActivity] = useState<DesktopServiceActivity | null>(null)
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
  const [graph, setGraph] = useState<BrainGraphPage | null>(null)
  const [graphLoading, setGraphLoading] = useState(false)
  const [graphError, setGraphError] = useState('')
  const searchRequestRef = useRef(0)
  const [contextError, setContextError] = useState('')
  const [queryHistory, setQueryHistory] = useState<string[]>([])
  const [queryHistoryIndex, setQueryHistoryIndex] = useState(-1)
  const searchRef = useRef<HTMLInputElement>(null)
  const sourceJobs = useSourceJobs()
  const installerPollingRef = useRef(false)
  const installerStatusRef = useRef<DesktopInstallJob['status'] | null>(null)
  const updatePollingRef = useRef(false)
  const refreshedSourceJobsRef = useRef<Set<string>>(new Set())
  const documentScope = `${workspace}\u0000${source}\u0000${debouncedDocumentQuery}`
  const documentScopeRef = useRef(documentScope)
  const searchAbortRef = useRef<AbortController | null>(null)
  const searchScopeRef = useRef('')
  const contextAbortRef = useRef<AbortController | null>(null)
  const contextScopeRef = useRef('')
  const documentListAbortRef = useRef<AbortController | null>(null)
  const documentSelectAbortRef = useRef<AbortController | null>(null)
  const graphAbortRef = useRef<AbortController | null>(null)
  const contextRequestRef = useRef(0)
  const documentListRequestRef = useRef(0)
  const documentSelectRequestRef = useRef(0)
  const graphRequestRef = useRef(0)
  const documentPageLoadingRef = useRef(false)
  const sourceWidthRef = useRef(sourceWidth)
  const contextWidthRef = useRef(contextWidth)
  documentScopeRef.current = documentScope
  sourceWidthRef.current = sourceWidth
  contextWidthRef.current = contextWidth

  const runReadinessScan = useCallback(async (): Promise<DesktopReadiness> => {
    setReadinessActivity({ status: 'running', detail: null })
    try {
      const next = await scanDesktopReadiness()
      setDesktopReadiness(next)
      setReadinessActivity({ status: 'succeeded', detail: null })
      return next
    } catch (caught) {
      const detail = caught instanceof Error ? caught.message : 'Readiness scan failed'
      setReadinessActivity({ status: 'failed', detail })
      throw caught
    }
  }, [])

  useEffect(() => {
    const timeout = window.setTimeout(() => setDebouncedDocumentQuery(documentQuery.trim()), 250)
    return () => window.clearTimeout(timeout)
  }, [documentQuery])

  useEffect(() => {
    let disposed = false
    let initialRequest = true
    let controller: AbortController | null = null

    const refresh = () => {
      controller?.abort()
      const nextController = new AbortController()
      controller = nextController
      const isInitialRequest = initialRequest
      initialRequest = false
      void getStatus(nextController.signal)
        .then((next) => {
          if (disposed || nextController.signal.aborted) return
          setStatus(next)
          setStatusError('')
        })
        .catch((caught: unknown) => {
          if (disposed || nextController.signal.aborted) return
          setStatusError(caught instanceof Error ? caught.message : 'Status unavailable')
        })
        .finally(() => {
          // Status is independent from an in-flight query. If a user submits
          // a search before the first health request finishes, the status
          // response must not hide the query's loading state.
          if (
            isInitialRequest &&
            !disposed &&
            !nextController.signal.aborted &&
            searchRequestRef.current === 0
          ) {
            setLoading(false)
          }
        })
    }

    refresh()
    const timer = window.setInterval(refresh, STATUS_REFRESH_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
      controller?.abort()
    }
  }, [])

  useEffect(() => {
    const requestId = ++documentListRequestRef.current
    documentListAbortRef.current?.abort()
    const controller = new AbortController()
    documentListAbortRef.current = controller
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
        if (documentListRequestRef.current !== requestId) return
        if (documentScopeRef.current !== requestedScope) return
        setDocuments(page.documents)
        setDocumentCursor(page.next_cursor)
      })
      .catch((caught: unknown) => {
        if (isAbort(caught) || controller.signal.aborted) return
        if (documentListRequestRef.current !== requestId) return
        if (documentScopeRef.current !== requestedScope) return
        setDocuments([])
        setDocumentCursor(null)
        setDocumentsError(caught instanceof Error ? caught.message : 'Documents unavailable')
      })
      .finally(() => {
        if (
          documentListRequestRef.current === requestId &&
          !controller.signal.aborted &&
          documentScopeRef.current === requestedScope
        ) {
          documentPageLoadingRef.current = false
          setDocumentsLoading(false)
        }
      })
    return () => {
      controller.abort()
      if (
        documentListRequestRef.current === requestId &&
        documentScopeRef.current === requestedScope
      ) {
        documentPageLoadingRef.current = false
        setDocumentsLoading(false)
      }
    }
  }, [debouncedDocumentQuery, source, workspace])

  useEffect(() => {
    if (view !== 'knowledge' || workspaceTab !== 'graph') return
    const requestId = ++graphRequestRef.current
    graphAbortRef.current?.abort()
    const controller = new AbortController()
    graphAbortRef.current = controller
    setGraph(null)
    setGraphError('')
    setGraphLoading(true)
    void getGraph(
      workspace || undefined,
      source || undefined,
      debouncedDocumentQuery || undefined,
      undefined,
      controller.signal
    )
      .then((next) => {
        if (graphRequestRef.current !== requestId || controller.signal.aborted) return
        setGraph(next)
      })
      .catch((caught: unknown) => {
        if (isAbort(caught) || controller.signal.aborted) return
        if (graphRequestRef.current !== requestId) return
        setGraphError(caught instanceof Error ? caught.message : 'Graph data unavailable')
      })
      .finally(() => {
        if (graphRequestRef.current === requestId && !controller.signal.aborted) {
          setGraphLoading(false)
        }
      })
    return () => controller.abort()
  }, [debouncedDocumentQuery, source, view, workspace, workspaceTab])

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
    void getDesktopUpdate()
      .then(setDesktopUpdate)
      .catch(() => {
        // The Updates section will surface a more specific updater error.
      })
  }, [])

  useEffect(() => {
    if (!isDesktopApp) return
    const completed = sourceJobs.jobs.filter(
      (job) =>
        job.completed_at_unix_seconds !== null && !['running', 'cancelling'].includes(job.status)
    )
    const unseen = completed.filter((job) => !refreshedSourceJobsRef.current.has(job.id))
    if (unseen.length === 0) return
    unseen.forEach((job) => refreshedSourceJobsRef.current.add(job.id))

    let active = true
    void getStatus()
      .then((next) => {
        if (!active) return
        setStatus(next)
        setStatusError('')
      })
      .catch((caught: unknown) => {
        if (!active) return
        setStatusError(caught instanceof Error ? caught.message : 'Status unavailable')
      })
    return () => {
      active = false
    }
  }, [sourceJobs.jobs])

  useEffect(() => {
    if (!isDesktopApp || !installerJob || !isActiveInstaller(installerJob)) return
    let disposed = false
    const poll = () => {
      if (disposed || installerPollingRef.current) return
      installerPollingRef.current = true
      void getDesktopInstaller(installerJob.id)
        .then((next) => {
          if (!disposed) setInstallerJob(next)
        })
        .catch((caught: unknown) => {
          // Installer jobs are held in native memory. If a Desktop restart
          // discarded the job, clear the stale shell snapshot instead of
          // showing an install that can no longer be inspected.
          if (!disposed && isMissingInstallerJobError(caught)) setInstallerJob(null)
        })
        .finally(() => {
          installerPollingRef.current = false
        })
    }
    poll()
    const timer = window.setInterval(poll, INSTALLER_POLL_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [installerJob?.id, installerJob?.status])

  useEffect(() => {
    const previous = installerStatusRef.current
    const next = installerJob?.status ?? null
    installerStatusRef.current = next
    if (
      !isDesktopApp ||
      !installerJob ||
      next !== 'succeeded' ||
      previous === 'succeeded' ||
      !previous ||
      !['running', 'cancelling'].includes(previous)
    ) {
      return
    }
    // The shell owns installer polling, so it also owns the post-install
    // readiness scan. This keeps the result when Settings is unmounted.
    void runReadinessScan().catch(() => {})
  }, [installerJob?.status, runReadinessScan])

  useEffect(() => {
    if (
      !isDesktopApp ||
      !desktopUpdate ||
      !['downloading', 'installing'].includes(desktopUpdate.phase)
    ) {
      return
    }
    let disposed = false
    const poll = () => {
      if (disposed || updatePollingRef.current) return
      updatePollingRef.current = true
      void getDesktopUpdate()
        .then((next) => {
          if (!disposed) setDesktopUpdate(next)
        })
        .catch(() => {
          // Keep the last progress snapshot while the native updater is busy.
        })
        .finally(() => {
          updatePollingRef.current = false
        })
    }
    poll()
    const timer = window.setInterval(poll, 400)
    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [desktopUpdate?.phase])

  const agentContext = useMemo(
    () => buildAgentContext(activeQuery, evidence),
    [activeQuery, evidence]
  )

  function isAbort(caught: unknown) {
    return caught instanceof DOMException
      ? caught.name === 'AbortError'
      : (caught as { name?: string } | null)?.name === 'AbortError'
  }

  function searchScope(nextSource: string, nextWorkspace: string, query: string) {
    return `${nextWorkspace}\u0000${nextSource}\u0000${query}`
  }

  function contextScope(nextQuery: string, nextWorkspace: string, nextSource: string) {
    return `${nextWorkspace}\u0000${nextSource}\u0000${nextQuery}`
  }

  function abortSearchRequest(): void {
    // A connector or test double may resolve after AbortController fires. The
    // generation check keeps that stale result from returning to the shell.
    searchRequestRef.current += 1
    searchAbortRef.current?.abort()
    setLoading(false)
    setError('')
  }

  function abortContextRequest(): void {
    contextRequestRef.current += 1
    contextAbortRef.current?.abort()
    setContextBundle(null)
    setContextLoading(false)
    setContextError('')
  }

  function clearScopedResults(): void {
    documentListRequestRef.current += 1
    documentListAbortRef.current?.abort()
    documentSelectRequestRef.current += 1
    documentSelectAbortRef.current?.abort()
    graphRequestRef.current += 1
    graphAbortRef.current?.abort()
    documentPageLoadingRef.current = false
    setDocuments([])
    setDocumentCursor(null)
    setDocumentsLoading(true)
    setDocumentsError('')
    setDocumentLoading(false)
    setAnswer(null)
    setEvidence([])
    setSelected(0)
    setActiveDocument(null)
    setWorkspaceTab('document')
    setGraph(null)
    setGraphError('')
    setGraphLoading(false)
  }

  function scopeSources(nextWorkspace: string, nextSource = source) {
    const nextScope = searchScope(nextSource, nextWorkspace, query)
    searchScopeRef.current = nextScope
    contextScopeRef.current = contextScope(activeQuery, nextWorkspace, nextSource)
  }

  async function runSearch(
    value: string,
    nextSource = source,
    nextWorkspace = workspace,
    recordHistory = true
  ) {
    if (recordHistory) {
      const sliced = queryHistory.slice(0, queryHistoryIndex + 1)
      const nextHistory = sliced.at(-1) === value ? sliced : [...sliced, value]
      setQueryHistory(nextHistory)
      setQueryHistoryIndex(nextHistory.length - 1)
    }

    const requestId = ++searchRequestRef.current
    const requestedScope = searchScope(nextSource, nextWorkspace, value)
    setLoading(true)
    setError('')
    // Keep the active result surface visible while retrieval is in flight.
    // Otherwise a search started from the document tab looks idle until the
    // answer arrives.
    setWorkspaceTab('answer')
    searchScopeRef.current = requestedScope
    const controller = new AbortController()
    searchAbortRef.current?.abort()
    searchAbortRef.current = controller
    try {
      const next = await getAnswer(
        value,
        nextWorkspace || undefined,
        nextSource || undefined,
        controller.signal
      )
      if (searchRequestRef.current !== requestId || searchScopeRef.current !== requestedScope) {
        return
      }
      setAnswer(next)
      setContextBundle(null)
      setEvidence(next.evidence)
      setActiveQuery(value)
      setSelected(0)
    } catch (caught) {
      if (controller.signal.aborted || isAbort(caught)) return
      if (searchRequestRef.current !== requestId || searchScopeRef.current !== requestedScope) {
        return
      }
      setError(caught instanceof Error ? caught.message : 'Search failed')
      setAnswer(null)
      setEvidence([])
    } finally {
      if (
        searchRequestRef.current === requestId &&
        searchScopeRef.current === requestedScope &&
        !controller.signal.aborted
      ) {
        setLoading(false)
      }
    }
  }

  function submit(event: FormEvent) {
    event.preventDefault()
    if (query.trim()) void runSearch(query.trim())
  }

  function chooseSource(next: string, project?: string) {
    const nextWorkspace = project ?? workspace
    const sameScope = source === next && workspace === nextWorkspace
    const nextSource = sameScope ? '' : next
    abortSearchRequest()
    abortContextRequest()
    clearScopedResults()
    scopeSources(nextWorkspace, nextSource)
    if (project) setWorkspace(project)
    setSource(nextSource)
    setLeftOpen(false)
  }

  function chooseWorkspace(next: string) {
    const nextWorkspace = next
    const nextSource = ''
    if (nextWorkspace !== workspace || source !== nextSource) {
      abortSearchRequest()
      abortContextRequest()
      clearScopedResults()
    }
    scopeSources(nextWorkspace, nextSource)
    setWorkspace(nextWorkspace)
    setSource(nextSource)
  }

  async function loadMoreDocuments() {
    if (!documentCursor || documentsLoading || documentPageLoadingRef.current) return
    const requestedScope = documentScopeRef.current
    const requestId = ++documentListRequestRef.current
    const controller = new AbortController()
    documentListAbortRef.current?.abort()
    documentListAbortRef.current = controller
    documentPageLoadingRef.current = true
    setDocumentsLoading(true)
    setDocumentsError('')
    try {
      const page = await getDocuments(
        workspace || undefined,
        source || undefined,
        debouncedDocumentQuery || undefined,
        documentCursor,
        controller.signal
      )
      if (documentListRequestRef.current !== requestId) return
      if (documentScopeRef.current !== requestedScope) return
      setDocuments((current) => [
        ...current,
        ...page.documents.filter((item) => !current.some((existing) => existing.id === item.id)),
      ])
      setDocumentCursor(page.next_cursor)
    } catch (caught) {
      if (isAbort(caught) || controller.signal.aborted) return
      if (documentListRequestRef.current !== requestId) return
      if (documentScopeRef.current === requestedScope) {
        setDocumentsError(caught instanceof Error ? caught.message : 'Documents unavailable')
      }
    } finally {
      if (
        documentListRequestRef.current === requestId &&
        !controller.signal.aborted &&
        documentScopeRef.current === requestedScope
      ) {
        documentPageLoadingRef.current = false
        setDocumentsLoading(false)
      }
    }
  }

  async function chooseDocument(id: string) {
    const requestId = ++documentSelectRequestRef.current
    const controller = new AbortController()
    documentSelectAbortRef.current?.abort()
    documentSelectAbortRef.current = controller
    setDocumentLoading(true)
    setDocumentsError('')
    try {
      const next = await getDocument(id, controller.signal)
      if (documentSelectRequestRef.current !== requestId) return
      setActiveDocument(next)
      setLeftOpen(false)
    } catch (caught) {
      if (
        documentSelectRequestRef.current !== requestId ||
        controller.signal.aborted ||
        isAbort(caught)
      )
        return
      setDocumentsError(caught instanceof Error ? caught.message : 'Document unavailable')
    } finally {
      if (documentSelectRequestRef.current === requestId && !controller.signal.aborted) {
        setDocumentLoading(false)
      }
    }
  }

  async function retrieveAgentContext() {
    const requestId = ++contextRequestRef.current
    const requestedScope = contextScope(activeQuery, workspace, source)
    setContextLoading(true)
    setContextError('')
    contextScopeRef.current = requestedScope
    const controller = new AbortController()
    contextAbortRef.current?.abort()
    contextAbortRef.current = controller
    try {
      const next = await getContext(
        activeQuery,
        workspace || undefined,
        source || undefined,
        controller.signal
      )
      if (contextRequestRef.current !== requestId || contextScopeRef.current !== requestedScope) {
        return
      }
      setContextBundle(next)
    } catch (caught) {
      if (
        contextAbortRef.current?.signal.aborted ||
        contextRequestRef.current !== requestId ||
        contextScopeRef.current !== requestedScope ||
        isAbort(caught)
      ) {
        return
      }
      setContextError(caught instanceof Error ? caught.message : 'Context retrieval failed')
    } finally {
      if (
        contextRequestRef.current === requestId &&
        contextScopeRef.current === requestedScope &&
        !controller.signal.aborted
      ) {
        setContextLoading(false)
      }
    }
  }

  function canLeaveSettings() {
    if (view !== 'settings' || !settingsDirty) return true
    const leave = window.confirm('Discard unsaved Cortana settings changes?')
    if (leave) setSettingsDirty(false)
    return leave
  }

  function navigate(next: AppView) {
    if (next !== 'settings' && !canLeaveSettings()) return
    setView(next)
    if (next === 'knowledge') setWorkspaceTab('document')
  }

  function focusSearch() {
    if (!canLeaveSettings()) return
    setView('knowledge')
    searchRef.current?.focus()
    searchRef.current?.select()
  }

  function focusDocumentFilter() {
    if (!canLeaveSettings()) return
    setView('knowledge')
    setLeftOpen(true)
    window.setTimeout(() => document.getElementById('document-filter')?.focus(), 0)
  }

  function cancelSourceJob(id: string) {
    void cancelDesktopSourceValidation(id)
      .then(sourceJobs.remember)
      .catch((caught: unknown) => {
        setStatusError(caught instanceof Error ? caught.message : 'Source job cancellation failed')
      })
  }

  function openGraph() {
    if (!canLeaveSettings()) return
    setView('knowledge')
    setWorkspaceTab('graph')
  }

  function openTimeline() {
    if (!canLeaveSettings()) return
    setView('knowledge')
    setWorkspaceTab('timeline')
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
        <TitleActions
          onOpenSources={() => setLeftOpen(true)}
          canGoBack={queryHistoryIndex > 0}
          canGoForward={queryHistoryIndex >= 0 && queryHistoryIndex < queryHistory.length - 1}
          onHistoryBack={() => {
            const nextIndex = queryHistoryIndex - 1
            if (nextIndex < 0) return
            const next = queryHistory[nextIndex]
            setQueryHistoryIndex(nextIndex)
            setQuery(next)
            void runSearch(next, source, workspace, false)
          }}
          onHistoryForward={() => {
            const nextIndex = queryHistoryIndex + 1
            if (nextIndex >= queryHistory.length) return
            const next = queryHistory[nextIndex]
            setQueryHistoryIndex(nextIndex)
            setQuery(next)
            void runSearch(next, source, workspace, false)
          }}
        />
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
        <TitleActions
          context
          onOpenContext={() => setRightOpen(true)}
          onOpenFilters={focusDocumentFilter}
          onOpenHistory={() => navigate('conversations')}
        />
      </header>
      <Navigation
        view={view}
        workspaceTab={workspaceTab}
        onNavigate={navigate}
        onSearch={focusSearch}
        onOpenGraph={openGraph}
        onOpenTimeline={openTimeline}
      />
      {view === 'settings' ? (
        <SettingsView
          initialSection={settingsSection}
          onDirtyChange={setSettingsDirty}
          onJob={sourceJobs.remember}
          sourceJobs={sourceJobs.jobs}
          installerJob={installerJob}
          onInstallerJob={setInstallerJob}
          readiness={desktopReadiness}
          onReadiness={setDesktopReadiness}
          readinessActivity={readinessActivity}
          onReadinessScan={runReadinessScan}
          desktopUpdate={desktopUpdate}
          onDesktopUpdate={setDesktopUpdate}
          serviceActivity={serviceActivity}
          onServiceActivity={setServiceActivity}
          onSaved={(next) => {
            setDesktopSettings(next)
            setSettingsDirty(false)
            void getStatus()
              .then((nextStatus) => {
                setStatus(nextStatus)
                setStatusError('')
              })
              .catch(() => {
                setStatusError('Status unavailable after saving settings')
              })
            if (workspace && !next.workspaces.some((item) => item.id === workspace)) {
              chooseWorkspace('')
            } else if (
              source &&
              !next.sources.some(
                (item) => item.name === source && (!workspace || item.project === workspace)
              )
            ) {
              abortSearchRequest()
              abortContextRequest()
              clearScopedResults()
              scopeSources(workspace, '')
              setSource('')
            }
          }}
        />
      ) : view === 'knowledge' ? (
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
            statusError={statusError}
            onSelect={chooseSource}
            onSelectWorkspace={chooseWorkspace}
            onDocumentQueryChange={setDocumentQuery}
            onSelectDocument={(id) => void chooseDocument(id)}
            onLoadMoreDocuments={() => void loadMoreDocuments()}
            onOpenSourcesSettings={() => {
              setSettingsSection('sources')
              setView('settings')
            }}
            onClose={() => setLeftOpen(false)}
            onCancelSourceJob={cancelSourceJob}
            jobs={sourceJobs.jobs}
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
            graph={graph}
            graphLoading={graphLoading}
            graphError={graphError}
            tab={workspaceTab}
            onTabChange={setWorkspaceTab}
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
      ) : (
        <UtilityView
          kind={view}
          status={status}
          sourceJobs={sourceJobs.jobs}
          query={activeQuery}
          answer={answer}
          evidence={evidence}
          loading={loading}
          error={error}
          contextBundle={contextBundle}
          contextLoading={contextLoading}
          contextError={contextError}
          contextTokens={estimateTokens(agentContext)}
          desktopAvailable={isDesktopApp}
          onSearchFocus={focusSearch}
          onRetrieveContext={() => void retrieveAgentContext()}
          onOpenSettings={() => setView('settings')}
          onOpenProject={() => void openDesktopProject()}
          onCancelSourceJob={cancelSourceJob}
        />
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
                focusSearch()
              }}
            >
              Search the brain <kbd>⌘ K</kbd>
            </button>
            <button
              onClick={() => {
                setCommandPaletteOpen(false)
                focusDocumentFilter()
              }}
            >
              Filter documents <kbd>⌘ ⇧ F</kbd>
            </button>
            <button
              onClick={() => {
                setCommandPaletteOpen(false)
                chooseWorkspace('')
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
        <span title={status?.embedding_fingerprint ?? undefined}>
          Embedding: {embeddingLabel(status?.embedding_fingerprint)}
        </span>
        <span>Query: {status?.query.mode ?? '—'}</span>
        <span>
          <FileText size={13} /> Docs: {(status?.documents ?? 0).toLocaleString()}
        </span>
        <IngestionIndicator status={status} />
        <ActiveSourceJobs
          jobs={sourceJobs.jobs}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setView('inbox')
          }}
        />
        <InstallerIndicator
          job={installerJob}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('readiness')
            setView('settings')
          }}
        />
        <ServiceActivityIndicator
          activity={serviceActivity}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('services')
            setView('settings')
          }}
        />
        <ReadinessActivityIndicator
          activity={readinessActivity}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('readiness')
            setView('settings')
          }}
        />
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
            {desktopUpdateStatusSuffix(desktopUpdate)}
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

function ActiveSourceJobs({ jobs, onOpen }: { jobs: DesktopSourceJob[]; onOpen: () => void }) {
  const active = activeJobs(jobs)
  if (active.length === 0) return null
  const detail = active
    .map((job) => `${job.source} · ${job.operation} · ${describeSourceJobProgress(job)}`)
    .join(', ')
  return (
    <button
      type="button"
      className="source-jobs status-link"
      aria-label="Open active source jobs"
      title={`${detail}. Open the activity inbox.`}
      onClick={onOpen}
    >
      <LoaderCircle className="spin" size={13} /> {active.length} active source job
      {active.length === 1 ? '' : 's'}
    </button>
  )
}

function isActiveInstaller(job: DesktopInstallJob): boolean {
  return job.status === 'running' || job.status === 'cancelling'
}

function isMissingInstallerJobError(error: unknown): boolean {
  return error instanceof Error && error.message.includes('installation job was not found')
}

function InstallerIndicator({
  job,
  onOpen,
}: {
  job: DesktopInstallJob | null
  onOpen: () => void
}) {
  if (!job) return null
  const active = isActiveInstaller(job)
  const state = active ? 'running' : job.status === 'succeeded' ? 'healthy' : 'warning'
  const label = active ? `Install: ${job.tool} · ${job.status}` : `Install: ${job.status}`
  return (
    <button
      type="button"
      className={`installer-health ${state}`}
      aria-label={`Open installer status for ${job.tool}`}
      title={`${job.summary}. Open readiness for details.`}
      onClick={onOpen}
    >
      <i /> {label}
    </button>
  )
}

function ServiceActivityIndicator({
  activity,
  onOpen,
}: {
  activity: DesktopServiceActivity | null
  onOpen: () => void
}) {
  if (!activity) return null
  const active = activity.status === 'running'
  const state = active ? 'running' : activity.status === 'succeeded' ? 'healthy' : 'warning'
  const action = activity.action === 'install' ? 'Install' : activity.action
  return (
    <button
      type="button"
      className={`service-activity-health ${state}`}
      aria-label="Open service activity"
      title={`${action} ${activity.target}${activity.detail ? `: ${activity.detail}` : ''}. Open services for details.`}
      onClick={onOpen}
    >
      {active && <LoaderCircle className="spin" size={13} />}
      {!active && <i />}
      Service: {action} {activity.target}
      {active ? '…' : activity.status === 'succeeded' ? ' · done' : ' · failed'}
    </button>
  )
}

function ReadinessActivityIndicator({
  activity,
  onOpen,
}: {
  activity: DesktopReadinessActivity | null
  onOpen: () => void
}) {
  if (!activity) return null
  const active = activity.status === 'running'
  const state = active ? 'running' : activity.status === 'succeeded' ? 'healthy' : 'warning'
  return (
    <button
      type="button"
      className={`readiness-activity-health ${state}`}
      aria-label="Open readiness activity"
      title={`${activity.detail || (active ? 'System readiness scan is running.' : 'System readiness scan completed.')}`}
      onClick={onOpen}
    >
      {active && <LoaderCircle className="spin" size={13} />}
      {!active && <i />}
      Readiness: {active ? 'scanning…' : activity.status === 'succeeded' ? 'ready' : 'failed'}
    </button>
  )
}

function desktopUpdateStatusSuffix(update: DesktopUpdate | null): string {
  if (!update) return ''
  if (update.phase === 'downloading' || update.phase === 'installing') {
    const percent =
      update.total_bytes && update.total_bytes > 0
        ? ` ${Math.min(100, Math.round((update.downloaded_bytes / update.total_bytes) * 100))}%`
        : ''
    return ` · ${update.phase}${percent}`
  }
  if (update.restart_required || update.phase === 'installed') return ' · Restart required'
  if (update.phase === 'failed') return ' · update failed'
  if (update.phase === 'available' && update.available_version) {
    return ` · ${update.available_version} available`
  }
  return ''
}
