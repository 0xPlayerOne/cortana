import { invoke, isTauri } from '@tauri-apps/api/core'

import { demoEvidence, demoStatus } from './demo'
import { buildAgentContext, estimateTokens } from './context'
import type {
  AnswerResponse,
  BrainDocument,
  BrainDocumentPage,
  BrainGraphPage,
  BrainStatus,
  ContextBundle,
  DesktopInitialSyncOutcome,
  DesktopInitialSyncPlan,
  DesktopInstallJob,
  DesktopInfo,
  DesktopHindsightStatus,
  DesktopReadiness,
  DesktopServiceReport,
  DesktopSettings,
  DesktopSettingsExport,
  DesktopSettingsImport,
  DesktopSettingsUpdate,
  DesktopSourceJob,
  DesktopUpdate,
  AuditEvent,
  DesktopSetupOpen,
  InitialSyncBudget,
} from './types'

export const isDemoMode = new URLSearchParams(window.location.search).has('demo')
export const isDesktopApp = isTauri()

let tokenPromptInFlight: Promise<string | null> | null = null

export async function getDesktopSettings(): Promise<DesktopSettings> {
  if (!isDesktopApp) throw new Error('Settings are available in Cortana Desktop')
  return invokeDesktop<DesktopSettings>('desktop_settings_get')
}

export async function saveDesktopSettings(update: DesktopSettingsUpdate): Promise<DesktopSettings> {
  if (!isDesktopApp) throw new Error('Settings are available in Cortana Desktop')
  return invokeDesktop<DesktopSettings>('desktop_settings_save', { update })
}

export async function exportDesktopSettings(): Promise<DesktopSettingsExport | null> {
  if (!isDesktopApp) throw new Error('Settings export is available in Cortana Desktop')
  return invokeDesktop<DesktopSettingsExport | null>('desktop_settings_export')
}

export async function importDesktopSettings(): Promise<DesktopSettingsImport | null> {
  if (!isDesktopApp) throw new Error('Settings import is available in Cortana Desktop')
  return invokeDesktop<DesktopSettingsImport | null>('desktop_settings_import')
}

export async function scanDesktopReadiness(): Promise<DesktopReadiness> {
  if (!isDesktopApp) throw new Error('Readiness is available in Cortana Desktop')
  return invokeDesktop<DesktopReadiness>('desktop_readiness_scan')
}

export async function getDesktopInfo(): Promise<DesktopInfo> {
  if (!isDesktopApp) throw new Error('Desktop information is available in Cortana Desktop')
  return invokeDesktop<DesktopInfo>('desktop_info')
}

export async function setDesktopAutostart(enabled: boolean): Promise<DesktopInfo> {
  if (!isDesktopApp) throw new Error('Desktop autostart is available in Cortana Desktop')
  return invokeDesktop<DesktopInfo>('desktop_autostart_set', { enabled })
}

export async function getDesktopServices(): Promise<DesktopServiceReport> {
  if (!isDesktopApp) throw new Error('Service status is available in Cortana Desktop')
  return invokeDesktop<DesktopServiceReport>('desktop_services_status')
}

export async function installDesktopServices(): Promise<DesktopServiceReport> {
  if (!isDesktopApp) throw new Error('Service installation is available in Cortana Desktop')
  return invokeDesktop<DesktopServiceReport>('desktop_services_install', { approved: true })
}

export async function getDesktopHindsightStatus(): Promise<DesktopHindsightStatus> {
  if (!isDesktopApp) throw new Error('Hindsight status is available in Cortana Desktop')
  return invokeDesktop<DesktopHindsightStatus>('desktop_hindsight_status')
}

export async function runDesktopServiceAction(
  service: DesktopServiceReport['services'][number]['name'],
  action: 'start' | 'stop' | 'restart'
): Promise<DesktopServiceReport> {
  if (!isDesktopApp) throw new Error('Service control is available in Cortana Desktop')
  return invokeDesktop<DesktopServiceReport>('desktop_service_action', {
    service,
    action,
    approved: true,
  })
}

export async function runDesktopServicesActionAll(
  action: 'start' | 'stop' | 'restart'
): Promise<DesktopServiceReport> {
  if (!isDesktopApp) throw new Error('Service control is available in Cortana Desktop')
  return invokeDesktop<DesktopServiceReport>('desktop_services_action_all', {
    action,
    approved: true,
  })
}

