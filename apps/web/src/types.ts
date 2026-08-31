export type Evidence = {
  chunk_id: string
  source: string
  source_id: string
  title: string
  uri: string | null
  content: string
  score: number
  semantic_rank: number | null
  lexical_rank: number | null
  updated_at: string
  metadata?: {
    code?: {
      repository?: string
      branch?: string | null
      commit_sha?: string | null
      dirty?: boolean
      revision?: string
    }
    [key: string]: unknown
  }
}

export type BrainDocumentSummary = {
  id: string
  source: string
  source_id: string
  title: string
  uri: string | null
  updated_at: string
  project: string
  chunk_count: number
  content_chars: number
}

export type BrainDocumentReference = Pick<
  BrainDocumentSummary,
  'id' | 'source' | 'source_id' | 'title' | 'uri' | 'updated_at' | 'project'
>

export type BrainDocument = BrainDocumentSummary & {
  content: string
  metadata: Record<string, unknown>
  acl: string[]
  backlinks: BrainDocumentReference[]
  surrounding: BrainDocumentReference[]
  truncated: boolean
}

export type BrainDocumentPage = {
  documents: BrainDocumentSummary[]
  next_cursor: string | null
}

export type BrainGraphNodeKind =
  | 'workspace'
  | 'source'
  | 'document'
  | 'chunk'
  | 'entity'
  | 'memory'
  | 'observation'
  | 'mental-model'
  | 'repository'
  | 'file'
  | 'symbol'

export type BrainGraphNode = {
  id: string
  kind: BrainGraphNodeKind
  label: string
  project: string
  source: string | null
  document_id: string | null
  canonical_record_id?: string | null
  contract_version?: 'cortana.knowledge-graph.v1'
  updated_at?: string
  acl?: string[]
  content_revision?: string
  lifecycle_status?: string
  representation_contract_version?: string
  derivation_version?: string
  memory_revision?: number
  supporting_memory_ids?: string[]
  contradicting_memory_ids?: string[]
  citation_authority?: boolean
}

export type BrainGraphEdge = {
  source: string
  target: string
  kind:
    | 'contains'
    | 'references'
    | 'backlink'
    | 'nearby'
    | 'same-thread'
    | 'authored-by'
    | 'mentions'
    | 'temporal'
    | 'semantically-related'
    | 'supports'
    | 'contradicts'
    | 'reinforces'
    | 'supersedes'
    | 'observes'
    | 'derives'
    | 'depends-on'
    | 'defines'
    | 'calls'
    | 'imports'
  contract_version?: 'cortana.knowledge-graph.v1'
  origin?: 'explicit' | 'derived' | 'inferred'
  derivation_version?: string
  confidence?: number | null
  citation_authority?: boolean
  updated_at?: string
  project?: string
  acl?: string[]
  support?: {
    record_ids: string[]
    invalidation_keys: string[]
  }
}

export type BrainGraphPage = {
  contract_version?: 'cortana.knowledge-graph.v1'
  nodes: BrainGraphNode[]
  edges: BrainGraphEdge[]
  next_cursor: string | null
}

export type SourceSummary = {
  source: string
  project: string
  documents: number
  chunks: number
  latest_updated_at: string | null
}

export type SourceSyncSummary = {
  source: string
  project: string
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | 'budget_exceeded'
  started_at: string
  completed_at: string | null
  documents: number | null
  bytes: number | null
  deleted: number | null
  /** Durable in-flight counters; absent when talking to an older server. */
  progress_documents?: number
  progress_bytes?: number
  progress_updated_at?: string | null
  budget_documents: number
  budget_bytes: number
  budget_seconds: number
}

export type SourceAuthorizationSummary = {
  method: 'none' | 'token' | 'google_oauth' | 'github_oauth' | 'discord_rpc' | 'slack_oauth'
  setup_required: boolean
  authorized: boolean
}

export type GithubRepositorySummary = {
  id: number
  full_name: string
  private: boolean
  default_branch: string
  html_url: string
}

export type GithubRepositoryList = {
  repositories: GithubRepositorySummary[]
  truncated: boolean
}

export type DiscordChannelSummary = {
  id: string
  name: string
  kind: string
}

export type DiscordGuildChannels = {
  id: string
  name: string
  channels: DiscordChannelSummary[]
  truncated: boolean
}

