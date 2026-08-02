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
  getDesktopHindsightStatus,
  getDesktopInfo,
  getDesktopInstaller,
  getDesktopHonchoStatus,
  getDesktopServices,
  getDesktopSettings,
  getDesktopUpdate,
  getDocument,
  getDocuments,
  getContext,
  getGraph,
  getStatus,
  openDesktopSourceSetup,
  saveDesktopSettings,
  cancelDesktopSourceValidation,
  scanDesktopReadiness,
  isDemoMode,
  isDesktopApp,
  openDesktopProject,
  startDesktopSourceAuthorization,
} from './api'
import { ContextPanel } from './components/ContextPanel'
import { type AppView, Navigation, TitleActions } from './components/Navigation'
import { SettingsView } from './components/SettingsView'
import { SourcePanel } from './components/SourcePanel'
import { UtilityView } from './components/UtilityView'
import { Workspace, type WorkspaceTab } from './components/Workspace'
import { buildAgentContext, estimateTokens } from './context'
import { embeddingLabel } from './operations'
import { shortcutLabel } from './shortcuts'
import {
  readSourceSelectionPreference,
  readWorkspacePreference,
  writeSourceSelectionPreference,
  writeWorkspacePreference,
} from './workspacePreference'
import {
  activeJobs,
  describeSourceJobProgress,
  sourceJobAttention,
  useSourceJobs,
} from './sourceJobs'
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
  DesktopHindsightStatus,
  DesktopHonchoStatus,
  DesktopReadiness,
  DesktopReadinessActivity,
  DesktopServiceActivity,
  DesktopServiceReport,
  DesktopSourceJob,
  DesktopUpdate,
  Evidence,
} from './types'

const STATUS_REFRESH_MS = 15_000
const INSTALLER_POLL_MS = 1_000
const MAX_DOCUMENT_QUERY_BYTES = 256
const textEncoder = new TextEncoder()