export async function getDesktopUpdate(): Promise<DesktopUpdate> {
  if (!isDesktopApp) throw new Error('Updates are available in Cortana Desktop')
  return invokeDesktop<DesktopUpdate>('desktop_update_status')
}

export async function checkDesktopUpdate(): Promise<DesktopUpdate> {
  if (!isDesktopApp) throw new Error('Updates are available in Cortana Desktop')
  return invokeDesktop<DesktopUpdate>('desktop_update_check')
}

export async function installDesktopUpdate(
  expectedVersion: string,
  restart: boolean
): Promise<DesktopUpdate> {
  if (!isDesktopApp) throw new Error('Updates are available in Cortana Desktop')
  return invokeDesktop<DesktopUpdate>('desktop_update_install', {
    expectedVersion,
    approved: true,
    restart,
  })
}

export async function getRuntimeAudit(limit = 100): Promise<AuditEvent[]> {
  if (!isDesktopApp) throw new Error('Audit is available in Cortana Desktop')
  return invokeDesktop<AuditEvent[]>('brain_audit', { limit })
}

export async function getDesktopAudit(limit = 100): Promise<AuditEvent[]> {
  if (!isDesktopApp) throw new Error('Audit is available in Cortana Desktop')
  return invokeDesktop<AuditEvent[]>('desktop_audit', { limit })
}

export async function openDesktopProject(): Promise<void> {
  if (!isDesktopApp) throw new Error('Project links are available in Cortana Desktop')
  return invokeDesktop<void>('desktop_project_open')
}

export async function openDesktopUrl(url: string): Promise<void> {
  if (!isDesktopApp) throw new Error('Desktop URL opens are available in Cortana Desktop')
  const parsed = new URL(url)
  const allowed = ['http:', 'https:', 'mailto:', 'file:']
  if (!allowed.includes(parsed.protocol)) {
    throw new Error(`Unsupported link scheme: ${parsed.protocol.replace(':', '')}`)
  }
  return invokeDesktop<void>('desktop_url_open', { url })
}

export async function startDesktopInstaller(tool: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_start', { tool, approved: true })
}

export async function getDesktopInstaller(id: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_status', { id })
}

export async function cancelDesktopInstaller(id: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_cancel', { id })
}

export async function startDesktopSourceValidation(
  source: string,
  budget?: InitialSyncBudget
): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Source validation is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>(
    'desktop_source_validation_start',
    budget ? { source, budget } : { source }
  )
}

export async function startDesktopSourceAuthorization(source: string): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Source authorization is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>('desktop_source_authorization_start', { source })
}

export async function startDesktopSourceTrialSync(source: string): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Trial sync is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>('desktop_source_trial_sync_start', {
    source,
    approved: true,
  })
}

export async function openDesktopSourceSetup(source: string): Promise<DesktopSetupOpen> {
  if (!isDesktopApp) throw new Error('Source setup is available in Cortana Desktop')
  return invokeDesktop<DesktopSetupOpen>('desktop_source_setup_open', { source })
}

export async function pickDesktopPath(
  kind: 'directory' | 'oauth-client' | 'google-token'
): Promise<string | null> {
  if (!isDesktopApp) throw new Error('Native path selection is available in Cortana Desktop')
  return invokeDesktop<string | null>('desktop_path_pick', { kind })
}

export async function getDesktopSourceValidation(id: string): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Source validation is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>('desktop_source_validation_status', { id })
}

export async function cancelDesktopSourceValidation(id: string): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Source validation is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>('desktop_source_validation_cancel', { id })
}

export async function planDesktopInitialSync(
  source: string,
  budget: InitialSyncBudget
): Promise<DesktopInitialSyncPlan> {
  if (!isDesktopApp) throw new Error('Initial sync is available in Cortana Desktop')
  const outcome = await invokeDesktop<DesktopInitialSyncOutcome>('desktop_source_initial_sync', {
    source,
    budget,
    operation: 'plan',
    planId: '',
    approved: false,
  })
  if (outcome.outcome !== 'plan') {
    throw new Error('Initial sync plan request returned an unexpected result')
  }
  return outcome
}

export async function startDesktopInitialSync(
  source: string,
  budget: InitialSyncBudget,
  planId: string
): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Initial sync is available in Cortana Desktop')
  const outcome = await invokeDesktop<DesktopInitialSyncOutcome>('desktop_source_initial_sync', {
    source,
    budget,
    operation: 'execute',
    planId,
    approved: true,
  })
  if (outcome.outcome !== 'job') {
    throw new Error('Initial sync execution returned an unexpected result')
  }
  return outcome
}