export type DiscordChannelList = {
  guilds: DiscordGuildChannels[]
  truncated: boolean
}

export type DiscordServerSummary = {
  id: string
  name: string
}

export type DiscordServerList = {
  guilds: DiscordServerSummary[]
  truncated: boolean
}

export type SlackWorkspaceSummary = {
  id: string
  name: string
}

export type SlackWorkspaceList = {
  teams: SlackWorkspaceSummary[]
  truncated: boolean
}

export type BuzzCommunitySummary = {
  id: string
  name: string
}

export type BuzzCommunityList = {
  communities: BuzzCommunitySummary[]
  truncated: boolean
}

export type ConfiguredSourceSummary = {
  name: string
  source: string
  kind: string
  project: string
  enabled: boolean
  acl: string[]
  max_documents: number
  max_bytes: number
  max_duration_seconds: number
  authorization?: SourceAuthorizationSummary | null
  validation?: {
    source: string
    project: string
    kind: string
    status: 'succeeded' | 'failed'
    validated_at: string
    // `fresh` is false when the succeeded validation is older than the
    // configured `validation_max_age_hours` bound. Older servers omit both
    // fields; the workspace treats a missing flag as current.
    fresh?: boolean
    age_seconds?: number
    // `complete` is false when the succeeded validation was a bounded sample
    // that may authorize only equally bounded non-reconciling runs. `null` or
    // an omitted field means completeness is unknown and cannot authorize
    // recurring or full-corpus sync.
    complete?: boolean | null
    documents: number | null
    bytes: number | null
    max_documents: number
    max_bytes: number
    max_seconds: number
    error: string | null
    error_category?:
      'timeout' | 'authorization' | 'missing-credential-or-path' | 'budget' | 'connector' | null
  } | null
}

export type IngestionStatus = {
  mode: 'manual' | 'scheduled'
  scheduled: boolean
  max_documents_per_source: number
  max_bytes_per_source: number
  max_duration_seconds: number
  request_concurrency: number
  validation_max_age_hours?: number
  sync_freshness_hours?: number
  validation_state_error?: string | null
  configured_sources: ConfiguredSourceSummary[]
}

export type WorkspaceSettings = {
  id: string
  name: string
  account_label: string | null
  color: string | null
}

export type SourceKind =
  | 'filesystem'
  | 'apple-notes'
  | 'buzz'
  | 'google-drive'
  | 'gmail'
  | 'google-calendar'
  | 'github'
  | 'slack'
  | 'discord'
  | 'external'

export type SourceSettings = {
  name: string
  kind: SourceKind
  enabled: boolean
  project: string
  root: string | null
  source: string | null
  channels: string[]
  /** Exact Apple Notes folder names included by this source. */
  folders?: string[]
  /** Exact Apple Notes folder names excluded by this source. */
  exclude_folders?: string[]
  repositories: string[]
  /** Discord servers (guilds) assigned to this source's workspace via Desktop RPC authorization. */
  servers: string[]
  /** Slack team (workspace) ids assigned to this source's workspace via browser authorization. */
  teams: string[]
  /** Slack team display names kept index-aligned with `teams`. */
  team_names: string[]
  /** Buzz community ids assigned to this source's workspace from the read-only agents/teams.json identity file. */
  communities: string[]
  /** Buzz community display names kept index-aligned with `communities`. */
  community_names: string[]
  token_env: string | null
  token_path: string | null
  oauth_client_path: string | null
  query: string | null
  labels: string[]
  max_content_chars: number | null
  max_documents: number | null
  max_bytes: number | null
  max_duration_seconds: number | null
  exclude: string[]
  acl: string[]
  editable: boolean
}

export type BrainStatus = {
  status: string
  stats_stale?: boolean
  stats_age_seconds?: number
  stats_warning?: string
  embedding_fingerprint: string | null
  embedding_cache_entries: number
  embedding_cache_hits: number
  query_cache_entries: number
  query_cache_hits: number
  answers_total: number
  retrieval_fallbacks_total?: number
  query: {
    mode: 'extractive' | 'synthesized'
    model: string | null
    max_planned_queries: number
    retrieval_limit: number
    result_limit: number
    cache_ttl_seconds: number
    answer_timeout_seconds: number
  }
  documents: number
  chunks: number
  sources: SourceSummary[]
  sync_runs: SourceSyncSummary[]
  ingestion: IngestionStatus
  workspaces: WorkspaceSettings[]
  memory: MemoryStats
}

