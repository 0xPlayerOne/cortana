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
}

export type BrainDocumentSummary = {
  id: string
  source: string
  title: string
  uri: string | null
  updated_at: string
  project: string
  chunk_count: number
  content_chars: number
}

export type BrainDocument = BrainDocumentSummary & {
  content: string
  metadata: Record<string, unknown>
  truncated: boolean
}

export type BrainDocumentPage = {
  documents: BrainDocumentSummary[]
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
  budget_documents: number
  budget_bytes: number
  budget_seconds: number
}

export type ConfiguredSourceSummary = {
  name: string
  source: string
  kind: string
  project: string
  enabled: boolean
  max_documents: number
  max_bytes: number
  max_duration_seconds: number
  validation?: {
    source: string
    project: string
    kind: string
    status: 'succeeded' | 'failed'
    validated_at: string
    documents: number | null
    bytes: number | null
    max_documents: number
    max_bytes: number
    max_seconds: number
    error: string | null
  } | null
}

export type IngestionStatus = {
  mode: 'manual' | 'scheduled'
  scheduled: boolean
  max_documents_per_source: number
  max_bytes_per_source: number
  max_duration_seconds: number
  request_concurrency: number
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
  embedding_fingerprint: string | null
  embedding_cache_entries: number
  embedding_cache_hits: number
  query_cache_entries: number
  query_cache_hits: number
  answers_total: number
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

export type AuthPrincipalSettings = {
  principal: string
  token_env: string
  scopes: Array<'query' | 'status' | 'admin'>
  acl: string[]
}

export type DesktopSettings = {
  config_path: string
  secret_file_path: string
  needs_setup: boolean
  restart_required: boolean
  workspaces: WorkspaceSettings[]
  sources: SourceSettings[]
  auth_principals: AuthPrincipalSettings[]
  embedding: EmbeddingSettings
  query: QuerySettings
  ingestion: {
    max_documents_per_source: number
    max_bytes_per_source: number
    max_duration_seconds: number
    document_batch_size: number
    request_concurrency: number
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
  'workspaces' | 'sources' | 'auth_principals' | 'embedding' | 'query' | 'ingestion' | 'runtime'
> & {
  secrets: Array<{ name: string; value?: string; clear?: boolean }>
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
  operation: 'validation' | 'authorization' | 'trial-sync'
  source: string
  kind: string
  project: string
  status: 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled'
  summary: string
  log: string
  started_at_unix_seconds: number
  completed_at_unix_seconds: number | null
  exit_code: number | null
  retryable: boolean
  writes_indexed_data: boolean
}

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

export type DesktopInfo = {
  desktop_version: string
  backend_origin: string
  autostart_enabled: boolean
  platform: string
}

export type DesktopServiceReport = {
  platform: string
  supported: boolean
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
  query: string
  answer: string
  evidence: Evidence[]
  plan: {
    queries: string[]
    model_generated: boolean
  }
  mode: 'extractive' | 'synthesized'
  cached: boolean
  latency_ms: number
  warnings: string[]
}

export type ContextBundle = {
  query: string
  context: string
  evidence: Evidence[]
  metrics: {
    retrieved: number
    included: number
    omitted: number
    estimated_tokens: number
    max_tokens: number
  }
}