export async function getStatus(signal?: AbortSignal): Promise<BrainStatus> {
  if (isDemoMode) return demoStatus
  if (isTauri()) return invokeDesktop<BrainStatus>('brain_status', undefined, signal)
  const response = await authorizedFetch('/v1/status', { signal })
  if (!response.ok) throw new Error(`Status request failed (${response.status})`)
  return (await response.json()) as BrainStatus
}

export async function getContext(
  query: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<ContextBundle> {
  if (isDemoMode) {
    const evidence = demoEvidence
      .filter((item) => !source || item.source === source)
      .sort((left, right) => right.score - left.score)
    const context = buildAgentContext(query, evidence)
    return {
      query,
      context,
      evidence,
      metrics: {
        retrieved: evidence.length,
        included: evidence.length,
        omitted: 0,
        estimated_tokens: estimateTokens(context),
        max_tokens: 8000,
      },
    }
  }
  if (isTauri()) {
    return invokeDesktop<ContextBundle>(
      'brain_context',
      {
        request: {
          query,
          project: project || null,
          source: source || null,
        },
      },
      signal
    )
  }
  const response = await authorizedFetch('/v1/context', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      project: project || null,
      source: source || null,
      limit: 20,
      max_tokens: 8000,
    }),
    signal,
  })
  if (!response.ok) throw new Error(`Context retrieval failed (${response.status})`)
  return (await response.json()) as ContextBundle
}

export async function getDocuments(
  project?: string,
  source?: string,
  query?: string,
  cursor?: string,
  signal?: AbortSignal
): Promise<BrainDocumentPage> {
  if (isDemoMode) {
    const documents = demoEvidence
      .filter((item) => !source || item.source === source)
      .map((item) => ({
        id: item.chunk_id.replace(/[^a-f0-9]/gi, '').padEnd(16, '0'),
        source: item.source,
        source_id: item.source_id,
        title: item.title,
        uri: item.uri,
        updated_at: item.updated_at,
        project: project || 'demo',
        chunk_count: 1,
        content_chars: item.content.length,
      }))
    return { documents, next_cursor: null }
  }
  if (isTauri()) {
    return invokeDesktop<BrainDocumentPage>(
      'brain_documents',
      {
        request: {
          project: project || null,
          source: source || null,
          query: query || null,
          cursor: cursor || null,
          limit: 50,
        },
      },
      signal
    )
  }
  const params = new URLSearchParams({ limit: '50' })
  if (project) params.set('project', project)
  if (source) params.set('source', source)
  if (query) params.set('query', query)
  if (cursor) params.set('cursor', cursor)
  const response = await authorizedFetch(`/v1/documents?${params}`, { signal })
  if (!response.ok) throw new Error(`Document list failed (${response.status})`)
  return (await response.json()) as BrainDocumentPage
}

export async function getDocument(id: string, signal?: AbortSignal): Promise<BrainDocument> {
  if (isDemoMode) {
    const item = demoEvidence.find(
      (candidate) => candidate.chunk_id.replace(/[^a-f0-9]/gi, '').padEnd(16, '0') === id
    )
    if (!item) throw new Error('Document not found')
    return {
      id,
      source: item.source,
      source_id: item.source_id,
      title: item.title,
      uri: item.uri,
      updated_at: item.updated_at,
      project: 'demo',
      chunk_count: 1,
      content_chars: item.content.length,
      content: item.content,
      metadata: {},
      acl: [],
      backlinks: [],
      surrounding: [],
      truncated: false,
    }
  }
  if (isTauri()) {
    return invokeDesktop<BrainDocument>('brain_document', { id }, signal)
  }
  const response = await authorizedFetch(`/v1/documents/${encodeURIComponent(id)}`, { signal })
  if (!response.ok) throw new Error(`Document read failed (${response.status})`)
  return (await response.json()) as BrainDocument
}