export type MemoryStats = {
  active: number
  expired: number
  superseded: number
  retracted: number
  total: number
}

export type ProviderSettings = {
  provider: 'local' | 'cloud'
  base_url: string
  model: string
  api_key_env: string | null
}

export type EmbeddingSettings = ProviderSettings & {
  dimension: number
  cache_max_entries: number
  request_timeout_seconds: number
  request_concurrency: number
  startup_timeout_seconds: number
  memory_limit_mb: number
}

export type QuerySettings = ProviderSettings & {
  synthesis_enabled: boolean
  max_planned_queries: number
  retrieval_limit: number
  result_limit: number
  context_tokens: number
  output_tokens: number
  request_timeout_seconds: number
  answer_timeout_seconds: number
  request_concurrency: number
  cache_max_entries: number
  cache_ttl_seconds: number
}

export type MemorySettings = {
  max_active: number
  default_confidence: number
  default_importance: number
}

export type AuthPrincipalSettings = {
  principal: string
  token_env: string
  scopes: Array<'query' | 'status' | 'admin' | 'memory'>
  acl: string[]
}

export type ProviderModelKind = 'embedding' | 'query'

/** One model advertised by the configured OpenAI-compatible provider. */
export type ProviderModelEntry = {
  id: string
  object: string | null
  owned_by: string | null
  created: number | null
  /** Explicit capability metadata advertised by the provider; never inferred. */
  capabilities?: unknown
}

/** Bounded model catalog returned by `cortana provider-models`. */
export type ProviderModelList = {
  kind: ProviderModelKind
  /** Normalized provider base URL the catalog was fetched from. */
  provider: string
  models: ProviderModelEntry[]
  truncated: boolean
}

export type DesktopSettings = {
  config_path: string
  secret_file_path: string
  secret_file_managed: boolean
  embedding_service_program: string | null
  needs_setup: boolean
  restart_required: boolean
  workspaces: WorkspaceSettings[]
  sources: SourceSettings[]
  auth_principals: AuthPrincipalSettings[]
  embedding: EmbeddingSettings
  query: QuerySettings
  memory: MemorySettings
  ingestion: {
    max_documents_per_source: number
    max_bytes_per_source: number
    max_duration_seconds: number
    document_batch_size: number
    request_concurrency: number
    sync_freshness_hours: number
  }
  runtime: {
    data_dir: string
    connector_timeout_seconds: number
    audit_max_events: number
  }
  secrets: Array<{
    name: string
    configured: boolean
    source: 'secret-file' | 'environment' | 'unset'
  }>
}

export type DesktopSettingsUpdate = Pick<
  DesktopSettings,
  | 'workspaces'
  | 'sources'
  | 'auth_principals'
  | 'embedding'
  | 'query'
  | 'memory'
  | 'ingestion'
  | 'runtime'
> & {
  secrets: Array<{ name: string; value?: string; clear?: boolean }>
}

export type DesktopSecretStorageMigration = {
  backend: 'native'
  migrated: number
  file_values_removed: boolean
}

export type DesktopPortableSettings = Omit<DesktopSettingsUpdate, 'secrets'>

export type DesktopSettingsExport = {
  path: string
  format_version: number
  secrets_included: false
  omitted_external_sources: string[]
}

export type DesktopSettingsImport = {
  path: string
  format_version: number
  secrets_included: false
  preserved_external_sources: string[]
  settings: DesktopPortableSettings
}

export type DesktopSourceJob = {
  id: string
  operation: 'connection-check' | 'validation' | 'authorization' | 'trial-sync' | 'initial-sync'
  source: string
  kind: string
  project: string
  acl: string[]
  status: 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled'
  summary: string
  log: string
  started_at_unix_seconds: number
  completed_at_unix_seconds: number | null
  exit_code: number | null
  retryable: boolean
  writes_indexed_data: boolean
  budget: string | null
}

export type InitialSyncBudget = 'small' | 'medium' | 'large'

