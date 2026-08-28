import { invoke, isTauri } from '@tauri-apps/api/core'

import {
  demoCanonicalMemories,
  demoDerivedMemories,
  demoEvidence,
  demoMemoryCandidates,
  demoMemoryClassification,
  demoStatus,
} from './demo'
import { buildAgentContext, estimateTokens } from './context'
import { safeSourceLink } from './sourceLinks'
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
  DesktopReadiness,
  DesktopServiceReport,
  DesktopDatabaseActionResult,
  DesktopSchedule,
  DesktopSettings,
  DesktopSettingsExport,
  DesktopSettingsImport,
  DesktopSettingsUpdate,
  DesktopSecretStorageMigration,
  DesktopSourceJob,
  DesktopUpdate,
  AuditEvent,
  DesktopSetupOpen,
  InitialSyncBudget,
  GithubRepositoryList,
  DiscordChannelList,
  DiscordServerList,
  SlackWorkspaceList,
  BuzzCommunityList,
  ProviderModelKind,
  ProviderModelList,
  ReflectResponse,
  AgentMemory,
  DerivedMemoryResponse,
  MemoryCandidate,
  MemoryCandidateActionResult,
  MemoryCandidateClassification,
  MemoryReviewPolicy,
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

export async function migrateDesktopSecrets(): Promise<DesktopSecretStorageMigration> {
  if (!isDesktopApp) throw new Error('Secure-storage migration is available in Cortana Desktop')
  return invokeDesktop<DesktopSecretStorageMigration>('desktop_secret_storage_migrate', {
    approved: true,
  })
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

export async function migrateDesktopEmbeddingGeneration(from: string): Promise<string> {
  if (!isDesktopApp)
    throw new Error('Embedding generation migration is available in Cortana Desktop')
  return invokeDesktop<string>('desktop_embedding_generation_migrate', { from, approved: true })
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

export async function installDesktopSyncService(): Promise<DesktopServiceReport> {
  if (!isDesktopApp) throw new Error('Recurring sync installation is available in Cortana Desktop')
  return invokeDesktop<DesktopServiceReport>('desktop_services_install_sync', { approved: true })
}

export async function getDesktopSchedule(): Promise<DesktopSchedule> {
  if (!isDesktopApp) throw new Error('Service scheduling is available in Cortana Desktop')
  return invokeDesktop<DesktopSchedule>('desktop_schedule_get')
}

export async function saveDesktopSchedule(schedule: DesktopSchedule): Promise<DesktopSchedule> {
  if (!isDesktopApp) throw new Error('Service scheduling is available in Cortana Desktop')
  return invokeDesktop<DesktopSchedule>('desktop_schedule_save', { schedule })
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

export async function backupDesktopDatabase(): Promise<DesktopDatabaseActionResult | null> {
  if (!isDesktopApp) throw new Error('Database backup is available in Cortana Desktop')
  return invokeDesktop<DesktopDatabaseActionResult | null>('desktop_database_backup', {
    approved: true,
  })
}

export async function restoreDesktopDatabase(): Promise<DesktopDatabaseActionResult | null> {
  if (!isDesktopApp) throw new Error('Database restore is available in Cortana Desktop')
  return invokeDesktop<DesktopDatabaseActionResult | null>('desktop_database_restore', {
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

export async function openDesktopSecretFile(): Promise<void> {
  if (!isDesktopApp) throw new Error('Secret file opens are available in Cortana Desktop')
  return invokeDesktop<void>('desktop_secret_file_open')
}

export async function openDesktopUrl(url: string): Promise<void> {
  if (!isDesktopApp) throw new Error('Desktop URL opens are available in Cortana Desktop')
  const safe = safeSourceLink(url, { allowLocalFile: true })
  if (!safe) throw new Error('Unsupported or unsafe source link')
  return invokeDesktop<void>('desktop_url_open', { url: safe })
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

export async function listDesktopGithubRepositories(source: string): Promise<GithubRepositoryList> {
  if (!isDesktopApp) throw new Error('GitHub repository discovery is available in Cortana Desktop')
  return invokeDesktop<GithubRepositoryList>('desktop_github_repositories', { source })
}

export async function listDesktopDiscordChannels(source: string): Promise<DiscordChannelList> {
  if (!isDesktopApp) throw new Error('Discord channel discovery is available in Cortana Desktop')
  return invokeDesktop<DiscordChannelList>('desktop_discord_channels', { source })
}

export async function listDesktopDiscordServers(source: string): Promise<DiscordServerList> {
  if (!isDesktopApp) throw new Error('Discord server discovery is available in Cortana Desktop')
  return invokeDesktop<DiscordServerList>('desktop_discord_servers', { source })
}

export async function listDesktopSlackWorkspaces(source: string): Promise<SlackWorkspaceList> {
  if (!isDesktopApp) throw new Error('Slack workspace discovery is available in Cortana Desktop')
  return invokeDesktop<SlackWorkspaceList>('desktop_slack_workspaces', { source })
}

export async function listDesktopBuzzCommunities(source: string): Promise<BuzzCommunityList> {
  if (!isDesktopApp) throw new Error('Buzz community discovery is available in Cortana Desktop')
  return invokeDesktop<BuzzCommunityList>('desktop_buzz_communities', { source })
}

export async function listDesktopProviderModels(
  kind: ProviderModelKind
): Promise<ProviderModelList> {
  if (!isDesktopApp) throw new Error('Provider model discovery is available in Cortana Desktop')
  return invokeDesktop<ProviderModelList>('desktop_provider_models', { kind })
}

export async function pickDesktopPath(
  kind:
    'directory' | 'oauth-client' | 'google-token' | 'github-token' | 'discord-token' | 'slack-token'
): Promise<string | null> {
  if (!isDesktopApp) throw new Error('Native path selection is available in Cortana Desktop')
  return invokeDesktop<string | null>('desktop_path_pick', { kind })
}

export async function getDesktopSourceValidation(id: string): Promise<DesktopSourceJob> {
  if (!isDesktopApp) throw new Error('Source validation is available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob>('desktop_source_validation_status', { id })
}

export async function getDesktopSourceJobs(): Promise<DesktopSourceJob[]> {
  if (!isDesktopApp) throw new Error('Source jobs are available in Cortana Desktop')
  return invokeDesktop<DesktopSourceJob[]>('desktop_source_jobs_status')
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
  if (!response.ok) {
    // A local embedding restart can temporarily block the first statistics
    // snapshot while the model warms. Preserve the server's bounded retry
    // guidance instead of labeling a healthy-but-warming index as offline.
    if (response.status === 503) {
      const detail = (await response.text().catch(() => '')).trim()
      if (detail === 'Cortana is warming up; live status will be available shortly') {
        throw new Error(detail)
      }
    }
    throw new Error(`Status request failed (${response.status})`)
  }
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

export async function getReflection(
  objective: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<ReflectResponse> {
  const request = {
    objective,
    project: project || null,
    memory: { limit: 32 },
    include_evidence: true,
    token_budget: 2048,
    provider_policy: 'deterministic-only',
    deadline_ms: 5000,
    source: source || null,
  }
  if (isTauri()) {
    return invokeDesktop<ReflectResponse>('brain_reflect', { request }, signal)
  }
  const response = await authorizedFetch('/v1/memory/reflect', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
    signal,
  })
  if (!response.ok) throw new Error(`Reflection failed (${response.status})`)
  return (await response.json()) as ReflectResponse
}

export async function listMemoryCandidates(
  project?: string,
  queryValue?: string,
  status?: string
): Promise<MemoryCandidate[]> {
  if (isDemoMode) {
    const needle = queryValue?.trim().toLowerCase()
    return demoMemoryCandidates.filter(
      (candidate) =>
        (!project || candidate.project === project) &&
        (!status || candidate.status === status) &&
        (!needle ||
          `${candidate.title} ${candidate.content} ${candidate.project} ${candidate.source}`
            .toLowerCase()
            .includes(needle))
    )
  }
  type CandidatePage = { candidates: MemoryCandidate[]; truncated: boolean }
  const request = {
    project: project || null,
    limit: 1000,
    query: queryValue || null,
    status: status || null,
  }
  if (isTauri()) {
    const page = await invokeDesktop<CandidatePage>('brain_memory_candidates', { request })
    if (page.truncated) {
      throw new Error('Memory candidate review was truncated; narrow the search or status filter')
    }
    return page.candidates
  }
  const query = new URLSearchParams({ limit: '1000' })
  if (project) query.set('project', project)
  if (queryValue) query.set('query', queryValue)
  if (status) query.set('status', status)
  const response = await authorizedFetch(`/v1/memory/candidates?${query}`, {})
  if (!response.ok) throw new Error(`Memory candidate review failed (${response.status})`)
  const page = (await response.json()) as CandidatePage
  if (page.truncated) {
    throw new Error('Memory candidate review was truncated; narrow the search or status filter')
  }
  return page.candidates
}

export async function classifyMemoryCandidate(id: string): Promise<MemoryCandidateClassification> {
  if (isDemoMode) return { ...demoMemoryClassification, candidate_id: id }
  if (isTauri()) {
    return invokeDesktop<MemoryCandidateClassification>('brain_memory_candidate_action', {
      id,
      action: 'classify',
      request: null,
    })
  }
  const response = await authorizedFetch(
    `/v1/memory/candidates/${encodeURIComponent(id)}/classify`,
    { method: 'POST' }
  )
  if (!response.ok) throw new Error(`Memory candidate classification failed (${response.status})`)
  return (await response.json()) as MemoryCandidateClassification
}

export type MemoryCandidateAction =
  'approve' | 'edit-approve' | 'working' | 'supersede' | 'reject' | 'redact' | 'retry'

export async function actOnMemoryCandidate(
  id: string,
  action: MemoryCandidateAction,
  policy: MemoryReviewPolicy,
  edit?: { title: string; content: string }
): Promise<MemoryCandidateActionResult> {
  if (isDemoMode) {
    return {
      status: action === 'reject' || action === 'redact' ? action : 'review',
      updated: false,
      memory_id: null,
    }
  }
  const request = { policy: consolidationPolicy(policy), edit: edit || null }
  if (isTauri()) {
    return invokeDesktop<MemoryCandidateActionResult>('brain_memory_candidate_action', {
      id,
      action,
      request,
    })
  }
  if (action === 'edit-approve') {
    const edited = await authorizedFetch(`/v1/memory/candidates/${encodeURIComponent(id)}/edit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(edit),
    })
    if (!edited.ok) throw new Error(`Memory candidate edit failed (${edited.status})`)
  }
  if (action === 'working') {
    const working = await authorizedFetch(
      `/v1/memory/candidates/${encodeURIComponent(id)}/working`,
      {
        method: 'POST',
      }
    )
    if (!working.ok) throw new Error(`Memory candidate working failed (${working.status})`)
  }
  if (action === 'retry') {
    const retry = await authorizedFetch(`/v1/memory/candidates/${encodeURIComponent(id)}/retry`, {
      method: 'POST',
    })
    if (!retry.ok) throw new Error(`Memory candidate retry failed (${retry.status})`)
  }
  const suffix = action === 'reject' ? 'cancel' : action === 'redact' ? 'redact' : 'consolidate'
  const response = await authorizedFetch(
    `/v1/memory/candidates/${encodeURIComponent(id)}/${suffix}`,
    suffix === 'consolidate'
      ? {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            policy: request.policy,
            explicit_approval: true,
            action: action === 'supersede' ? 'supersede' : null,
          }),
        }
      : { method: 'POST' }
  )
  if (!response.ok) throw new Error(`Memory candidate ${action} failed (${response.status})`)
  return (await response.json()) as MemoryCandidateActionResult
}

export async function setMemoryConsolidationPaused(paused: boolean): Promise<void> {
  if (isDemoMode) return
  if (isTauri()) {
    await invokeDesktop('brain_memory_consolidation_control', {
      action: paused ? 'pause' : 'resume',
    })
    return
  }
  const response = await authorizedFetch(
    `/v1/memory/consolidation/${paused ? 'pause' : 'resume'}`,
    { method: 'POST' }
  )
  if (!response.ok) throw new Error(`Memory consolidation control failed (${response.status})`)
}

export async function getMemoryConsolidationState(): Promise<{
  paused: boolean
  canControl: boolean
}> {
  if (isDemoMode) return { paused: false, canControl: true }
  if (isTauri()) {
    const response = await invokeDesktop<{ paused: boolean; can_control: boolean }>(
      'brain_memory_consolidation_control',
      {
        action: 'status',
      }
    )
    return { paused: response.paused, canControl: response.can_control }
  }
  const response = await authorizedFetch('/v1/memory/consolidation/status', {})
  if (!response.ok) throw new Error(`Memory consolidation status failed (${response.status})`)
  const state = (await response.json()) as { paused: boolean; can_control: boolean }
  return { paused: state.paused, canControl: state.can_control }
}

export async function listDerivedMemories(project?: string): Promise<DerivedMemoryResponse> {
  if (isDemoMode) return demoDerivedMemories
  const request = { project: project || null, limit: 64 }
  if (isTauri()) {
    return invokeDesktop<DerivedMemoryResponse>('brain_memory_derived', { request })
  }
  const query = new URLSearchParams({ limit: '64' })
  if (project) query.set('project', project)
  const response = await authorizedFetch(`/v1/memory/derived?${query}`, {})
  if (!response.ok) throw new Error(`Derived memory review failed (${response.status})`)
  return (await response.json()) as DerivedMemoryResponse
}

export async function listCanonicalMemories(project?: string): Promise<AgentMemory[]> {
  if (isDemoMode) {
    return demoCanonicalMemories.filter((memory) => !project || memory.project === project)
  }
  const request = { project: project || null, limit: 100 }
  if (isTauri()) {
    return invokeDesktop<AgentMemory[]>('brain_memory_export', { request })
  }
  const query = new URLSearchParams({ limit: '100' })
  if (project) query.set('project', project)
  const response = await authorizedFetch(`/v1/memory/export?${query}`, {})
  if (!response.ok) throw new Error(`Canonical memory review failed (${response.status})`)
  return (await response.json()) as AgentMemory[]
}

function consolidationPolicy(policy: MemoryReviewPolicy) {
  return {
    version: 'cortana.memory.consolidation.v1',
    enabled: true,
    auto_retain_min_confidence: 0.9,
    auto_retain_min_importance: 0.65,
    max_queue: 1000,
    max_retries: 3,
    retry_backoff_seconds:
      policy.schedule === 'daily' ? 86_400 : policy.schedule === 'hourly' ? 3_600 : 0,
    candidate_expiry_days: policy.candidateExpiryDays,
    preferences: {
      allow_auto_retain: policy.autoCommit,
      allow_working_retention: true,
    },
    ceilings: {
      max_working_days: policy.maxWorkingDays,
      max_durable_days: policy.maxDurableDays,
      max_active: policy.maxActive,
    },
  }
}

export async function getDocuments(
  project?: string,
  source?: string,
  query?: string,
  cursor?: string,
  signal?: AbortSignal
): Promise<BrainDocumentPage> {
  if (isDemoMode) {
    const normalizedQuery = query?.trim().toLowerCase()
    const documents = demoEvidence
      .filter(
        (item) =>
          (!source || item.source === source) &&
          (!normalizedQuery ||
            [item.title, item.source, item.source_id].some((value) =>
              value.toLowerCase().includes(normalizedQuery)
            ))
      )
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
        'Merge short-lived changes into main after the full test and security suite passes, then monitor the release and roll back if health regresses [1]. Keep an explicit rollback owner in the checklist [2].',
      evidence,
      plan: {
        queries: [query, 'release promotion main checks', 'rollback owner health regression'],
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
  window.sessionStorage.removeItem('cortana_api_token')
  const token = await requestAccessToken()
  if (!token || init.signal?.aborted) return response

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
