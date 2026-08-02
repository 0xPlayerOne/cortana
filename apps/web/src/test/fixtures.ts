import { demoEvidence } from '../demo'
import type {
  AnswerResponse,
  AuditEvent,
  BrainDocument,
  BrainDocumentPage,
  BrainDocumentSummary,
  DesktopInfo,
  DesktopSettings,
  DesktopUpdate,
} from '../types'

function summaryOf(item: (typeof demoEvidence)[number], project: string): BrainDocumentSummary {
  return {
    id: item.chunk_id.replace(/[^a-f0-9]/gi, '').padEnd(16, '0'),
    source: item.source,
    source_id: item.source_id,
    title: item.title,
    uri: item.uri,
    updated_at: item.updated_at,
    project,
    chunk_count: 1,
    content_chars: item.content.length,
  }
}

/** First keyset page: two documents, one more page available. */
export const firstDocumentsPage: BrainDocumentPage = {
  documents: demoEvidence.slice(0, 2).map((item) => summaryOf(item, 'work')),
  next_cursor: 'cursor-2',
}

/** Second keyset page: one new document, end of the result set. */
export const secondDocumentsPage: BrainDocumentPage = {
  documents: demoEvidence.slice(2, 3).map((item) => summaryOf(item, 'work')),
  next_cursor: null,
}

/** Canonical document returned by getDocument for a selected row. */
export const canonicalDocument: BrainDocument = {
  ...summaryOf(demoEvidence[1], 'work'),
  content:
    'Promote staging only after unit, integration, end-to-end, and security checks pass.\n\nObserve the deployment before closing the release.',
  metadata: { owner: 'release-eng', review: ['staging', 'prod'] },
  acl: ['work'],
  backlinks: [
    {
      id: 'link-back',
      source: 'work-drive',
      source_id: 'rollback-checklist',
      title: 'Deployment rollback checklist',
      uri: null,
      updated_at: '2026-07-25T08:00:00Z',
      project: 'work',
    },
  ],
  surrounding: [
    {
      id: 'link-around',
      source: 'personal-notes',
      source_id: 'incident-response',
      title: 'Incident response playbook',
      uri: null,
      updated_at: '2026-07-13T09:30:00Z',
      project: 'work',
    },
  ],
  truncated: false,
}

/** Successful answer for the search error/retry flow. */
export const answerResponse: AnswerResponse = {
  query: 'release cadence',
  answer:
    'Promote short-lived changes through staging after the full test and security suite passes [1].',
  evidence: demoEvidence,
  plan: { queries: ['release cadence'], model_generated: true },
  mode: 'synthesized',
  cached: false,
  latency_ms: 184,
  warnings: ['Read-only preview'],
}

/** Desktop settings fixture; needs_setup=false keeps the app on the knowledge view. */
export const desktopSettings: DesktopSettings = {
  config_path: '/Users/you/.config/cortana/config.toml',
  secret_file_path: '/Users/you/.config/cortana/secrets.env',
  secret_file_managed: true,
  embedding_service_program: null,
  needs_setup: false,
  restart_required: false,
  workspaces: [
    { id: 'work', name: 'Work', account_label: 'team@example.com', color: '#5A9BD5' },
    { id: 'personal', name: 'Personal', account_label: null, color: '#E8A83B' },
  ],
  sources: [],
  auth_principals: [],
  embedding: {
    provider: 'local',
    base_url: 'http://127.0.0.1:6999/v1',
    model: 'Qwen/Qwen3-Embedding-0.6B',
    api_key_env: null,
    dimension: 1024,
    cache_max_entries: 250000,
    request_timeout_seconds: 180,
    request_concurrency: 4,
    startup_timeout_seconds: 120,
    memory_limit_mb: 4096,
  },
  query: {
    provider: 'local',
    base_url: 'http://127.0.0.1:8008/v1',
    model: 'auto-efficient',
    api_key_env: null,
    synthesis_enabled: false,
    max_planned_queries: 4,
    retrieval_limit: 10,
    result_limit: 20,
    context_tokens: 8000,
    output_tokens: 1200,
    request_timeout_seconds: 45,
    answer_timeout_seconds: 55,
    request_concurrency: 4,
    cache_max_entries: 10000,
    cache_ttl_seconds: 3600,
  },
  hindsight: {
    enabled: false,
    provider: 'hindsight',
    base_url: 'http://127.0.0.1:8888',
    bank: 'default',
    token_env: null,
    optional: true,
    wired_to_ingestion: false,
  },
  honcho: {
    enabled: false,
    provider: 'honcho',
    base_url: 'https://api.honcho.dev',
    workspace_id: 'default',
    peer_id: 'cortana',
    session_prefix: 'cortana',
    token_env: null,
    optional: true,
    wired_to_ingestion: false,
  },
  ingestion: {
    max_documents_per_source: 2000,
    max_bytes_per_source: 134217728,
    max_duration_seconds: 900,
    document_batch_size: 16,
    request_concurrency: 1,
  },
  runtime: {
    data_dir: '/Users/you/Library/Application Support/cortana',
    connector_timeout_seconds: 21600,
    audit_max_events: 10000,
  },
  secrets: [],
}

export const desktopInfo: DesktopInfo = {
  desktop_version: '0.11.2',
  backend_origin: 'http://127.0.0.1:7331',
  autostart_enabled: false,
  platform: 'macos',
}

export const runtimeAuditEvents: AuditEvent[] = [
  { id: 1, event: 'brain_answer', at_unix_seconds: 1785000000, scope: 'query' },
  { id: 2, event: 'brain_documents', at_unix_seconds: 1785000060, scope: 'status' },
]

export const desktopAuditEvents: AuditEvent[] = [
  { id: 7, action: 'settings_saved', at_unix_seconds: 1785000120 },
]

export const desktopUpdate: DesktopUpdate = {
  current_version: '0.11.2',
  available_version: '9.9.9',
  release_date: '2026-07-30T00:00:00Z',
  release_notes: 'Fixes and improvements.',
  changelog: '0.11.2 release notes.',
  github_url: 'https://github.com/0xPlayerOne/cortana',
  phase: 'available',
  downloaded_bytes: 0,
  total_bytes: 0,
  error: null,
  restart_required: false,
}