export const INITIAL_SYNC_BUDGETS: Array<{
  budget: InitialSyncBudget
  documents: number
  bytes: number
  seconds: number
}> = [
  { budget: 'small', documents: 100, bytes: 26_214_400, seconds: 900 },
  { budget: 'medium', documents: 500, bytes: 67_108_864, seconds: 1_800 },
  { budget: 'large', documents: 2_000, bytes: 134_217_728, seconds: 3_600 },
]

export type DesktopInitialSyncPlan = {
  source: string
  kind: string
  project: string
  acl: string[]
  enabled: boolean
  budget: InitialSyncBudget
  budget_documents: number
  budget_bytes: number
  budget_seconds: number
  writes_indexed_data: boolean
  requires_validation: boolean
  validation_covers_budget: boolean | null
  plan_id: string
}

export type DesktopInitialSyncOutcome =
  (DesktopInitialSyncPlan & { outcome: 'plan' }) | (DesktopSourceJob & { outcome: 'job' })

export type DesktopSetupOpen = {
  source: string
  kind: string
  url: string
  opened: boolean
}

export type DesktopReadiness = {
  scanned_at_unix_seconds: number
  platform: string
  tools_ready: boolean
  core: {
    passed: boolean
    query_mode: string
    embedding_generation: {
      stored: string | null
      configured: string
    }
    checks: Array<{ name: string; passed: boolean; detail: string }>
  } | null
  core_error: string | null
  tools: Array<{
    id: string
    label: string
    required: boolean
    available: boolean
    path: string | null
    version: string | null
    install_supported: boolean
    detail: string
  }>
}

export type DesktopReadinessActivity = {
  status: 'running' | 'succeeded' | 'failed'
  detail: string | null
}

export type DesktopInfo = {
  desktop_version: string
  backend_origin: string
  autostart_enabled: boolean
  platform: string
}

export type DesktopServiceReport = {
  platform: string
  supported: boolean
  activity?: DesktopServiceActivity | null
  services: Array<{
    name: 'embedding' | 'server' | 'sync' | 'backup'
    label: string
    installed: boolean
    loaded: boolean
    state: string | null
    pid: number | null
    last_exit_status: number | null
  }>
}

export type DesktopDatabaseActionResult = {
  action: 'backup' | 'restore'
  path: string
  bytes: number
  detail: string
}

export type DesktopSchedule = {
  sync_interval_seconds: number
  backup_interval_seconds: number
}

/**
 * Shell-owned status for a service control request. Native service commands
 * are awaited by the bridge, but keeping this small snapshot in App makes the
 * operation visible when the user changes Settings sections or returns to the
 * knowledge view before the command finishes.
 */
export type DesktopServiceActivity = {
  target: string
  action: 'install' | 'start' | 'stop' | 'restart'
  status: 'running' | 'succeeded' | 'failed' | 'cancelled'
  detail: string | null
  started_at_unix_seconds?: number
  elapsed_ms?: number | null
  last_output?: string | null
}

export type DesktopInstallJob = {
  id: string
  tool: string
  status: 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled'
  summary: string
  log: string
  started_at_unix_seconds: number
  completed_at_unix_seconds: number | null
  exit_code: number | null
  retryable: boolean
}

export type DesktopUpdate = {
  current_version: string
  available_version: string | null
  release_date: string | null
  release_notes: string | null
  changelog: string
  github_url: string
  phase:
    | 'idle'
    | 'checking'
    | 'current'
    | 'unavailable'
    | 'available'
    | 'downloading'
    | 'installing'
    | 'installed'
    | 'failed'
  downloaded_bytes: number
  total_bytes: number | null
  error: string | null
  restart_required: boolean
}

export type AuditEvent = Record<string, unknown>

export type AnswerResponse = {
  contract_version?: string
  query: string
  answer: string
  evidence: Evidence[]
  memories?: AgentMemory[]
  plan: {
    queries: string[]
    model_generated: boolean
  }
  mode: 'extractive' | 'synthesized'
  cached: boolean
  latency_ms: number
  warnings: string[]
  retrieval_mode?: 'hybrid' | 'lexical-fallback'
  retrieval_degraded?: boolean
  corpus_revision?: number
  memory_revision?: number | null
  degradation?: {
    code: string
    detail?: string | null
  } | null
}