export function App() {
  const [query, setQuery] = useState('How do releases work?')
  const [activeQuery, setActiveQuery] = useState(query)
  const [status, setStatus] = useState<BrainStatus | null>(null)
  const [evidence, setEvidence] = useState<Evidence[]>([])
  const [answer, setAnswer] = useState<AnswerResponse | null>(null)
  const [selected, setSelected] = useState(0)
  const [source, setSource] = useState(() => (isDesktopApp ? readSourceSelectionPreference() : ''))
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [statusError, setStatusError] = useState('')
  const [leftOpen, setLeftOpen] = useState(false)
  const [rightOpen, setRightOpen] = useState(false)
  const [view, setView] = useState<AppView>('knowledge')
  const [workspace, setWorkspace] = useState(() => (isDesktopApp ? readWorkspacePreference() : ''))
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>('document')
  const [desktopSettings, setDesktopSettings] = useState<DesktopSettings | null>(null)
  const [desktopInfo, setDesktopInfo] = useState<DesktopInfo | null>(null)
  const [desktopServices, setDesktopServices] = useState<DesktopServiceReport | null>(null)
  const [desktopServicesError, setDesktopServicesError] = useState('')
  const [settingsSection, setSettingsSection] = useState<
    'readiness' | 'services' | 'updates' | 'sources' | 'hindsight' | 'honcho'
  >('readiness')
  const [settingsDirty, setSettingsDirty] = useState(false)
  const [installerJob, setInstallerJob] = useState<DesktopInstallJob | null>(null)
  const [desktopUpdate, setDesktopUpdate] = useState<DesktopUpdate | null>(null)
  const [desktopReadiness, setDesktopReadiness] = useState<DesktopReadiness | null>(null)
  const [readinessActivity, setReadinessActivity] = useState<DesktopReadinessActivity | null>(null)
  const [serviceActivity, setServiceActivity] = useState<DesktopServiceActivity | null>(null)
  const [sourceJobError, setSourceJobError] = useState('')
  const [sourceToggleBusy, setSourceToggleBusy] = useState<string | null>(null)
  const [sourceToggleError, setSourceToggleError] = useState('')
  const [sourceToggleNotice, setSourceToggleNotice] = useState('')
  const [hindsightStatus, setHindsightStatus] = useState<DesktopHindsightStatus | null>(null)
  const [honchoStatus, setHonchoStatus] = useState<DesktopHonchoStatus | null>(null)
  const [documents, setDocuments] = useState<BrainDocumentSummary[]>([])
  const [documentCursor, setDocumentCursor] = useState<string | null>(null)
  const [documentsLoading, setDocumentsLoading] = useState(true)
  const [documentsError, setDocumentsError] = useState('')
  const [documentRetryNonce, setDocumentRetryNonce] = useState(0)
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
  const [graphRetryNonce, setGraphRetryNonce] = useState(0)
  const [pageVisible, setPageVisible] = useState(
    () => typeof document === 'undefined' || document.visibilityState !== 'hidden'
  )
  const searchRequestRef = useRef(0)
  const [contextError, setContextError] = useState('')
  const [queryHistory, setQueryHistory] = useState<string[]>([])
  const [queryHistoryIndex, setQueryHistoryIndex] = useState(-1)
  const searchRef = useRef<HTMLInputElement>(null)
  const sourceJobs = useSourceJobs()
  const sourceJobsError = sourceJobError || sourceJobs.error
  const sourceJobsRetry = sourceJobError
    ? undefined
    : sourceJobs.error
      ? sourceJobs.retry
      : undefined
  const installerPollingRef = useRef(false)
  const installerStatusRef = useRef<DesktopInstallJob['status'] | null>(null)
  const updatePollingRef = useRef(false)
  const sidecarPollingRef = useRef(false)
  const servicesPollingRef = useRef(false)
  const desktopServicesRequestRef = useRef(0)
  const refreshedSourceJobsRef = useRef<Set<string>>(new Set())
  const documentScope = `${workspace}\u0000${source}\u0000${debouncedDocumentQuery}`
  const documentFetchReady = !isDesktopApp || desktopSettings?.needs_setup === false
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
  const statusRequestRef = useRef(0)
  const statusRefreshRef = useRef<(() => void) | null>(null)
  const documentPageLoadingRef = useRef(false)
  const sourceWidthRef = useRef(sourceWidth)
  const contextWidthRef = useRef(contextWidth)
  documentScopeRef.current = documentScope
  sourceWidthRef.current = sourceWidth
  contextWidthRef.current = contextWidth

  useEffect(() => {
    const visibility = { current: document.visibilityState !== 'hidden' }
    const focused = { current: true }
    const syncForeground = () => setPageVisible(visibility.current && focused.current)
    const handleVisibilityChange = () => {
      visibility.current = document.visibilityState !== 'hidden'
      syncForeground()
    }
    const handleFocus = () => {
      focused.current = true
      syncForeground()
    }
    const handleBlur = () => {
      focused.current = false
      syncForeground()
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)
    window.addEventListener('focus', handleFocus)
    window.addEventListener('blur', handleBlur)
    let disposed = false
    let unlistenFocus: (() => void) | undefined
    if (isDesktopApp && '__TAURI_INTERNALS__' in window) {
      void import('@tauri-apps/api/window')
        .then(({ getCurrentWindow }) => {
          const currentWindow = getCurrentWindow()
          void currentWindow
            .isFocused()
            .then((payload) => {
              if (!disposed) {
                focused.current = payload
                syncForeground()
              }
            })
            .catch(() => {
              // The browser visibility and focus events remain the fallback
              // when the native focus snapshot is unavailable at startup.
            })
          return currentWindow.onFocusChanged(({ payload }) => {
            if (!disposed) {
              focused.current = payload
              syncForeground()
            }
          })
        })
        .then((unlisten) => {
          if (disposed) unlisten()
          else unlistenFocus = unlisten
        })
        .catch(() => {
          // Browser visibility remains the fallback when a native focus
          // listener is unavailable during early Desktop startup.
        })
    }
    return () => {
      disposed = true
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      window.removeEventListener('focus', handleFocus)
      window.removeEventListener('blur', handleBlur)
      unlistenFocus?.()
    }
  }, [])

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
    const timeout = window.setTimeout(() => {
      setDebouncedDocumentQuery(boundDocumentQuery(documentQuery).trim())
    }, 250)
    return () => window.clearTimeout(timeout)
  }, [documentQuery])

  useEffect(() => {
    if (!pageVisible) return
    let disposed = false
    let initialRequest = true
    let controller: AbortController | null = null

    const refresh = () => {
      controller?.abort()
      const nextController = new AbortController()
      controller = nextController
      const requestId = ++statusRequestRef.current
      const isInitialRequest = initialRequest
      initialRequest = false
      void getStatus(nextController.signal)
        .then((next) => {
          if (disposed || nextController.signal.aborted || statusRequestRef.current !== requestId)
            return
          setStatus(next)
          setStatusError('')
        })
        .catch((caught: unknown) => {
          if (disposed || nextController.signal.aborted || statusRequestRef.current !== requestId)
            return
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
            statusRequestRef.current === requestId &&
            searchRequestRef.current === 0
          ) {
            setLoading(false)
          }
        })
    }

    statusRefreshRef.current = refresh
    refresh()
    const timer = window.setInterval(refresh, STATUS_REFRESH_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
      controller?.abort()
      statusRequestRef.current += 1
      if (statusRefreshRef.current === refresh) statusRefreshRef.current = null
    }
  }, [pageVisible])

  useEffect(() => {
    // Desktop settings are the control-plane gate for the document index. On
    // first launch the settings request can redirect the shell to setup; do
    // not query a half-configured backend (or surface a noisy error) before
    // the user has finished that flow. The Knowledge view is the only surface
    // that consumes this list, so avoid background reads while managing the
    // local runtime in Settings as well.
    if (view !== 'knowledge' || !documentFetchReady) {
      documentListRequestRef.current += 1
      documentListAbortRef.current?.abort()
      documentPageLoadingRef.current = false
      if (view !== 'knowledge') {
        // Settings and utility views do not consume the document list. Keep
        // the last Knowledge snapshot so returning to it feels continuous,
        // while the next Knowledge render still performs a fresh scoped read.
        setDocumentsLoading(false)
        return
      }
      setDocuments([])
      setDocumentCursor(null)
      setDocumentsError('')
      setActiveDocument(null)
      // Keep the Knowledge pane honest during the one transient state where
      // Desktop settings have not arrived yet. Once setup is known to be
      // required, or while Settings is open, there is no document request to
      // wait for and the empty state should be calm instead of spinning.
      setDocumentsLoading(view === 'knowledge' && isDesktopApp && desktopSettings === null)
      return
    }
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
  }, [debouncedDocumentQuery, documentFetchReady, documentRetryNonce, source, view, workspace])

  useEffect(() => {
    if (view !== 'knowledge' || workspaceTab !== 'graph' || !documentFetchReady) {
      graphRequestRef.current += 1
      graphAbortRef.current?.abort()
      setGraph(null)
      setGraphError('')
      setGraphLoading(false)
      return
    }
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
  }, [
    debouncedDocumentQuery,
    documentFetchReady,
    graphRetryNonce,
    source,
    view,
    workspace,
    workspaceTab,
  ])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const modifier = event.metaKey || event.ctrlKey
      const target = event.target as HTMLElement | null
      const editing =
        target?.isContentEditable === true ||
        target?.tagName === 'INPUT' ||
        target?.tagName === 'TEXTAREA' ||
        target?.tagName === 'SELECT'
      const key = event.key.toLowerCase()
      if (event.key === 'Escape') {
        setCommandPaletteOpen(false)
        setLeftOpen(false)
        setRightOpen(false)
        return
      }
      // Keep command/filter shortcuts out of text fields. Cmd/Ctrl+K remains
      // intentionally global because it is the app's primary search action.
      if (editing && !(modifier && key === 'k')) return
      if (modifier && key === 'k') {
        event.preventDefault()
        searchRef.current?.focus()
        searchRef.current?.select()
      } else if (modifier && key === 'p') {
        event.preventDefault()
        setCommandPaletteOpen((open) => !open)
      } else if (modifier && event.shiftKey && key === 'f') {
        event.preventDefault()
        setLeftOpen(true)
        window.setTimeout(() => document.getElementById('document-filter')?.focus(), 0)
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
    if (!isDesktopApp || !pageVisible) return
    let disposed = false
    const refresh = () => {
      if (disposed || servicesPollingRef.current) return
      servicesPollingRef.current = true
      const requestId = ++desktopServicesRequestRef.current
      void getDesktopServices()
        .then((next) => {
          if (disposed || desktopServicesRequestRef.current !== requestId) return
          setDesktopServices(next)
          setDesktopServicesError('')
        })
        .catch((caught: unknown) => {
          if (disposed || desktopServicesRequestRef.current !== requestId) return
          setDesktopServicesError(
            caught instanceof Error ? caught.message : 'Service status is unavailable'
          )
        })
        .finally(() => {
          servicesPollingRef.current = false
        })
    }
    refresh()
    const timer = window.setInterval(refresh, STATUS_REFRESH_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
      desktopServicesRequestRef.current += 1
    }
  }, [pageVisible])

  useEffect(() => {
    if (!isDesktopApp || !desktopSettings || !pageVisible) return
    let disposed = false
    const refresh = () => {
      if (disposed || sidecarPollingRef.current) return
      sidecarPollingRef.current = true
      void Promise.allSettled([getDesktopHindsightStatus(), getDesktopHonchoStatus()])
        .then(([hindsight, honcho]) => {
          if (disposed) return
          if (hindsight.status === 'fulfilled') {
            setHindsightStatus(hindsight.value)
          } else {
            setHindsightStatus({
              enabled: desktopSettings.hindsight.enabled,
              configured: false,
              reachable: false,
              state: 'unreachable',
              endpoint: desktopSettings.hindsight.base_url,
              bank: desktopSettings.hindsight.bank,
              token_configured: false,
              detail:
                hindsight.reason instanceof Error
                  ? hindsight.reason.message
                  : 'Hindsight status is unavailable',
            })
          }
          if (honcho.status === 'fulfilled') {
            setHonchoStatus(honcho.value)
          } else {
            setHonchoStatus({
              enabled: desktopSettings.honcho.enabled,
              configured: false,
              reachable: false,
              state: 'unreachable',
              endpoint: desktopSettings.honcho.base_url,
              workspace_id: desktopSettings.honcho.workspace_id,
              peer_id: desktopSettings.honcho.peer_id,
              token_configured: false,
              detail:
                honcho.reason instanceof Error
                  ? honcho.reason.message
                  : 'Honcho status is unavailable',
            })
          }
        })
        .finally(() => {
          sidecarPollingRef.current = false
        })
    }
    refresh()
    const timer = window.setInterval(refresh, STATUS_REFRESH_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [desktopSettings, pageVisible])

  useEffect(() => {
    if (!isDesktopApp) return
    const completed = sourceJobs.jobs.filter(
      (job) =>
        job.completed_at_unix_seconds !== null && !['running', 'cancelling'].includes(job.status)
    )
    const completedIds = new Set(completed.map((job) => job.id))
    for (const id of refreshedSourceJobsRef.current) {
      if (!completedIds.has(id)) refreshedSourceJobsRef.current.delete(id)
    }
    const unseen = completed.filter((job) => !refreshedSourceJobsRef.current.has(job.id))
    if (unseen.length === 0) return
    unseen.forEach((job) => refreshedSourceJobsRef.current.add(job.id))

    let active = true
    const requestId = ++statusRequestRef.current
    void getStatus()
      .then((next) => {
        if (!active || statusRequestRef.current !== requestId) return
        setStatus(next)
        setStatusError('')
      })
      .catch((caught: unknown) => {
        if (!active || statusRequestRef.current !== requestId) return
        setStatusError(caught instanceof Error ? caught.message : 'Status unavailable')
      })
    return () => {
      active = false
      if (statusRequestRef.current === requestId) statusRequestRef.current += 1
    }
  }, [sourceJobs.jobs])

  useEffect(() => {
    if (!isDesktopApp || !installerJob || !isActiveInstaller(installerJob) || !pageVisible) return
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
  }, [installerJob?.id, installerJob?.status, pageVisible])

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
      !['downloading', 'installing'].includes(desktopUpdate.phase) ||
      !pageVisible
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
  }, [desktopUpdate?.phase, pageVisible])

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

  function boundDocumentQuery(query: string) {
    if (textEncoder.encode(query).length <= MAX_DOCUMENT_QUERY_BYTES) {
      return query
    }

    const parts: string[] = []
    let bytes = 0
    for (const token of query) {
      const nextBytes = textEncoder.encode(token).length
      if (bytes + nextBytes > MAX_DOCUMENT_QUERY_BYTES) break
      bytes += nextBytes
      parts.push(token)
    }
    return parts.join('')
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

  async function toggleSource(nextSource: string, project: string, enabled: boolean) {
    const key = `${project}:${nextSource}`
    if (sourceToggleBusy) return
    if (!isDesktopApp || !desktopSettings) {
      setSettingsSection('sources')
      setView('settings')
      return
    }
    if (settingsDirty) {
      setSourceToggleError('Save or discard settings changes before toggling a source.')
      setSettingsSection('sources')
      setView('settings')
      return
    }
    const current = desktopSettings.sources.find(
      (candidate) => candidate.name === nextSource && candidate.project === project
    )
    if (!current || current.enabled === enabled) {
      setSourceToggleError(
        current ? '' : `${nextSource} is not present in the saved Desktop source configuration.`
      )
      return
    }
    if (
      !window.confirm(
        `${enabled ? 'Enable' : 'Disable'} ${nextSource} in ${project}?\n\n` +
          'This changes future ingestion only. Existing indexed data remains queryable and is not deleted.'
      )
    ) {
      return
    }
    setSourceToggleBusy(key)
    setSourceToggleError('')
    setSourceToggleNotice('')
    try {
      const next = await saveDesktopSettings({
        workspaces: desktopSettings.workspaces,
        sources: desktopSettings.sources.map((candidate) =>
          candidate === current ? { ...candidate, enabled } : candidate
        ),
        auth_principals: desktopSettings.auth_principals,
        embedding: desktopSettings.embedding,
        query: desktopSettings.query,
        hindsight: desktopSettings.hindsight,
        honcho: desktopSettings.honcho,
        ingestion: desktopSettings.ingestion,
        runtime: desktopSettings.runtime,
        secrets: [],
      })
      setDesktopSettings(next)
      setSourceToggleNotice(
        next.restart_required
          ? 'Source setting saved. Restart the affected services from Settings to apply it.'
          : 'Source setting saved for future ingestion.'
      )
      try {
        const nextStatus = await getStatus()
        setStatus(nextStatus)
        setStatusError('')
      } catch (caught) {
        // The source setting is already persisted. Keep that success visible
        // and report only the best-effort health refresh failure.
        setStatusError(caught instanceof Error ? caught.message : 'Source status refresh failed')
      }
    } catch (caught) {
      setSourceToggleError(
        caught instanceof Error ? caught.message : 'Source setting could not be saved'
      )
    } finally {
      setSourceToggleBusy(null)
    }
  }

  async function openSourceSetup(sourceName: string, project: string) {
    if (sourceToggleBusy) return
    if (!isDesktopApp || !desktopSettings || desktopSettings.needs_setup || settingsDirty) {
      setSettingsSection('sources')
      setView('settings')
      return
    }
    const configuredSource = desktopSettings.sources.find(
      (source) => source.name === sourceName && source.project === project
    )
    if (!configuredSource) {
      setSourceToggleError(
        `${sourceName} in ${project} is not present in the saved Desktop source configuration.`
      )
      return
    }
    if (
      configuredSource &&
      ['google-drive', 'gmail', 'google-calendar'].includes(configuredSource.kind)
    ) {
      // Google authorization needs both an OAuth client and a writable token
      // destination. Keep incomplete setup in the typed source editor instead
      // of opening a provider URL that cannot fix the saved configuration.
      setSettingsSection('sources')
      setView('settings')
      return
    }
    setSourceToggleBusy(`setup:${sourceName}`)
    setSourceToggleError('')
    setSourceToggleNotice('')
    try {
      await openDesktopSourceSetup(sourceName)
      setSourceToggleNotice('Provider setup opened in your browser.')
    } catch (caught) {
      setSourceToggleError(
        caught instanceof Error ? caught.message : 'Provider setup could not open'
      )
    } finally {
      setSourceToggleBusy(null)
    }
  }

  async function authorizeSource(sourceName: string, project: string) {
    if (sourceToggleBusy) return
    if (!isDesktopApp || !desktopSettings || desktopSettings.needs_setup || settingsDirty) {
      setSettingsSection('sources')
      setView('settings')
      return
    }
    const configuredSource = desktopSettings.sources.find(
      (source) => source.name === sourceName && source.project === project
    )
    if (!configuredSource) {
      setSourceToggleError(
        `${sourceName} in ${project} is not present in the saved Desktop source configuration.`
      )
      return
    }
    if (
      !window.confirm(
        `Authorize ${sourceName} in ${project} with Google?\n\n` +
          'Cortana will open the system browser and store the read-only token in the configured private file.'
      )
    ) {
      return
    }
    setSourceToggleBusy(`authorize:${sourceName}`)
    setSourceToggleError('')
    setSourceToggleNotice('')
    try {
      const job = await startDesktopSourceAuthorization(sourceName)
      sourceJobs.remember(job)
      setSourceToggleNotice('Google authorization opened in your browser.')
    } catch (caught) {
      setSourceToggleError(
        caught instanceof Error ? caught.message : 'Google authorization could not start'
      )
    } finally {
      setSourceToggleBusy(null)
    }
  }

  function chooseSource(next: string, project?: string) {
    const nextWorkspace = project ?? workspace
    const sameScope = source === next && workspace === nextWorkspace
    const nextSource = sameScope ? '' : next
    abortSearchRequest()
    abortContextRequest()
    clearScopedResults()
    scopeSources(nextWorkspace, nextSource)
    if (project) {
      setWorkspace(project)
      if (isDesktopApp) {
        writeWorkspacePreference(project)
        writeSourceSelectionPreference(nextSource)
      }
    }
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
    if (isDesktopApp) {
      writeWorkspacePreference(nextWorkspace)
      writeSourceSelectionPreference(nextSource)
    }
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
    setSourceJobError('')
    void cancelDesktopSourceValidation(id)
      .then(sourceJobs.remember)
      .then(() => setSourceJobError(''))
      .catch((caught: unknown) => {
        setSourceJobError(
          caught instanceof Error ? caught.message : 'Source job cancellation failed'
        )
      })
  }

  const retryStatus = useCallback(() => {
    statusRefreshRef.current?.()
  }, [])

  const retryDocuments = useCallback(() => {
    setDocumentsError('')
    setDocumentRetryNonce((current) => current + 1)
  }, [])

  function openGraph() {
    if (!canLeaveSettings()) return
    setView('knowledge')
    setWorkspaceTab('graph')
  }

  function retryGraph() {
    setGraphRetryNonce((current) => current + 1)
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

  const configuredSourcesForWorkspace = workspace
    ? Array.from(
        new Set([
          ...(desktopSettings?.sources ?? [])
            .filter((item) => item.project === workspace)
            .map((item) => item.name),
          ...(status?.ingestion.configured_sources ?? [])
            .filter((item) => item.project === workspace)
            .map((item) => item.source),
          ...(status?.sources ?? [])
            .filter((item) => item.project === workspace)
            .map((item) => item.source),
        ])
      )
    : []
  // Settings may arrive before the runtime status call. An empty settings
  // source list is not enough evidence to evict a persisted source because
  // the runtime may still report configured/indexed sources shortly after
  // launch. Treat the inventory as authoritative once status is available,
  // or once non-empty saved source settings are present.
  const sourceInventoryReady = status !== null || (desktopSettings?.sources.length ?? 0) > 0
  const desktopSourceActionsReady =
    isDesktopApp &&
    desktopSettings !== null &&
    !desktopSettings.needs_setup &&
    desktopSettings.sources.length > 0

  const workspaceScope = workspaces.map((item) => item.id).join('\u0000')
  useEffect(() => {
    if (!workspace || !workspaceScope) return
    if (workspaces.some((item) => item.id === workspace)) return
    chooseWorkspace('')
  }, [workspace, workspaceScope])

  useEffect(() => {
    if (!isDesktopApp || !source) return
    // Source selection is a workspace-scoped filter. The public "All
    // workspaces" scope intentionally has no selected source, so a stale
    // preference from a prior workspace must not create an invisible filter
    // that the source tree cannot highlight or clear.
    if (!workspace) {
      writeSourceSelectionPreference('')
      setSource('')
      scopeSources('', '')
      return
    }
    if (!sourceInventoryReady || configuredSourcesForWorkspace.includes(source)) return
    writeSourceSelectionPreference('')
    setSource('')
    scopeSources(workspace, '')
  }, [workspace, source, sourceInventoryReady, configuredSourcesForWorkspace.join('\u0000')])

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
          {loading ? (
            <LoaderCircle className="spin" size={16} />
          ) : (
            <kbd>{shortcutLabel('MOD K')}</kbd>
          )}
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
          desktopSettings={desktopSettings ?? undefined}
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
          desktopUpdate={desktopUpdate ?? undefined}
          onDesktopUpdate={setDesktopUpdate}
          services={desktopServices}
          onServices={setDesktopServices}
          servicesError={desktopServicesError}
          onServicesError={setDesktopServicesError}
          desktopInfo={desktopInfo}
          onDesktopInfo={setDesktopInfo}
          serviceActivity={serviceActivity}
          onServiceActivity={setServiceActivity}
          hindsightStatus={hindsightStatus}
          onHindsightStatus={setHindsightStatus}
          honchoStatus={honchoStatus}
          onHonchoStatus={setHonchoStatus}
          onSaved={(next) => {
            setDesktopSettings(next)
            setSettingsDirty(false)
            setHindsightStatus(null)
            setHonchoStatus(null)
            // A settings save can change the configured embedding/runtime
            // services. Refresh the shell-owned snapshots immediately rather
            // than waiting for the next 15-second health tick.
            const servicesRequestId = ++desktopServicesRequestRef.current
            void getDesktopServices()
              .then((nextServices) => {
                if (desktopServicesRequestRef.current !== servicesRequestId) return
                setDesktopServices(nextServices)
                setDesktopServicesError('')
              })
              .catch((caught: unknown) => {
                if (desktopServicesRequestRef.current !== servicesRequestId) return
                setDesktopServicesError(
                  caught instanceof Error ? caught.message : 'Service status is unavailable'
                )
              })
            void getDesktopInfo()
              .then(setDesktopInfo)
              .catch(() => {
                // Keep the previous metadata snapshot when the refresh is
                // unavailable; the Services panel can retry explicitly.
              })
            const statusRequestId = ++statusRequestRef.current
            void getStatus()
              .then((nextStatus) => {
                if (statusRequestRef.current !== statusRequestId) return
                setStatus(nextStatus)
                setStatusError('')
              })
              .catch(() => {
                if (statusRequestRef.current !== statusRequestId) return
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
              if (isDesktopApp) writeSourceSelectionPreference('')
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
            onRetryStatus={retryStatus}
            sourceJobError={sourceJobsError}
            onRetrySourceJobs={sourceJobsRetry}
            onSelect={chooseSource}
            onSelectWorkspace={chooseWorkspace}
            onDocumentQueryChange={setDocumentQuery}
            onSelectDocument={(id) => void chooseDocument(id)}
            onLoadMoreDocuments={() => void loadMoreDocuments()}
            onRetryDocuments={retryDocuments}
            onOpenSourcesSettings={() => {
              setSettingsSection('sources')
              setView('settings')
            }}
            onOpenSourceSetup={
              desktopSourceActionsReady
                ? (name, project) => void openSourceSetup(name, project)
                : undefined
            }
            onAuthorizeSource={
              desktopSourceActionsReady
                ? (name, project) => void authorizeSource(name, project)
                : undefined
            }
            onToggleSource={
              desktopSourceActionsReady
                ? (name, project, enabled) => void toggleSource(name, project, enabled)
                : undefined
            }
            sourceToggleBusy={sourceToggleBusy}
            sourceToggleDisabled={
              settingsDirty || desktopSettings === null || Boolean(desktopSettings?.needs_setup)
            }
            sourceToggleError={sourceToggleError}
            sourceToggleNotice={sourceToggleNotice}
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
            onRetryGraph={retryGraph}
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
          statusError={statusError}
          onRetryStatus={retryStatus}
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
          sourceJobError={sourceJobsError}
          onRetrySourceJobs={sourceJobsRetry}
          onSearchFocus={focusSearch}
          onRetrieveContext={() => void retrieveAgentContext()}
          onOpenSettings={() => setView('settings')}
          onOpenProject={() => openDesktopProject()}
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
              type="button"
              autoFocus
              onClick={() => {
                setCommandPaletteOpen(false)
                focusSearch()
              }}
            >
              Search the brain <kbd>{shortcutLabel('MOD K')}</kbd>
            </button>
            <button
              type="button"
              onClick={() => {
                setCommandPaletteOpen(false)
                focusDocumentFilter()
              }}
            >
              Filter documents <kbd>{shortcutLabel('MOD ⇧ F')}</kbd>
            </button>
            <button
              type="button"
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
                type="button"
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
              type="button"
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
          <FileText size={13} /> Docs: {status ? status.documents.toLocaleString() : '—'}
        </span>
        <IngestionIndicator status={status} />
        <ActiveSourceJobs
          jobs={sourceJobs.jobs}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setView('inbox')
          }}
        />
        <SourceJobsErrorIndicator
          error={sourceJobsError}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setView('inbox')
          }}
        />
        <SourceJobAttentionIndicator
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
        <ServiceHealthIndicator
          report={desktopServices}
          error={desktopServicesError}
          embeddingRequired={desktopSettings?.embedding.provider !== 'cloud'}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('services')
            setView('settings')
          }}
        />
        <SidecarStatusIndicator
          label="Hindsight"
          status={hindsightStatus}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('hindsight')
            setView('settings')
          }}
        />
        <SidecarStatusIndicator
          label="Honcho"
          status={honchoStatus}
          onOpen={() => {
            if (!canLeaveSettings()) return
            setSettingsSection('honcho')
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
              if (!canLeaveSettings()) return
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
  if (!status) {
    return (
      <span className="ingestion-health">
        <i /> Ingestion: checking
      </span>
    )
  }
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
    .map(
      (job) =>
        `${job.project} · ${job.source} · ${job.operation} · ${describeSourceJobProgress(job)}`
    )
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

function SourceJobsErrorIndicator({ error, onOpen }: { error: string; onOpen: () => void }) {
  const detail = error.replace(/\s+/g, ' ').trim()
  if (!detail) return null
  const label = detail.length > 160 ? `${detail.slice(0, 157)}…` : detail
  return (
    <button
      type="button"
      className="source-jobs status-link attention"
      aria-label="Open source job status"
      title={`${detail}. Open the activity inbox.`}
      onClick={onOpen}
    >
      <i /> Source jobs: {label}
    </button>
  )
}

function SourceJobAttentionIndicator({
  jobs,
  onOpen,
}: {
  jobs: DesktopSourceJob[]
  onOpen: () => void
}) {
  const attention = sourceJobAttention(jobs)
  if (attention.length === 0) return null
  const detail = attention.map((job) => `${job.project} · ${job.source} · ${job.status}`).join(', ')
  return (
    <button
      type="button"
      className="source-jobs status-link attention"
      aria-label="Open source job attention"
      title={`${detail}. Open the activity inbox.`}
      onClick={onOpen}
    >
      <i /> {attention.length} source job{attention.length === 1 ? '' : 's'} need attention
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

export function ServiceHealthIndicator({
  report,
  error,
  embeddingRequired = true,
  onOpen,
}: {
  report: DesktopServiceReport | null
  error: string
  embeddingRequired?: boolean
  onOpen: () => void
}) {
  if (error) {
    return (
      <button
        type="button"
        className="service-activity-health warning"
        aria-label="Open service health"
        title={`${error}. Open Services for details.`}
        onClick={onOpen}
      >
        <i /> Services: unavailable
      </button>
    )
  }
  if (!report) return null
  const core = report.services.filter(
    (service) => service.name === 'server' || (service.name === 'embedding' && embeddingRequired)
  )
  const coreLoaded = core.filter((service) => service.loaded).length
  const coreExitFailure = core.some(
    (service) => service.last_exit_status !== null && service.last_exit_status !== 0
  )
  const coreAttention = core.length === 0 || coreLoaded < core.length || coreExitFailure
  const state = !report.supported || coreAttention ? 'warning' : 'healthy'
  const detail = report.services
    .map(
      (service) =>
        `${service.name}: ${service.state || (service.installed ? 'installed' : 'not installed')}`
    )
    .join(' · ')
  const label = !report.supported
    ? `Services: unsupported on ${report.platform}`
    : coreAttention
      ? 'Services: core attention'
      : `Services: core ${coreLoaded}/${core.length} online`
  return (
    <button
      type="button"
      className={`service-activity-health ${state}`}
      aria-label="Open service health"
      title={`${detail}. Open Services for controls.`}
      onClick={onOpen}
    >
      <i /> {label}
    </button>
  )
}

function SidecarStatusIndicator({
  label,
  status,
  onOpen,
}: {
  label: string
  status: { state: string; detail: string | null } | null
  onOpen: () => void
}) {
  if (!status) return null
  const state =
    status.state === 'healthy'
      ? 'healthy'
      : status.state === 'disabled'
        ? 'disabled'
        : status.state === 'reachable'
          ? 'reachable'
          : 'warning'
  const readable = status.state.replaceAll('_', ' ')
  return (
    <button
      type="button"
      className={`sidecar-health ${state}`}
      aria-label={`Open ${label} status`}
      title={`${label}: ${readable}${status.detail ? ` — ${status.detail}` : ''}`}
      onClick={onOpen}
    >
      <i /> {label}: {readable}
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