export async function getGraph(
  project?: string,
  source?: string,
  query?: string,
  cursor?: string,
  signal?: AbortSignal
): Promise<BrainGraphPage> {
  if (isDemoMode) {
    const page = await getDocuments(project, source, query, cursor, signal)
    const nodes: BrainGraphPage['nodes'] = []
    const edges: BrainGraphPage['edges'] = []
    const seen = new Set<string>()
    for (const document of page.documents) {
      const workspaceId = `workspace:${JSON.stringify([document.project])}`
      const sourceId = `source:${JSON.stringify([document.project, document.source])}`
      if (!seen.has(workspaceId)) {
        seen.add(workspaceId)
        nodes.push({
          id: workspaceId,
          kind: 'workspace',
          label: document.project,
          project: document.project,
          source: null,
          document_id: null,
        })
      }
      if (!seen.has(sourceId)) {
        seen.add(sourceId)
        nodes.push({
          id: sourceId,
          kind: 'source',
          label: document.source,
          project: document.project,
          source: document.source,
          document_id: null,
        })
        edges.push({ source: workspaceId, target: sourceId, kind: 'contains' })
      }
      const documentId = `document:${document.id}`
      nodes.push({
        id: documentId,
        kind: 'document',
        label: document.title,
        project: document.project,
        source: document.source,
        document_id: document.id,
      })
      edges.push({ source: sourceId, target: documentId, kind: 'contains' })
    }
    return { nodes, edges, next_cursor: page.next_cursor }
  }
  const request = {
    project: project || null,
    source: source || null,
    query: query || null,
    cursor: cursor || null,
    limit: 100,
  }
  if (isTauri()) {
    return invokeDesktop<BrainGraphPage>('brain_graph', { request }, signal)
  }
  const params = new URLSearchParams({ limit: '100' })
  if (project) params.set('project', project)
  if (source) params.set('source', source)
  if (query) params.set('query', query)
  if (cursor) params.set('cursor', cursor)
  const response = await authorizedFetch(`/v1/graph?${params}`, { signal })
  if (!response.ok) throw new Error(`Graph data failed (${response.status})`)
  return (await response.json()) as BrainGraphPage
}

export async function getAnswer(
  query: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<AnswerResponse> {
  if (isDemoMode) {
    const evidence = demoEvidence
      .filter((item) => !source || item.source === source)
      .sort((left, right) => right.score - left.score)
    return {
      query,
      answer:
        'Promote short-lived changes through staging after the full test and security suite passes, then monitor the release and roll back if health regresses [1]. Keep an explicit rollback owner in the checklist [2].',
      evidence,
      plan: {
        queries: [query, 'release promotion staging checks', 'rollback owner health regression'],
        model_generated: true,
      },
      mode: 'synthesized',
      cached: false,
      latency_ms: 184,
      warnings: [],
    }
  }
  if (isTauri()) {
    return invokeDesktop<AnswerResponse>(
      'brain_answer',
      {
        request: {
          query,
          project: project || null,
          source: source || null,
        },
      },
      signal
    )
  }
  const response = await authorizedFetch('/v1/answer', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      project: project || null,
      source: source || null,
    }),
    signal,
  })
  if (!response.ok) throw new Error(`Answer request failed (${response.status})`)
  return (await response.json()) as AnswerResponse
}

async function invokeDesktop<T>(
  command: string,
  args?: Record<string, unknown>,
  signal?: AbortSignal
): Promise<T> {
  if (signal?.aborted) throw new DOMException('Request aborted', 'AbortError')
  try {
    const result = await invoke<T>(command, args)
    if (signal?.aborted) throw new DOMException('Request aborted', 'AbortError')
    return result
  } catch (caught) {
    if (caught instanceof Error) throw caught
    throw new Error(typeof caught === 'string' ? caught : 'Desktop request failed')
  }
}

async function authorizedFetch(input: string, init: RequestInit): Promise<Response> {
  const request = (token: string | null) => {
    const headers = new Headers(init.headers)
    if (token) headers.set('Authorization', `Bearer ${token}`)
    return fetch(input, { ...init, headers })
  }
  const current = window.sessionStorage.getItem('cortana_api_token')
  let response = await request(current)
  if (response.status !== 401) return response

  // A cold web shell starts status, document, and graph requests together.
  // Reuse one prompt for that burst instead of opening several modal dialogs.
  if (init.signal?.aborted) return response
  const token = await requestAccessToken()
  if (!token) return response

  window.sessionStorage.removeItem('cortana_api_token')
  window.sessionStorage.setItem('cortana_api_token', token)
  response = await request(token)
  if (response.status === 401) window.sessionStorage.removeItem('cortana_api_token')
  return response
}

function requestAccessToken(): Promise<string | null> {
  if (!tokenPromptInFlight) {
    tokenPromptInFlight = Promise.resolve(window.prompt('Enter the Cortana access token'))
      .then((token) => token?.trim() || null)
      .finally(() => {
        tokenPromptInFlight = null
      })
  }
  return tokenPromptInFlight
}