export type ReflectResponse = {
  contract_version: string
  request_digest: string
  status:
    'completed' | 'fallback' | 'provider_unavailable' | 'provider_failed' | 'deadline_exceeded'
  objective: string
  project?: string | null
  memory_revision: number
  privacy_scope_digest: string
  provider: {
    policy: 'deterministic-only' | 'prefer-provider' | 'require-provider'
    selected: string
    status: string
    detail?: string | null
  }
  claims: Array<{
    text: string
    supporting_memory_ids: string[]
    supporting_evidence_ids: string[]
  }>
  patterns: Array<{ statement: string; supporting_memory_ids: string[] }>
  tensions: Array<{ statement: string; supporting_memory_ids: string[] }>
  recommendations: Array<{ statement: string; supporting_memory_ids: string[] }>
  chronology: Array<{ observed_at: string; title: string; memory_id: string }>
  proposed_candidates: Array<{
    project: string
    title: string
    content: string
    content_type: string
    retention_tier: string
    scope: string
    supporting_memory_ids: string[]
    approval_required: boolean
  }>
  evidence_ids: string[]
  metrics: {
    memories_considered: number
    memories_included: number
    evidence_considered: number
    evidence_included: number
    estimated_tokens: number
    canonical_memory_mutated: false
  }
}

export type ContextBundle = {
  contract_version?: string
  context_bundle_id?: string
  canonical_digest?: string
  created_at?: string
  token_budget?: number
  query: string
  context: string
  evidence: Evidence[]
  memories?: AgentMemory[]
  metrics: {
    retrieved: number
    included: number
    omitted: number
    memories_retrieved?: number
    memories_included?: number
    memories_omitted?: number
    estimated_tokens: number
    max_tokens: number
  }
  retrieval_mode?: 'hybrid' | 'lexical-fallback'
  degradation?: {
    code: string
    detail?: string | null
  } | null
  retrieval_warning?: string | null
  corpus_revision?: number
  memory_revision?: number | null
  embedding_fingerprint?: string | null
  retrieval_contract_version?: string
  privacy_scope_digest?: string
}

export type AgentMemory = {
  id: string
  kind: string
  content_type?: string
  retention_tier?: string
  scope?: string
  project: string
  title: string
  content: string
  confidence: number
  importance: number
  relevance_score?: number
  source?: string
  source_id?: string
  status?: string
  provenance?: Record<string, unknown>
  supersedes_id?: string | null
  observed_at?: string
  valid_until?: string | null
  updated_at: string
}

export type MemoryCandidate = {
  id: string
  observation_kind: string
  content_type: string
  retention_tier: string
  scope: string
  project: string
  title: string
  content: string
  source: string
  source_id: string
  dedupe_key?: string | null
  confidence: number
  importance: number
  sensitivity: string
  status: string
  acl: string[]
  provenance: Record<string, unknown>
  expires_at: string
  rejection_reason?: string | null
  created_at: string
  updated_at: string
  consolidation?: {
    status: string
    decision: string
    classification: string
    policy_version: string
    attempts: number
    memory_id?: string | null
    last_error?: string | null
    updated_at: string
    reason_code?: string | null
    explanation?: string | null
    supporting_memory_ids: string[]
  } | null
}

export type MemoryCandidateActionResult = {
  status: string
  updated?: boolean
  memory_id?: string | null
  decision?: {
    decision: string
    classification: string
    reason_code: string
    explanation: string
  }
}

export type MemoryCandidateClassification = {
  candidate_id: string
  classification: string
  confidence: number
  supporting_memory_ids: string[]
  explanation: string
}

export type DerivedMemoryResponse = {
  contract_version: string
  derivation_version: string
  memory_revision: number
  canonical_memory_mutated: false
  recomputed: true
  representations: Array<{
    id: string
    kind: string
    statement: string
    confidence: number
    supporting_memory_ids: string[]
    contradicting_memory_ids: string[]
    citation_authority: false
  }>
  relations: Array<{
    id: string
    kind: string
    subject: string
    predicate: string
    object: string
    supporting_memory_ids: string[]
    citation_authority: false
  }>
}

export type MemoryReviewPolicy = {
  autoCommit: boolean
  maxWorkingDays: number
  maxDurableDays: number
  maxActive: number
  candidateExpiryDays: number
  schedule: 'manual' | 'hourly' | 'daily'
}
