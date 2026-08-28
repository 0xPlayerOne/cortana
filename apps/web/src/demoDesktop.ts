import type {
  DesktopInfo,
  DesktopInstallJob,
  DesktopReadiness,
  DesktopReadinessActivity,
  DesktopServiceActivity,
  DesktopServiceReport,
  DesktopSettings,
  DesktopUpdate,
} from './types'

export const demoDesktopSettings: DesktopSettings = {
  config_path: '/example/cortana/config.toml',
  secret_file_path: '/example/cortana/secrets.env',
  secret_file_managed: true,
  embedding_service_program: null,
  needs_setup: false,
  restart_required: false,
  workspaces: [
    { id: 'work', name: 'Work', account_label: 'team@example.test', color: '#5A9BD5' },
    { id: 'personal', name: 'Personal', account_label: null, color: '#E8A83B' },
  ],
  sources: [
    {
      name: 'work-code',
      kind: 'filesystem',
      enabled: true,
      project: 'work',
      root: '/example/workspace',
      source: null,
      channels: [],
      repositories: [],
      servers: [],
      teams: [],
      team_names: [],
      communities: [],
      community_names: [],
      token_env: null,
      token_path: null,
      oauth_client_path: null,
      query: null,
      labels: ['demo'],
      max_content_chars: null,
      max_documents: 100,
      max_bytes: 5_242_880,
      max_duration_seconds: 60,
      exclude: [],
      acl: ['work'],
      editable: true,
    },
  ],
  auth_principals: [
    {
      principal: 'desktop-agent',
      token_env: 'CORTANA_DEMO_TOKEN',
      scopes: ['query'],
      acl: ['work'],
    },
  ],
  embedding: {
    provider: 'local',
    base_url: 'http://127.0.0.1:6999/v1',
    model: 'Qwen/Qwen3-Embedding-0.6B',
    api_key_env: null,
    dimension: 1024,
    cache_max_entries: 250000,
    request_timeout_seconds: 180,
    request_concurrency: 4,
    startup_timeout_seconds: 300,
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
  memory: { max_active: 100000, default_confidence: 0.7, default_importance: 0.5 },
  ingestion: {
    max_documents_per_source: 2000,
    max_bytes_per_source: 134217728,
    max_duration_seconds: 900,
    document_batch_size: 16,
    request_concurrency: 1,
    sync_freshness_hours: 48,
  },
  runtime: {
    data_dir: '/example/cortana/data',
    connector_timeout_seconds: 21600,
    audit_max_events: 10000,
  },
  secrets: [{ name: 'CORTANA_DEMO_TOKEN', configured: true, source: 'secret-file' }],
}

export const demoDesktopInfo: DesktopInfo = {
  desktop_version: 'demo',
  backend_origin: 'http://127.0.0.1:7331',
  autostart_enabled: true,
  platform: 'demo',
}

export const demoDesktopServices: DesktopServiceReport = {
  platform: 'demo',
  supported: true,
  services: (['embedding', 'server', 'sync', 'backup'] as const).map((name) => ({
    name,
    label: `demo.cortana.${name}`,
    installed: true,
    loaded: name !== 'sync',
    state: name === 'sync' ? 'scheduled' : 'running',
    pid: name === 'sync' ? null : 4242,
    last_exit_status: null,
  })),
}

export const demoDesktopUpdate: DesktopUpdate = {
  current_version: 'demo',
  available_version: 'demo-next',
  release_date: '2026-08-27T00:00:00Z',
  release_notes: 'A signed demo update used only for non-secret visual evidence.',
  changelog: 'Settings and recovery surfaces migrated to shared shadcn composition.',
  github_url: 'https://example.test/cortana/releases',
  phase: 'available',
  downloaded_bytes: 0,
  total_bytes: 1_048_576,
  error: null,
  restart_required: false,
}

export const demoDesktopReadiness: DesktopReadiness = {
  scanned_at_unix_seconds: 1_787_785_600,
  platform: 'demo',
  tools_ready: true,
  core: {
    passed: true,
    query_mode: 'synthesized',
    embedding_generation: { stored: 'demo-v1', configured: 'demo-v1' },
    checks: [
      { name: 'api-health', passed: true, detail: 'Local API is ready.' },
      { name: 'embedding-health', passed: true, detail: 'Embedding runtime is ready.' },
    ],
  },
  core_error: null,
  tools: [
    {
      id: 'connectors',
      label: 'Connector runtime',
      required: true,
      available: true,
      path: '/example/cortana/connectors',
      version: 'demo',
      install_supported: true,
      detail: 'Bundled connector runtime is ready.',
    },
  ],
}

export type DemoDesktopState =
  | 'configured'
  | 'setup'
  | 'busy'
  | 'success'
  | 'warning'
  | 'failure'
  | 'cancelled'
  | 'retry'
  | 'recovery'

export function demoDesktopState(state: DemoDesktopState) {
  let settings = demoDesktopSettings
  let readiness = demoDesktopReadiness
  let readinessActivity: DesktopReadinessActivity | null = null
  let serviceActivity: DesktopServiceActivity | null = null
  let installerJob: DesktopInstallJob | null = null
  let update = demoDesktopUpdate

  if (state === 'setup') {
    settings = { ...demoDesktopSettings, needs_setup: true, workspaces: [], sources: [] }
    readiness = {
      ...demoDesktopReadiness,
      tools_ready: false,
      core: null,
      core_error: 'Complete the required local tool setup, then retry readiness.',
      tools: demoDesktopReadiness.tools.map((tool) => ({
        ...tool,
        available: false,
        path: null,
        version: null,
        detail: 'Installation is required.',
      })),
    }
  } else if (state === 'busy') {
    readinessActivity = { status: 'running', detail: 'Checking bounded production gates.' }
  } else if (state === 'success') {
    serviceActivity = {
      target: 'core',
      action: 'restart',
      status: 'succeeded',
      detail: 'All core services restarted.',
    }
  } else if (state === 'warning') {
    readiness = {
      ...demoDesktopReadiness,
      tools_ready: false,
      core: { ...demoDesktopReadiness.core!, passed: false },
      core_error: 'One production gate needs attention before ingestion.',
    }
  } else if (state === 'failure' || state === 'retry') {
    readinessActivity = {
      status: 'failed',
      detail: 'Readiness check failed safely. Review services and retry.',
    }
  } else if (state === 'cancelled') {
    installerJob = {
      id: 'demo-cancelled',
      tool: 'connectors',
      status: 'cancelled',
      summary: 'Connector installation cancelled before changes were applied.',
      log: 'Cancelled safely. No secret values or host paths were retained.',
      started_at_unix_seconds: 1_787_785_600,
      completed_at_unix_seconds: 1_787_785_601,
      exit_code: null,
      retryable: true,
    }
  } else if (state === 'recovery') {
    settings = { ...demoDesktopSettings, restart_required: true }
    serviceActivity = {
      target: 'core',
      action: 'restart',
      status: 'failed',
      detail: 'Restart failed safely; configuration remains saved.',
    }
  }

  if (state === 'failure') {
    update = { ...demoDesktopUpdate, phase: 'failed', error: 'Signature check failed safely.' }
  }

  return { settings, readiness, readinessActivity, serviceActivity, installerJob, update }
}
