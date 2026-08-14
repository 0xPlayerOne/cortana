import { afterEach, expect, mock, test } from 'bun:test'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'

import { demoStatus } from './demo'
import {
  desktopAuditEvents,
  desktopInfo,
  desktopSettings,
  desktopUpdate,
  runtimeAuditEvents,
} from './test/fixtures'
import type {
  AuditEvent,
  DesktopServiceReport,
  DesktopDatabaseActionResult,
  DesktopSettings,
  DesktopPortableSettings,
  DesktopSettingsExport,
  DesktopSettingsImport,
  DesktopSettingsUpdate,
  DesktopSourceJob,
  DesktopInstallJob,
  SourceSettings,
} from './types'

// The footer label tracks the installed desktop version; the fixture derives
// it from package.json so this matcher cannot drift from the release.
const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\$&')
const updatesButtonName = new RegExp(
  `Cortana ${escapeRegExp(desktopInfo.desktop_version)} · Updates`
)

afterEach(async () => {
  await act(async () => {
    // Unmount before flushing pending work so the shell's polling effects are
    // cancelled before a late promise can update the next test's renderer.
    cleanup()
    await new Promise((resolve) => setTimeout(resolve, 0))
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
})
afterEach(() => {
  window.localStorage.removeItem('cortana.workspace-selection.v1')
  window.localStorage.removeItem('cortana.source-selection.v1')
  window.localStorage.removeItem('cortana.theme.v1')
  state.getDocumentsCalls = []
  state.getGraphCalls = 0
  state.saveSettingsCalls = 0
  state.applySettingsUpdate = false
  state.lastSettingsUpdate = null
  state.exportDesktopSettingsCalls = 0
  state.importDesktopSettingsCalls = 0
  state.databaseBackupCalls = 0
  state.databaseRestoreCalls = 0
  state.databaseBackupResult = {
    action: 'backup',
    path: '/tmp/cortana-backup.sqlite3',
    bytes: 4096,
    detail: 'backup verified',
  }
  state.databaseRestoreResult = {
    action: 'restore',
    path: '/tmp/cortana-backup.sqlite3',
    bytes: 4096,
    detail: 'database restored',
  }
  state.exportDesktopSettingsResult = {
    path: '/tmp/cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    omitted_external_sources: [],
  }
  state.importDesktopSettingsResult = {
    path: '/tmp/imported-cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    preserved_external_sources: [],
    settings: buildImportedSettings('/tmp/imported-runtime-dir'),
  }
  state.getDesktopServicesCalls = 0
  state.getDesktopSettingsCalls = 0
  state.deferDesktopSettings = false
  state.deferredDesktopSettings = []
  state.getDesktopUpdateCalls = 0
  state.serviceStatusError = null
  state.serviceSyncInstallCalls = 0
  state.schedule = { sync_interval_seconds: 900, backup_interval_seconds: 86400 }
  state.scheduleGetCalls = 0
  state.scheduleSaveCalls = 0
  state.openSecretFileCalls = 0
  state.embeddingMigrationCalls = []
  state.openUrlCalls = []
  state.openUrlError = null
  state.openProjectCalls = 0
  state.openProjectError = null
})

// Desktop-mode App: the tauri bridge is mocked with resolved local settings,
// info, and audit sources so the settings/audit navigation is exercised.
const realApi = await import('./api')

const workSource: SourceSettings = {
  name: 'work-code',
  kind: 'filesystem',
  enabled: true,
  project: 'work',
  root: '/Users/you/code',
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
  labels: [],
  max_content_chars: null,
  max_documents: null,
  max_bytes: null,
  max_duration_seconds: null,
  exclude: [],
  acl: [],
  editable: true,
}

const googleSource: SourceSettings = {
  ...workSource,
  name: 'personal-drive',
  kind: 'google-drive',
  project: 'personal',
  root: null,
  token_env: 'GOOGLE_TOKEN_JSON',
  token_path: '/Users/you/.config/cortana/google-token.json',
  oauth_client_path: '/Users/you/Downloads/google-oauth-client.json',
}

const googleEnvOnlySource: SourceSettings = {
  ...googleSource,
  token_path: null,
}

const buildImportedSettings = (dataDir: string): DesktopPortableSettings => ({
  ...desktopSettings,
  runtime: {
    ...desktopSettings.runtime,
    data_dir: dataDir,
  },
})

const state = {
  settings: desktopSettings as DesktopSettings,
  sourceJob: null as DesktopSourceJob | null,
  authorizationCalls: [] as string[],
  embeddingMigrationCalls: [] as string[],
  openUrlCalls: [] as string[],
  openUrlError: null as Error | null,
  getDocumentsCalls: [] as Array<{
    workspace: string | undefined
    source: string | undefined
    query: string | undefined
    cursor: string | null | undefined
  }>,
  getGraphCalls: 0,
  statusCalls: 0,
  getDesktopSettingsCalls: 0,
  deferDesktopSettings: false,
  deferredDesktopSettings: [] as Array<(settings: DesktopSettings) => void>,
  getDesktopServicesCalls: 0,
  getDesktopUpdateCalls: 0,
  serviceStatusError: null as Error | null,
  saveSettingsCalls: 0,
  applySettingsUpdate: false,
  lastSettingsUpdate: null as DesktopSettingsUpdate | null,
  exportDesktopSettingsCalls: 0,
  importDesktopSettingsCalls: 0,
  exportDesktopSettingsResult: {
    path: '/tmp/cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    omitted_external_sources: [],
  } as DesktopSettingsExport,
  importDesktopSettingsResult: {
    path: '/tmp/imported-cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    preserved_external_sources: [],
    settings: buildImportedSettings('/tmp/imported-runtime-dir'),
  } as DesktopSettingsImport,
  databaseBackupCalls: 0,
  databaseRestoreCalls: 0,
  databaseBackupResult: {
    action: 'backup',
    path: '/tmp/cortana-backup.sqlite3',
    bytes: 4096,
    detail: 'backup verified',
  } as DesktopDatabaseActionResult,
  databaseRestoreResult: {
    action: 'restore',
    path: '/tmp/cortana-backup.sqlite3',
    bytes: 4096,
    detail: 'database restored',
  } as DesktopDatabaseActionResult,
  serviceInstallCalls: 0,
  serviceSyncInstallCalls: 0,
  schedule: { sync_interval_seconds: 900, backup_interval_seconds: 86400 },
  scheduleGetCalls: 0,
  scheduleSaveCalls: 0,
  serviceRestartCalls: 0,
  openSecretFileCalls: 0,
  openProjectCalls: 0,
  openProjectError: null as Error | null,
  serviceAction: null as (() => Promise<DesktopServiceReport>) | null,
  readinessScan: null as
    (() => Promise<Awaited<ReturnType<typeof realApi.scanDesktopReadiness>>>) | null,
  installerJob: null as DesktopInstallJob | null,
}

const serviceReport: DesktopServiceReport = {
  platform: 'macos',
  supported: true,
  services: [
    {
      name: 'embedding',
      label: 'ai.cortana.embedding',
      installed: false,
      loaded: false,
      state: null,
      pid: null,
      last_exit_status: null,
    },
    {
      name: 'server',
      label: 'ai.cortana.server',
      installed: false,
      loaded: false,
      state: null,
      pid: null,
      last_exit_status: null,
    },
    {
      name: 'sync',
      label: 'ai.cortana.sync',
      installed: false,
      loaded: false,
      state: null,
      pid: null,
      last_exit_status: null,
    },
    {
      name: 'backup',
      label: 'ai.cortana.backup',
      installed: false,
      loaded: false,
      state: null,
      pid: null,
      last_exit_status: null,
    },
  ],
}

const installedServiceReport: DesktopServiceReport = {
  ...serviceReport,
  services: serviceReport.services.map((service) =>
    service.name === 'sync'
      ? service
      : { ...service, installed: true, loaded: true, state: 'running' }
  ),
}

const syncInstalledServiceReport: DesktopServiceReport = {
  ...serviceReport,
  services: serviceReport.services.map((service) => ({
    ...service,
    installed: true,
    loaded: true,
    state: 'running',
  })),
}

mock.module('./api', () => ({
  isDesktopApp: true,
  isDemoMode: false,
  getStatus: () => {
    state.statusCalls += 1
    return Promise.resolve(demoStatus)
  },
  getDocuments: (workspace?: string, source?: string, query?: string, cursor?: string | null) => {
    state.getDocumentsCalls.push({
      workspace,
      source,
      query,
      cursor,
    })
    return Promise.resolve({ documents: [], next_cursor: null })
  },
  getGraph: () => {
    state.getGraphCalls += 1
    return Promise.resolve({ nodes: [], edges: [], next_cursor: null })
  },
  getAnswer: () => Promise.reject(new Error('Answer request failed (503)')),
  getDocument: () => Promise.reject(new Error('Document unavailable')),
  getContext: () => Promise.reject(new Error('Context retrieval failed (503)')),
  getDesktopSettings: () => {
    state.getDesktopSettingsCalls += 1
    if (state.deferDesktopSettings) {
      return new Promise<DesktopSettings>((resolve) => {
        state.deferredDesktopSettings.push(resolve)
      })
    }
    return Promise.resolve(state.settings)
  },
  saveDesktopSettings: (update: DesktopSettingsUpdate) => {
    state.saveSettingsCalls += 1
    state.lastSettingsUpdate = update
    if (state.applySettingsUpdate) {
      state.settings = { ...state.settings, ...update, secrets: state.settings.secrets }
    }
    return Promise.resolve(state.settings)
  },
  getDesktopInfo: () => Promise.resolve(desktopInfo),
  setDesktopAutostart: (enabled: boolean) =>
    Promise.resolve({ ...desktopInfo, autostart_enabled: enabled }),
  getDesktopSchedule: () => {
    state.scheduleGetCalls += 1
    return Promise.resolve(state.schedule)
  },
  saveDesktopSchedule: (schedule: {
    sync_interval_seconds: number
    backup_interval_seconds: number
  }) => {
    state.scheduleSaveCalls += 1
    state.schedule = schedule
    return Promise.resolve(schedule)
  },
  getDesktopServices: () => {
    state.getDesktopServicesCalls += 1
    if (state.serviceStatusError) return Promise.reject(state.serviceStatusError)
    return Promise.resolve(serviceReport)
  },
  openDesktopSecretFile: () => {
    state.openSecretFileCalls += 1
    return Promise.resolve()
  },
  openDesktopProject: () => {
    state.openProjectCalls += 1
    return state.openProjectError ? Promise.reject(state.openProjectError) : Promise.resolve()
  },
  getDesktopSourceJobs: () => Promise.resolve([]),
  installDesktopServices: () => {
    state.serviceInstallCalls += 1
    return Promise.resolve(installedServiceReport)
  },
  installDesktopSyncService: () => {
    state.serviceSyncInstallCalls += 1
    return Promise.resolve(syncInstalledServiceReport)
  },
  runDesktopServicesActionAll: (action: 'start' | 'stop' | 'restart') => {
    if (action === 'restart') state.serviceRestartCalls += 1
    return state.serviceAction ? state.serviceAction() : Promise.resolve(installedServiceReport)
  },
  runDesktopServiceAction: () => Promise.resolve(installedServiceReport),
  installDesktopUpdate: () => Promise.resolve(desktopUpdate),
  checkDesktopUpdate: () => Promise.resolve(desktopUpdate),
  getDesktopHindsightStatus: () =>
    Promise.resolve({
      enabled: false,
      configured: false,
      reachable: false,
      state: 'disabled' as const,
      endpoint: 'http://127.0.0.1:8888',
      bank: 'default',
      token_configured: false,
      detail: 'Optional sidecar is disabled; normal ingestion is unchanged.',
    }),
  getDesktopHonchoStatus: () =>
    Promise.resolve({
      enabled: false,
      configured: false,
      reachable: false,
      state: 'disabled' as const,
      endpoint: 'https://api.honcho.dev/',
      workspace_id: 'default',
      peer_id: 'cortana',
      token_configured: false,
      detail: 'Optional sidecar is disabled; normal ingestion is unchanged.',
    }),
  getRuntimeAudit: (limit: number) => Promise.resolve(runtimeAuditEvents.slice(0, limit)),
  getDesktopAudit: (limit: number) => Promise.resolve(desktopAuditEvents.slice(0, limit)),
  getDesktopUpdate: () => {
    state.getDesktopUpdateCalls += 1
    return Promise.resolve(desktopUpdate)
  },
  exportDesktopSettings: () => {
    state.exportDesktopSettingsCalls += 1
    return Promise.resolve(state.exportDesktopSettingsResult)
  },
  importDesktopSettings: () => {
    state.importDesktopSettingsCalls += 1
    return Promise.resolve(state.importDesktopSettingsResult)
  },
  backupDesktopDatabase: () => {
    state.databaseBackupCalls += 1
    return Promise.resolve(state.databaseBackupResult)
  },
  restoreDesktopDatabase: () => {
    state.databaseRestoreCalls += 1
    return Promise.resolve(state.databaseRestoreResult)
  },
  scanDesktopReadiness: () =>
    state.readinessScan
      ? state.readinessScan()
      : Promise.resolve({
          scanned_at_unix_seconds: 1785000000,
          platform: 'macos',
          tools_ready: false,
          core: null,
          core_error: null,
          tools: [
            {
              id: 'uv',
              label: 'uv',
              required: true,
              available: false,
              path: null,
              version: null,
              install_supported: true,
              detail: 'uv is not installed',
            },
          ],
        }),
  migrateDesktopEmbeddingGeneration: (from: string) => {
    state.embeddingMigrationCalls.push(from)
    return Promise.resolve('embedding generation migrated')
  },
  openDesktopUrl: (url: string) => {
    state.openUrlCalls.push(url)
    if (state.openUrlError) return Promise.reject(state.openUrlError)
    return Promise.resolve()
  },
  startDesktopInstaller: (tool: string) => {
    state.installerJob = {
      id: 'install-1',
      tool,
      status: 'running',
      summary: `Installing ${tool}`,
      log: '',
      started_at_unix_seconds: 1785000000,
      completed_at_unix_seconds: null,
      exit_code: null,
      retryable: false,
    }
    return Promise.resolve(state.installerJob)
  },
  getDesktopInstaller: () =>
    state.installerJob
      ? Promise.resolve(state.installerJob)
      : Promise.reject(new Error('installation job was not found')),
  cancelDesktopInstaller: () => {
    if (state.installerJob) state.installerJob = { ...state.installerJob, status: 'cancelling' }
    return Promise.resolve(state.installerJob!)
  },
  startDesktopSourceValidation: (source: string) => {
    const job: DesktopSourceJob = {
      id: 'job-validate-1',
      operation: 'validation',
      source,
      kind: 'filesystem',
      project: 'work',
      acl: ['work'],
      status: 'running',
      summary: 'Validating source work-code…',
      log: '',
      started_at_unix_seconds: 1785000000,
      completed_at_unix_seconds: null,
      exit_code: null,
      retryable: false,
      writes_indexed_data: false,
      budget: null,
    }
    state.sourceJob = job
    return Promise.resolve(job)
  },
  startDesktopSourceAuthorization: (source: string) => {
    state.authorizationCalls.push(source)
    const job: DesktopSourceJob = {
      id: 'job-authorize-1',
      operation: 'authorization',
      source,
      kind: 'google-drive',
      project: 'personal',
      acl: ['personal'],
      status: 'running',
      summary: 'Waiting for Google authorization in the system browser.',
      log: '',
      started_at_unix_seconds: 1785000000,
      completed_at_unix_seconds: null,
      exit_code: null,
      retryable: false,
      writes_indexed_data: false,
      budget: null,
    }
    state.sourceJob = job
    return Promise.resolve(job)
  },
  startDesktopSourceTrialSync: () => Promise.reject(new Error('trial sync unavailable')),
  openDesktopSourceSetup: () => Promise.reject(new Error('source setup unavailable')),
  listDesktopGithubRepositories: () => Promise.resolve({ truncated: false, repositories: [] }),
  listDesktopDiscordChannels: () => Promise.reject(new Error('Discord channels unavailable')),
  listDesktopDiscordServers: () => Promise.reject(new Error('Discord servers unavailable')),
  listDesktopSlackWorkspaces: () => Promise.reject(new Error('Slack workspaces unavailable')),
  listDesktopBuzzCommunities: () => Promise.reject(new Error('Buzz communities unavailable')),
  listDesktopProviderModels: (kind: 'embedding' | 'query') =>
    Promise.resolve({ kind, provider: 'local', models: [], truncated: false }),
  pickDesktopPath: () => Promise.resolve(null),
  planDesktopInitialSync: () => Promise.reject(new Error('initial sync unavailable')),
  startDesktopInitialSync: () => Promise.reject(new Error('initial sync unavailable')),
  getDesktopSourceValidation: (id: string) => {
    if (!state.sourceJob || state.sourceJob.id !== id) {
      return Promise.reject(new Error('source job was not found'))
    }
    return Promise.resolve(state.sourceJob)
  },
  cancelDesktopSourceValidation: (id: string) => {
    if (!state.sourceJob || state.sourceJob.id !== id) {
      return Promise.reject(new Error('source job was not found'))
    }
    state.sourceJob = {
      ...state.sourceJob,
      status: 'cancelling',
      summary: 'Cancelling source validation…',
    }
    return Promise.resolve(state.sourceJob)
  },
}))

const { App, ServiceHealthIndicator } = await import('./App')

async function flushDesktopBootstrap() {
  // The shell starts several independent control-plane reads on mount. Keep
  // those promise continuations inside React's act scope before asserting or
  // dispatching the first user event.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
}

test('global command shortcuts do not hijack editable fields', async () => {
  render(<App />)
  const search = await screen.findByRole('textbox', { name: 'Search your knowledge' })
  await flushDesktopBootstrap()

  act(() => fireEvent.keyDown(search, { key: 'p', ctrlKey: true }))
  expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull()

  act(() => fireEvent.keyDown(window, { key: 'p', ctrlKey: true }))
  expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeTruthy()

  act(() => fireEvent.keyDown(search, { key: 'Escape' }))
  expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull()
})

test('desktop shell restores workspace and source scope and clears stale selections', async () => {
  window.localStorage.setItem('cortana.workspace-selection.v1', 'work')
  window.localStorage.setItem('cortana.source-selection.v1', 'work-code')
  render(<App />)
  await waitFor(() => {
    expect((screen.getByRole('combobox') as HTMLSelectElement).value).toBe('work')
    expect(state.getDocumentsCalls.at(-1)?.source).toBe('work-code')
  })

  await act(async () => {
    cleanup()
    await Promise.resolve()
  })
  window.localStorage.setItem('cortana.workspace-selection.v1', 'work')
  window.localStorage.setItem('cortana.source-selection.v1', 'missing')
  render(<App />)
  await waitFor(() => {
    expect(state.getDocumentsCalls.at(-1)?.source).toBeUndefined()
    expect(window.localStorage.getItem('cortana.source-selection.v1')).toBeNull()
  })

  await act(async () => {
    cleanup()
    await Promise.resolve()
  })
  window.localStorage.setItem('cortana.workspace-selection.v1', 'missing')
  render(<App />)
  await waitFor(() => {
    const migrated = (screen.getByRole('combobox') as HTMLSelectElement).value
    expect(migrated).toBeTruthy()
    expect(window.localStorage.getItem('cortana.workspace-selection.v1')).toBe(migrated)
  })

  await act(async () => {
    cleanup()
    await Promise.resolve()
  })
  window.localStorage.setItem('cortana.workspace-selection.v1', 'personal')
  window.localStorage.setItem('cortana.source-selection.v1', 'work-code')
  render(<App />)
  await waitFor(() => {
    expect(state.getDocumentsCalls.at(-1)?.workspace).toBe('personal')
    expect(state.getDocumentsCalls.at(-1)?.source).toBeUndefined()
    expect(window.localStorage.getItem('cortana.source-selection.v1')).toBeNull()
  })

  await act(async () => {
    cleanup()
    await Promise.resolve()
  })
  window.localStorage.setItem('cortana.workspace-selection.v1', 'personal')
  window.localStorage.setItem('cortana.source-selection.v1', 'missing')
  render(<App />)
  await waitFor(() => {
    expect(state.getDocumentsCalls.at(-1)?.workspace).toBe('personal')
    expect(state.getDocumentsCalls.at(-1)?.source).toBeUndefined()
    expect(window.localStorage.getItem('cortana.source-selection.v1')).toBeNull()
  })
})

test('desktop shell ignores malformed persisted source scope', async () => {
  window.localStorage.setItem('cortana.workspace-selection.v1', 'work')
  window.localStorage.setItem('cortana.source-selection.v1', '   ')
  render(<App />)
  await waitFor(() => {
    expect(state.getDocumentsCalls.at(-1)?.source).toBeUndefined()
  })
})

test('desktop setup does not query documents before the control plane is ready', async () => {
  const originalSettings = state.settings
  try {
    state.settings = { ...desktopSettings, needs_setup: true }
    render(<App />)
    await flushDesktopBootstrap()

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    await waitFor(() => expect(screen.getByText('Index online')).toBeTruthy())
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Open service health' })).toBeTruthy()
    )
    await flushDesktopBootstrap()
    expect(state.getDocumentsCalls).toHaveLength(0)

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Graph' }))
      await Promise.resolve()
      await Promise.resolve()
    })
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'No graph data' })).toBeTruthy()
    )
    expect(state.getGraphCalls).toBe(0)
    await flushDesktopBootstrap()
  } finally {
    state.settings = originalSettings
  }
})

test('desktop shell pauses passive health polling while hidden and refreshes on restore', async () => {
  const descriptor = Object.getOwnPropertyDescriptor(document, 'visibilityState')
  const setVisibility = (value: 'hidden' | 'visible') => {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value,
    })
    document.dispatchEvent(new Event('visibilitychange'))
  }
  try {
    await act(async () => {
      setVisibility('visible')
      await Promise.resolve()
    })
    render(<App />)
    await flushDesktopBootstrap()
    await waitFor(() => expect(state.getDesktopServicesCalls).toBeGreaterThan(0))
    const servicesBeforeHidden = state.getDesktopServicesCalls
    const statusBeforeHidden = state.statusCalls

    await act(async () => {
      setVisibility('hidden')
      await Promise.resolve()
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 25))
    })
    expect(state.getDesktopServicesCalls).toBe(servicesBeforeHidden)
    expect(state.statusCalls).toBe(statusBeforeHidden)

    // The webview may become visible before native focus is restored. That
    // event alone must not restart passive polling in the background.
    await act(async () => {
      window.dispatchEvent(new Event('blur'))
      setVisibility('visible')
      await Promise.resolve()
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 25))
    })
    expect(state.getDesktopServicesCalls).toBe(servicesBeforeHidden)
    expect(state.statusCalls).toBe(statusBeforeHidden)

    await act(async () => {
      window.dispatchEvent(new Event('focus'))
      await Promise.resolve()
    })
    await waitFor(() => expect(state.getDesktopServicesCalls).toBeGreaterThan(servicesBeforeHidden))
    expect(state.statusCalls).toBeGreaterThan(statusBeforeHidden)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
    await flushDesktopBootstrap()
  } finally {
    if (descriptor) Object.defineProperty(document, 'visibilityState', descriptor)
    else setVisibility('visible')
  }
})

test('desktop settings navigation opens the audit trail and renders both event sources', async () => {
  render(<App />)

  // Desktop chrome: version and updates shortcut live in the footer.
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())

  // Rail navigation into the settings view.
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  expect(screen.getByText('Control plane')).toBeTruthy()
  const save = screen.getByRole('button', { name: 'Save changes' })
  expect(save).toBeTruthy()
  expect(save.hasAttribute('disabled')).toBe(true)
  fireEvent.submit(document.getElementById('settings-form')!)
  expect(state.saveSettingsCalls).toBe(0)

  // Section navigation into the audit trail.
  fireEvent.click(screen.getByRole('button', { name: 'Audit' }))
  await waitFor(() => expect(screen.getByText('2 runtime · 1 Desktop events')).toBeTruthy())
  expect(state.saveSettingsCalls).toBe(0)
  expect(screen.getByText('Runtime retrieval')).toBeTruthy()
  expect(screen.getByText('Desktop actions')).toBeTruthy()
  expect(screen.getByText('brain_answer')).toBeTruthy()
  expect(screen.getByText('brain_documents')).toBeTruthy()
  expect(screen.getByText('settings_saved')).toBeTruthy()

  // Refreshing keeps the audit list stable.
  fireEvent.click(screen.getByRole('button', { name: /Refresh/ }))
  await waitFor(() => expect(screen.getByText('2 runtime · 1 Desktop events')).toBeTruthy())
})

test('audit trail export downloads exactly the loaded redacted events as JSON', async () => {
  // Capture the browser download plumbing instead of letting happy-dom resolve
  // blob URLs, so the payload, filename, and cleanup are all observable.
  const originalCreateObjectURL = URL.createObjectURL
  const originalRevokeObjectURL = URL.revokeObjectURL
  const originalCreateElement = document.createElement.bind(document)
  const downloads: Array<{ blob: Blob; url: string }> = []
  const revoked: string[] = []
  const anchors: HTMLAnchorElement[] = []
  URL.createObjectURL = (blob: Blob) => {
    const url = `blob:test:${downloads.length}`
    downloads.push({ blob, url })
    return url
  }
  URL.revokeObjectURL = (url: string) => {
    revoked.push(url)
  }
  document.createElement = (tag: string, options?: ElementCreationOptions) => {
    const element = originalCreateElement(tag, options)
    if (tag === 'a') anchors.push(element as HTMLAnchorElement)
    return element
  }
  try {
    render(<App />)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Audit' }))
    await waitFor(() => expect(screen.getByText('2 runtime · 1 Desktop events')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Export' }))

    // One JSON blob is offered with the exact events already on screen — no
    // additional fields, no secret material beyond the redacted snapshots.
    expect(downloads).toHaveLength(1)
    const { blob, url } = downloads[0]
    expect(blob.type).toBe('application/json')
    const payload = JSON.parse(await blob.text()) as {
      exported_at: string
      runtime: AuditEvent[]
      desktop: AuditEvent[]
    }
    expect(payload.runtime).toEqual(runtimeAuditEvents)
    expect(payload.desktop).toEqual(desktopAuditEvents)
    expect(Object.keys(payload).sort()).toEqual(['desktop', 'exported_at', 'runtime'])
    expect(Number.isNaN(Date.parse(payload.exported_at))).toBe(false)

    // Deterministic, date-stamped filename on the triggered anchor.
    expect(anchors).toHaveLength(1)
    expect(anchors[0].download).toMatch(/^cortana-audit-\d{4}-\d{2}-\d{2}\.json$/)
    expect(anchors[0].href).toBe(url)

    // The object URL is released after the download click is processed.
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)))
    expect(revoked).toEqual([url])
  } finally {
    URL.createObjectURL = originalCreateObjectURL
    URL.revokeObjectURL = originalRevokeObjectURL
    document.createElement = originalCreateElement
  }
})

test('advanced settings export is blocked while draft is dirty', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))

  const dataDir = screen.getByLabelText('Data directory') as HTMLInputElement
  fireEvent.change(dataDir, { target: { value: '/tmp/dirty-runtime-directory' } })
  expect(dataDir.value).toBe('/tmp/dirty-runtime-directory')

  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton.hasAttribute('disabled')).toBe(true)
  fireEvent.click(exportButton)
  expect(state.exportDesktopSettingsCalls).toBe(0)

  const saveChanges = screen.getByRole('button', { name: 'Save changes' })
  expect(saveChanges.hasAttribute('disabled')).toBe(false)
  expect(state.saveSettingsCalls).toBe(0)
})

test('advanced settings export shows redacted notice and calls the export bridge when clean', async () => {
  state.exportDesktopSettingsResult = {
    path: '/tmp/cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    omitted_external_sources: ['s3-uploader'],
  }
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))

  const exportButton = screen.getByRole('button', { name: 'Export' })
  expect(exportButton.hasAttribute('disabled')).toBe(false)

  fireEvent.click(exportButton)
  await waitFor(() => {
    const status = screen.getByRole('status') as HTMLElement
    expect(status.textContent).toContain(
      'Redacted settings exported to /tmp/cortana-settings.toml.'
    )
    expect(status.textContent).toContain('Executable connectors omitted: s3-uploader.')
  })
  expect(state.exportDesktopSettingsCalls).toBe(1)
})

test('advanced import preview cancellation keeps draft values unchanged', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => false
  state.importDesktopSettingsResult = {
    path: '/tmp/imported-cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    preserved_external_sources: ['s3-uploader'],
    settings: buildImportedSettings('/tmp/imported-runtime-dir'),
  }
  try {
    render(<App />)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))

    const dataDir = screen.getByLabelText('Data directory') as HTMLInputElement
    fireEvent.change(dataDir, { target: { value: '/tmp/dirty-draft' } })
    expect(dataDir.value).toBe('/tmp/dirty-draft')

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Import preview' }))
      await Promise.resolve()
    })
    await waitFor(() => expect(state.importDesktopSettingsCalls).toBe(1))

    expect(dataDir.value).toBe('/tmp/dirty-draft')
    expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(
      false
    )
    await flushDesktopBootstrap()
  } finally {
    window.confirm = originalConfirm
  }
})

test('advanced settings import preview applies as unsaved draft and requires explicit save', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.importDesktopSettingsResult = {
    path: '/tmp/imported-cortana-settings.toml',
    format_version: 2,
    secrets_included: false,
    preserved_external_sources: ['s3-uploader'],
    settings: buildImportedSettings('/tmp/imported-runtime-dir'),
  }
  try {
    render(<App />)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))

    fireEvent.click(screen.getByRole('button', { name: 'Import preview' }))
    await waitFor(() =>
      expect(
        screen.getByText(
          'Imported settings are ready for review. Preserved executable connectors: s3-uploader.'
        )
      ).toBeTruthy()
    )

    expect(state.importDesktopSettingsCalls).toBe(1)
    expect(state.saveSettingsCalls).toBe(0)

    const dataDir = screen.getByLabelText('Data directory') as HTMLInputElement
    expect(dataDir.value).toBe('/tmp/imported-runtime-dir')

    const saveChanges = screen.getByRole('button', { name: 'Save changes' })
    expect(saveChanges.hasAttribute('disabled')).toBe(false)
    fireEvent.click(saveChanges)
    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
    expect(state.lastSettingsUpdate?.runtime.data_dir).toBe('/tmp/imported-runtime-dir')
  } finally {
    window.confirm = originalConfirm
  }
})

test('updates project link surfaces native browser failures', async () => {
  const originalError = state.openProjectError
  state.openProjectError = new Error('browser unavailable')
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Updates' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Updates' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'View Cortana source on GitHub' }))
    await waitFor(() => expect(screen.getByText('browser unavailable')).toBeTruthy())
    expect(state.openProjectCalls).toBe(1)
  } finally {
    state.openProjectError = originalError
  }
})

test('desktop shell surfaces optional sidecar health without opening settings', async () => {
  render(<App />)
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Open Hindsight status' })).toBeTruthy()
  )
  expect(screen.getByRole('button', { name: 'Open Honcho status' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Open service health' })).toBeTruthy()
  expect(screen.getByText('Services: core attention')).toBeTruthy()
  expect(screen.getByText('Hindsight: disabled')).toBeTruthy()
  expect(screen.getByText('Honcho: disabled')).toBeTruthy()
})

test('desktop Help links use the native external URL bridge', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Help' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Help' }))
  const documentation = screen.getByRole('link', { name: /Documentation/ })
  fireEvent.click(documentation)
  await waitFor(() =>
    expect(state.openUrlCalls).toEqual(['https://github.com/0xPlayerOne/cortana/tree/main/docs'])
  )
})

test('desktop Help links surface native browser failures', async () => {
  state.openUrlError = new Error('browser unavailable')
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Help' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Help' }))
  fireEvent.click(screen.getByRole('link', { name: /Documentation/ }))
  await waitFor(() => expect(screen.getByText('browser unavailable')).toBeTruthy())
  expect(state.openUrlCalls).toEqual(['https://github.com/0xPlayerOne/cortana/tree/main/docs'])
})

test('desktop Help project action surfaces native browser failures', async () => {
  state.openProjectError = new Error('browser unavailable')
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Help' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Help' }))
  fireEvent.click(screen.getByRole('button', { name: 'Open project page' }))
  await waitFor(() => expect(screen.getByText('browser unavailable')).toBeTruthy())
  expect(state.openProjectCalls).toBe(1)
})

test('desktop shell does not present a stale service report after refresh failure', () => {
  render(
    <ServiceHealthIndicator
      report={installedServiceReport}
      error="service status transport failed"
      onOpen={() => {}}
    />
  )

  expect(screen.getByText('Services: unavailable')).toBeTruthy()
  const health = screen.getByRole('button', { name: 'Open service health' })
  expect(health.getAttribute('title')).toBeNull()
  expect(health.getAttribute('data-tooltip')).toContain('service status transport failed')
})

test('desktop shell does not require the local embedding service for cloud embeddings', () => {
  render(
    <ServiceHealthIndicator
      report={{
        ...installedServiceReport,
        services: installedServiceReport.services.map((service) =>
          service.name === 'embedding'
            ? { ...service, installed: false, loaded: false, state: null }
            : service
        ),
      }}
      error=""
      embeddingRequired={false}
      onOpen={() => {}}
    />
  )

  expect(screen.getByText('Services: core 1/1 online')).toBeTruthy()
})

test('query number fields keep drafts inside native bounds', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Query' }))

  const retrieval = screen.getByLabelText('Retrieval candidates') as HTMLInputElement
  fireEvent.change(retrieval, { target: { value: '999' } })
  expect(retrieval.value).toBe('100')
  fireEvent.change(retrieval, { target: { value: '1.5' } })
  expect(retrieval.value).toBe('100')

  const cacheEntries = screen.getByLabelText(/Cache entries/) as HTMLInputElement
  fireEvent.change(cacheEntries, { target: { value: '0' } })
  expect(cacheEntries.value).toBe('0')
  const cacheLifetime = screen.getByLabelText(/^Cache lifetime \(seconds\)/) as HTMLInputElement
  fireEvent.change(cacheLifetime, { target: { value: '0' } })
  expect(cacheLifetime.value).toBe('0')
})

test('embedding settings explain local service command ownership', async () => {
  const originalSettings = state.settings
  state.settings = { ...desktopSettings, embedding_service_program: '/opt/text-embeddings-router' }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Embedding' }))
    expect(
      screen.getByText(/\/opt\/text-embeddings-router \(managed in config\.toml\)/)
    ).toBeTruthy()
    expect(screen.getByText(/does not edit shell command arrays/i)).toBeTruthy()
  } finally {
    state.settings = originalSettings
  }
})

test('embedding model field supports preset catalog with custom fallback', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    embedding: {
      ...desktopSettings.embedding,
      provider: 'local',
      model: 'Qwen/Qwen3-Embedding-0.6B',
      base_url: 'http://127.0.0.1:6999/v1',
    },
  }

  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Embedding' }))

    const catalog = screen.getByRole('combobox', { name: 'Model catalog' })
    expect((catalog as HTMLSelectElement).value).toBe('Qwen/Qwen3-Embedding-0.6B')

    fireEvent.change(catalog, { target: { value: 'Custom' } })
    const custom = screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement
    fireEvent.change(custom, { target: { value: 'local/custom-model' } })
    expect(custom.value).toBe('local/custom-model')
  } finally {
    state.settings = originalSettings
  }
})

test('query model field preserves custom text until provider discovery', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    query: {
      ...desktopSettings.query,
      provider: 'cloud',
      model: 'provider-custom-embedding',
      base_url: 'https://api.openai.com/v1',
    },
  }

  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Query' }))

    const custom = screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement
    fireEvent.change(custom, { target: { value: 'custom-model-name' } })
    expect(custom.value).toBe('custom-model-name')
  } finally {
    state.settings = originalSettings
  }
})

test('settings add controls avoid reusing removed identifiers', async () => {
  const originalSettings = state.settings
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.settings = {
    ...desktopSettings,
    workspaces: [
      { id: 'workspace-1', name: 'One', account_label: null, color: '#5A9BD5' },
      { id: 'workspace-2', name: 'Two', account_label: null, color: '#E8A83B' },
    ],
    sources: [
      { ...workSource, name: 'source-1' },
      { ...workSource, name: 'source-2' },
    ],
    auth_principals: [
      {
        principal: 'agent-1',
        token_env: 'CORTANA_AGENT_1_TOKEN',
        scopes: ['query', 'status'],
        acl: ['workspace-1'],
      },
      {
        principal: 'agent-2',
        token_env: 'CORTANA_AGENT_2_TOKEN',
        scopes: ['query', 'status'],
        acl: ['workspace-2'],
      },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )

    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove One' }))
    fireEvent.click(screen.getByRole('button', { name: /Add workspace/ }))
    expect(
      (screen.getAllByLabelText(/Scope ID/) as HTMLInputElement[]).map((input) => input.value)
    ).toContain('new-workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove source-1' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add source' }))
    fireEvent.click(screen.getByRole('tab', { name: /Two/ }))
    expect(
      (screen.getAllByLabelText(/Source name/) as HTMLInputElement[]).map((input) => input.value)
    ).toContain('source-1')

    fireEvent.click(screen.getByRole('button', { name: 'Access' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add principal' }))
    expect(
      (screen.getAllByLabelText('Principal name') as HTMLInputElement[]).map((input) => input.value)
    ).toContain('agent-3')
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
  }
})

test('workspace controls protect scopes assigned to sources', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [{ ...workSource, project: 'work' }],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))

    expect(
      (screen.getByRole('button', { name: 'Remove Work' }) as HTMLButtonElement).disabled
    ).toBe(true)
    expect((screen.getAllByLabelText(/Scope ID/)[0] as HTMLInputElement).disabled).toBe(true)
  } finally {
    state.settings = originalSettings
  }
})

test('workspace cards show display name and advanced details', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    workspaces: [{ id: 'work', name: 'Work', account_label: 'team@example.com', color: '#5A9BD5' }],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))

    fireEvent.click(screen.getByText('Advanced workspace details'))
    expect(screen.getByText('ID is internal; account labels are optional metadata.')).toBeTruthy()
    expect(screen.getByLabelText(/Scope ID/i)).toBeTruthy()
    const accountLabel = screen.getByLabelText(/Account label/i)
    expect(accountLabel).toBeTruthy()
    expect(accountLabel.getAttribute('placeholder')).toBe('e.g. Nifty League')
    expect(screen.getByDisplayValue('Work')).toBeTruthy()
    expect((screen.getByLabelText(/Scope ID/i) as HTMLInputElement).readOnly).toBe(true)
    const upload = screen.getByLabelText('Upload logo for Work') as HTMLInputElement
    expect(upload.accept).toContain('image/png')
  } finally {
    state.settings = originalSettings
  }
})

test('settings warns before discarding dirty changes', async () => {
  const originalConfirm = window.confirm
  const responses = [false, true]
  window.confirm = () => responses.shift() ?? true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Draft work' },
    })
    expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(
      false
    )

    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  } finally {
    window.confirm = originalConfirm
  }
})

test('settings can discard a draft without leaving the control plane', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  window.confirm = () => true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    const displayName = screen.getAllByLabelText('Display name')[0] as HTMLInputElement
    fireEvent.change(displayName, { target: { value: 'Draft work' } })
    expect(screen.getByRole('button', { name: 'Discard' })).toBeTruthy()

    // Make the native reload differ from the shell snapshot. Discard must
    // reconcile both so remounting Settings cannot resurrect the old draft.
    state.settings = {
      ...originalSettings,
      workspaces: originalSettings.workspaces.map((workspace, index) =>
        index === 0 ? { ...workspace, name: 'Reloaded work' } : workspace
      ),
    }
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }))
    await waitFor(() =>
      expect((screen.getAllByLabelText('Display name')[0] as HTMLInputElement).value).toBe(
        'Reloaded work'
      )
    )
    expect(screen.queryByRole('button', { name: 'Discard' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(true)

    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    await waitFor(() =>
      expect((screen.getAllByLabelText('Display name')[0] as HTMLInputElement).value).toBe(
        'Reloaded work'
      )
    )
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
  }
})

test('late desktop bootstrap settings cannot overwrite a shell-reconciled snapshot', async () => {
  const originalSettings = state.settings
  const reloadedSettings = {
    ...originalSettings,
    workspaces: originalSettings.workspaces.map((workspace, index) =>
      index === 0 ? { ...workspace, name: 'Reloaded work' } : workspace
    ),
  }
  state.deferDesktopSettings = true
  try {
    render(<App />)
    await waitFor(() => expect(state.deferredDesktopSettings.length).toBe(1))

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(state.deferredDesktopSettings.length).toBe(2))

    // Settings completes its own read first; the App bootstrap request then
    // resolves with the stale snapshot it started with.
    state.deferredDesktopSettings[1]!(reloadedSettings)
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    state.deferredDesktopSettings[0]!(originalSettings)

    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    await waitFor(() =>
      expect((screen.getAllByLabelText('Display name')[0] as HTMLInputElement).value).toBe(
        'Reloaded work'
      )
    )
  } finally {
    state.deferDesktopSettings = false
    state.deferredDesktopSettings = []
    state.settings = originalSettings
  }
})

test('the footer updates shortcut opens the updates section directly', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: updatesButtonName }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  await waitFor(() => expect(screen.getByText('Version 9.9.9 is available')).toBeTruthy())
  expect(screen.getByText('Installed version')).toBeTruthy()
  expect(screen.getByRole('button', { name: /Install and restart/ })).toBeTruthy()

  // Back to the knowledge workspace via the rail.
  fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
})

test('updates section renders release markdown safely', async () => {
  const originalNotes = desktopUpdate.release_notes
  const originalChangelog = desktopUpdate.changelog
  desktopUpdate.release_notes =
    '# Release Notes\n\n- Indexed local Q&A\n- Added [dashboard](https://example.com/help)\n\n`inline` code'
  desktopUpdate.changelog = '### Changelog\n\n1. Added feature\n2. Fixed bugs'

  try {
    render(<App />)
    fireEvent.click(screen.getByRole('button', { name: /· Updates/ }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Updates' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Updates' }))

    expect(screen.getByRole('heading', { name: 'Release Notes' })).toBeTruthy()
    expect(screen.getByText('Indexed local Q&A')).toBeTruthy()
    const link = screen.getByRole('link', { name: 'dashboard' }) as HTMLAnchorElement
    expect(link.href).toBe('https://example.com/help')
    expect(screen.getByText('inline')).toBeTruthy()
  } finally {
    desktopUpdate.release_notes = originalNotes
    desktopUpdate.changelog = originalChangelog
  }
})

test('the footer updates shortcut respects unsaved settings changes', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => false
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Unsaved workspace' },
    })
    fireEvent.click(screen.getByRole('button', { name: updatesButtonName }))
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeTruthy()
    expect(screen.queryByText('Installed version')).toBeNull()
  } finally {
    window.confirm = originalConfirm
  }
})

test('source settings opens the Sources section directly', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())

  fireEvent.click(screen.getByLabelText('Source settings'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )

  const sources = screen.getByRole('button', { name: 'Sources' })
  expect(sources.className).toContain('active')
})

test('source settings show a compact workspace-first row with collapsed advanced controls', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [
      workSource,
      { ...workSource, name: 'personal-notes', project: 'personal', enabled: false },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))

    expect(screen.getByText('Files & code')).toBeTruthy()
    expect(screen.getByText('Work · Enabled')).toBeTruthy()
    expect(screen.queryByText('Personal · Disabled')).toBeNull()
    expect(screen.getByRole('img', { name: 'Files and code connector' })).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: /Personal/ }))
    expect(screen.getByText('Personal · Disabled')).toBeTruthy()
    expect(screen.queryByText('Work · Enabled')).toBeNull()
    fireEvent.click(screen.getByRole('tab', { name: /Work/ }))
    const summary = screen.getByText('Advanced source settings')
    const details = summary.closest('details') as HTMLDetailsElement
    expect(details.open).toBe(false)
    fireEvent.click(summary)
    expect(details.open).toBe(true)
    expect(screen.getByLabelText('Source label')).toBeTruthy()
  } finally {
    state.settings = originalSettings
  }
})

test('Apple Notes sources expose exact include and exclude folder filters', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [
      {
        ...workSource,
        name: 'work-notes',
        kind: 'apple-notes',
        folders: ['Nifty League'],
        exclude_folders: ['The Pink Binder'],
      },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    fireEvent.click(screen.getByText('Advanced source settings'))

    expect(
      (screen.getByRole('textbox', { name: 'Include Apple Notes folders' }) as HTMLTextAreaElement)
        .value
    ).toBe('Nifty League')
    expect(
      (screen.getByRole('textbox', { name: 'Exclude Apple Notes folders' }) as HTMLTextAreaElement)
        .value
    ).toBe('The Pink Binder')
  } finally {
    state.settings = originalSettings
  }
})

test('source settings quarantine legacy scopes until they are assigned to a workspace', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [
      {
        ...workSource,
        name: 'community-discord',
        kind: 'discord',
        project: 'community',
        enabled: false,
      },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))

    fireEvent.click(screen.getByRole('tab', { name: /Needs assignment/ }))
    expect(screen.getByRole('alert').textContent).toContain('uses the legacy community scope')
    expect((screen.getByRole('checkbox') as HTMLInputElement).disabled).toBe(true)
    expect((screen.getByRole('button', { name: 'Validate' }) as HTMLButtonElement).disabled).toBe(
      true
    )
    expect(
      (screen.getByRole('button', { name: 'Initial sync' }) as HTMLButtonElement).disabled
    ).toBe(true)

    const workspace = screen.getByRole('combobox', {
      name: 'Workspace for community-discord',
    }) as HTMLSelectElement
    expect(workspace.options[0]?.textContent).toBe('Unassigned: community')
    fireEvent.change(workspace, { target: { value: 'work' } })

    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull())
    expect((screen.getByRole('checkbox') as HTMLInputElement).disabled).toBe(false)
  } finally {
    state.settings = originalSettings
  }
})

test('GitHub code sources expose an explicit workspace-scoped repository allowlist', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [
      {
        ...workSource,
        name: 'work-github',
        kind: 'github',
        repositories: ['0xPlayerOne/cortana'],
        token_env: 'GITHUB_TOKEN',
      },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    fireEvent.click(screen.getByText('Advanced source settings'))

    const repositories = screen.getByRole('textbox', { name: 'GitHub repositories' })
    expect((repositories as HTMLTextAreaElement).value).toBe('0xPlayerOne/cortana')
    expect(screen.getByRole('button', { name: 'Setup' })).toBeTruthy()
    expect(screen.getByText(/only these repositories are indexed/)).toBeTruthy()
  } finally {
    state.settings = originalSettings
  }
})

test('source tree toggles a saved connector without touching indexed data', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  window.confirm = () => true
  state.applySettingsUpdate = true
  state.settings = { ...desktopSettings, sources: [{ ...workSource, enabled: false }] }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByLabelText('Open sources'))
    const toggle = await screen.findByRole('switch', { name: 'Enable work-code' })
    fireEvent.click(toggle)

    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
    expect(screen.getByText('Source setting saved for future ingestion.')).toBeTruthy()
    expect(state.lastSettingsUpdate?.sources).toEqual([
      expect.objectContaining({ name: 'work-code', project: 'work', enabled: true }),
    ])
    expect(state.lastSettingsUpdate?.secrets).toEqual([])
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
    state.applySettingsUpdate = false
    state.lastSettingsUpdate = null
  }
})

test('settings refuses duplicate canonical source labels in one workspace', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [
      { ...workSource, source: null },
      { ...workSource, name: 'work-drive', root: '/Users/you/drive', source: 'work-code' },
    ],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Draft workspace' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))

    await waitFor(() =>
      expect(screen.getByText(/Source identifier `work-code` is duplicated/)).toBeTruthy()
    )
    expect(state.saveSettingsCalls).toBe(0)
  } finally {
    state.settings = originalSettings
  }
})

test('settings navigation opens workspace and services first and groups plugins', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Settings' })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())

  const navigation = screen.getByRole('navigation', { name: 'Settings sections' })
  const buttons = within(navigation).getAllByRole('button')
  const labels = buttons.map((button) => button.textContent)
  expect(labels[0]).toBe('Services')
  expect(labels[1]).toBe('Workspaces')
  expect(labels[2]).toBe('Sources')
  expect(labels[3]).toBe('Readiness')

  fireEvent.click(screen.getByRole('button', { name: 'Plugins' }))
  expect(screen.getByRole('button', { name: 'Plugins' }).className).toContain('active')
  fireEvent.click(screen.getByRole('button', { name: 'Hindsight' }))
  expect(screen.getByRole('button', { name: 'Plugins' }).className).toContain('active')
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Hindsight memory sidecar' })).toBeTruthy()
  )
})

test('settings theme control updates and persists', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Settings' })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())

  const themeSelect = await screen.findByLabelText('Theme')
  expect(themeSelect).toBeTruthy()
  fireEvent.change(themeSelect, { target: { value: 'accessible' } })

  expect(window.localStorage.getItem('cortana.theme.v1')).toBe('accessible')
  expect(document.documentElement.getAttribute('data-theme')).toBe('accessible')
})

test('settings refuses padded or control-character source labels before save', async () => {
  const originalSettings = state.settings
  state.settings = {
    ...desktopSettings,
    sources: [{ ...workSource, source: ' work-code ' }],
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Draft workspace' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))

    await waitFor(() =>
      expect(screen.getByText(/Source label for `work-code` must not be empty/)).toBeTruthy()
    )
    expect(state.saveSettingsCalls).toBe(0)
  } finally {
    state.settings = originalSettings
  }
})
test('source tree actions resolve a configured source by its canonical label', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  const originalConfiguredSources = demoStatus.ingestion.configured_sources
  const originalIndexedSources = demoStatus.sources
  window.confirm = () => true
  state.applySettingsUpdate = true
  const labeledSource = { ...workSource, source: 'code-label', enabled: false }
  state.settings = { ...desktopSettings, sources: [labeledSource] }
  demoStatus.ingestion = {
    ...demoStatus.ingestion,
    configured_sources: demoStatus.ingestion.configured_sources.map((item) =>
      item.name === 'work-code' ? { ...item, source: 'code-label', enabled: false } : item
    ),
  }
  demoStatus.sources = demoStatus.sources.map((item) =>
    item.source === 'work-code' ? { ...item, source: 'code-label' } : item
  )
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByLabelText('Open sources'))
    const toggle = await screen.findByRole('switch', { name: 'Enable work-code' })
    fireEvent.click(toggle)

    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
    expect(screen.getByText('Source setting saved for future ingestion.')).toBeTruthy()
    expect(state.lastSettingsUpdate?.sources).toEqual([
      expect.objectContaining({ name: 'work-code', source: 'code-label', enabled: true }),
    ])
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
    state.applySettingsUpdate = false
    state.lastSettingsUpdate = null
    demoStatus.ingestion = {
      ...demoStatus.ingestion,
      configured_sources: originalConfiguredSources,
    }
    demoStatus.sources = originalIndexedSources
  }
})

test('Services settings stay a process-health surface with no source enablement controls', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Services' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

  // Process health actions are the Services surface.
  expect(screen.getByRole('button', { name: /Start all/ })).toBeTruthy()
  expect(screen.getByRole('button', { name: /Stop all/ })).toBeTruthy()
  expect(screen.getByRole('button', { name: /Restart all/ })).toBeTruthy()

  // No per-source enable/disable control lives in Services: no switch, and
  // the only checkbox is the desktop autostart launch preference, which is
  // process/launch behavior rather than source enablement.
  expect(screen.queryByRole('switch')).toBeNull()
  const checkboxes = screen.getAllByRole('checkbox')
  expect(checkboxes).toHaveLength(1)
  expect(screen.getByRole('checkbox', { name: /Open Cortana Desktop at login/ })).toBeTruthy()
  await flushDesktopBootstrap()
})

test('services settings offers an explicit safe core-service install', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.serviceInstallCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Install core services/ })).toBeTruthy()
    )
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install core services/ }))
      await Promise.resolve()
    })
    await waitFor(() => expect(state.serviceInstallCalls).toBe(1))
    expect(screen.getByText('3 loaded')).toBeTruthy()
    expect(screen.getByText(/Recurring sync is opt-in/)).toBeTruthy()
    expect(screen.getByRole('button', { name: /Enable recurring sync/ })).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
  }
})

test('services settings enables recurring sync only through its explicit action', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.serviceSyncInstallCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    const enable = await screen.findByRole('button', { name: /Enable recurring sync/ })
    fireEvent.click(enable)
    await waitFor(() => expect(state.serviceSyncInstallCalls).toBe(1))
    expect(screen.getByText('4 loaded')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /Enable recurring sync/ })).toBeNull()
  } finally {
    window.confirm = originalConfirm
  }
})

test('services settings saves bounded recurring sync and backup intervals', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Services' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
  await waitFor(() => expect(state.scheduleGetCalls).toBe(1))
  await waitFor(() => expect(screen.getByText('Background schedule')).toBeTruthy())
  const syncInterval = await screen.findByLabelText('Sync interval (seconds)')
  fireEvent.change(syncInterval, { target: { value: '1800' } })
  fireEvent.click(screen.getByRole('button', { name: /Save schedule/ }))
  await waitFor(() => expect(state.scheduleSaveCalls).toBe(1))
  expect(state.schedule.sync_interval_seconds).toBe(1800)
  expect(state.schedule.backup_interval_seconds).toBe(86400)
})

test('services settings requires explicit apply after changing an installed schedule', async () => {
  const originalConfirm = window.confirm
  const originalServices = serviceReport.services.map((service) => ({ ...service }))
  window.confirm = () => true
  serviceReport.services = serviceReport.services.map((service) =>
    service.name === 'sync'
      ? { ...service, installed: true, loaded: true, state: 'running' }
      : service
  )
  state.serviceSyncInstallCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    const syncInterval = await screen.findByLabelText('Sync interval (seconds)')
    fireEvent.change(syncInterval, { target: { value: '1800' } })
    fireEvent.click(screen.getByRole('button', { name: /Save schedule/ }))
    const apply = await screen.findByRole('button', { name: /Apply recurring sync schedule/ })
    fireEvent.click(apply)
    await waitFor(() => expect(state.serviceSyncInstallCalls).toBe(1))
  } finally {
    window.confirm = originalConfirm
    serviceReport.services.splice(0, serviceReport.services.length, ...originalServices)
  }
})

test('services settings refuses recurring sync while settings changes are unsaved', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.serviceSyncInstallCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Unsaved workspace' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    const enable = await screen.findByRole('button', { name: /Enable recurring sync/ })
    fireEvent.click(enable)

    await waitFor(() =>
      expect(screen.getByText(/Save changes before enabling recurring sync/)).toBeTruthy()
    )
    expect(state.serviceSyncInstallCalls).toBe(0)
  } finally {
    window.confirm = originalConfirm
  }
})

test('services settings reuses the shell service snapshot without a duplicate poll', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  await waitFor(() => expect(state.getDesktopServicesCalls).toBe(1))

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Services' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

  expect(state.getDesktopServicesCalls).toBe(1)
})

test('services settings exports a verified database backup with explicit confirmation', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.databaseBackupCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Backup database' }))
    await waitFor(() => expect(state.databaseBackupCalls).toBe(1))
    expect(
      screen.getByText(/Verified backup exported to \/tmp\/cortana-backup\.sqlite3/)
    ).toBeTruthy()
    expect(screen.getByText(/4096 bytes/)).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
  }
})

test('services settings permits restore with an installed but idle backup job and blocks running core services', async () => {
  const originalConfirm = window.confirm
  const originalServices = serviceReport.services.map((service) => ({ ...service }))
  window.confirm = () => true
  state.databaseRestoreCalls = 0
  serviceReport.services[3] = {
    ...serviceReport.services[3],
    installed: true,
    loaded: true,
    state: 'not running',
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

    const restore = screen.getByRole('button', { name: 'Restore database' })
    expect(restore.hasAttribute('disabled')).toBe(false)
    fireEvent.click(restore)
    await waitFor(() => expect(state.databaseRestoreCalls).toBe(1))
    expect(screen.getByText(/Database restored to \/tmp\/cortana-backup\.sqlite3/)).toBeTruthy()

    serviceReport.services[1] = {
      ...serviceReport.services[1],
      installed: true,
      loaded: true,
      state: 'running',
    }
    fireEvent.click(screen.getByRole('button', { name: 'Readiness' }))
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Restore database' }).hasAttribute('disabled')
      ).toBe(true)
    )
    expect(state.databaseRestoreCalls).toBe(1)
  } finally {
    serviceReport.services.splice(0, serviceReport.services.length, ...originalServices)
    window.confirm = originalConfirm
  }
})

test('settings view reuses the shell settings snapshot without a duplicate read', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  await waitFor(() => expect(state.getDesktopSettingsCalls).toBe(1))

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())

  expect(state.getDesktopSettingsCalls).toBe(1)
})

test('updates settings reuses the shell updater snapshot without a duplicate read', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  await waitFor(() => expect(state.getDesktopUpdateCalls).toBe(1))

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Updates' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Updates' })).toBeTruthy())

  expect(state.getDesktopUpdateCalls).toBe(1)
})

test('settings save refreshes shell service metadata immediately', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  await waitFor(() => expect(state.getDesktopServicesCalls).toBe(1))

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
  fireEvent.change(screen.getAllByLabelText('Display name')[0], {
    target: { value: 'Work settings' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))

  await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
  await waitFor(() => expect(state.getDesktopServicesCalls).toBe(2))
})

test('successful service actions clear a stale shell service error immediately', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.serviceStatusError = null
  state.serviceRestartCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

    state.serviceStatusError = new Error('service status transport failed')
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(screen.getByText('service status transport failed')).toBeTruthy())

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Restart all' }))
      await Promise.resolve()
    })
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))

    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    const serviceHealth = await screen.findByRole('button', { name: 'Open service health' })
    expect(serviceHealth.textContent).toContain('Services:')
    expect(serviceHealth.textContent).not.toContain('unavailable')
  } finally {
    window.confirm = originalConfirm
    state.serviceStatusError = null
  }
})

test('saving settings clears stale local service errors', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.serviceStatusError = new Error('service status transport failed')
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(screen.getByText('service status transport failed')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeTruthy())
    state.serviceStatusError = null
    fireEvent.change(screen.getAllByLabelText('Display name')[0], {
      target: { value: 'Work settings' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))

    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
    await waitFor(() => expect(screen.queryByText('service status transport failed')).toBeNull())
  } finally {
    window.confirm = originalConfirm
    state.serviceStatusError = null
  }
})

test('service activity survives leaving Settings while a native action is running', async () => {
  const originalConfirm = window.confirm
  const originalAction = state.serviceAction
  let resolveAction: ((report: DesktopServiceReport) => void) | undefined
  window.confirm = () => true
  state.serviceAction = () =>
    new Promise<DesktopServiceReport>((resolve) => {
      resolveAction = resolve
    })
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restart all' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Restart all' }))
    await waitFor(() => expect(screen.getByText('Service: restart core services…')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByText('Service: restart core services…')).toBeTruthy())

    resolveAction?.(installedServiceReport)
    await waitFor(() =>
      expect(screen.getByText('Service: restart core services · done')).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Open service activity' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
    expect(screen.getByText('Restart core services completed.')).toBeTruthy()
  } finally {
    state.serviceAction = originalAction
    window.confirm = originalConfirm
  }
})

test('readiness activity survives leaving Settings while a scan is running', async () => {
  const originalScan = state.readinessScan
  let resolveScan:
    ((value: Awaited<ReturnType<typeof realApi.scanDesktopReadiness>>) => void) | undefined
  state.readinessScan = () =>
    new Promise((resolve) => {
      resolveScan = resolve
    })
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() => expect(screen.getByText('Readiness: scanning…')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByText('Readiness: scanning…')).toBeTruthy())

    await act(async () => {
      resolveScan?.({
        scanned_at_unix_seconds: 1785000000,
        platform: 'macos',
        tools_ready: true,
        core: null,
        core_error: null,
        tools: [],
      })
      await Promise.resolve()
    })
    await waitFor(() => expect(screen.getByText('Readiness: ready')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Open readiness activity' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'System readiness' })).toBeTruthy()
    )
    expect(screen.getByText(/Last checked/)).toBeTruthy()
    await flushDesktopBootstrap()
  } finally {
    state.readinessScan = originalScan
  }
})

test('failed first-launch readiness scan waits for an explicit retry', async () => {
  const originalSettings = state.settings
  const originalScan = state.readinessScan
  let calls = 0
  state.settings = { ...desktopSettings, needs_setup: true }
  state.readinessScan = () => {
    calls += 1
    return Promise.reject(new Error('readiness unavailable'))
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByText('readiness unavailable')).toBeTruthy())
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(calls).toBe(1)
  } finally {
    state.settings = originalSettings
    state.readinessScan = originalScan
  }
})

test('successful first-launch readiness scan releases the scan control', async () => {
  const originalSettings = state.settings
  const originalScan = state.readinessScan
  state.settings = { ...desktopSettings, needs_setup: true }
  state.readinessScan = () =>
    Promise.resolve({
      scanned_at_unix_seconds: 1785000000,
      platform: 'macos',
      tools_ready: true,
      core: null,
      core_error: null,
      tools: [],
    })
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByText('Local tools ready')).toBeTruthy())
    const scan = screen.getByRole('button', { name: 'Run again' })
    expect(scan.hasAttribute('disabled')).toBe(false)
  } finally {
    state.settings = originalSettings
    state.readinessScan = originalScan
  }
})

test('embedding generation mismatch offers a confirmed desktop adoption action', async () => {
  const originalScan = state.readinessScan
  const originalConfirm = window.confirm
  let scanCalls = 0
  state.readinessScan = () => {
    scanCalls += 1
    const migrated = scanCalls > 1
    return Promise.resolve({
      scanned_at_unix_seconds: 1785000000 + scanCalls,
      platform: 'macos',
      tools_ready: true,
      core: {
        passed: migrated,
        query_mode: 'extractive',
        embedding_generation: {
          stored: migrated ? 'openai:http://127.0.0.1:6999/v1:model:256' : 'legacy:model:256',
          configured: 'openai:http://127.0.0.1:6999/v1:model:256',
        },
        checks: [
          {
            name: 'embedding-index',
            passed: migrated,
            detail: migrated ? 'index generation matches configured' : 'generation mismatch',
          },
        ],
      },
      core_error: null,
      tools: [],
    })
  }
  window.confirm = () => true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Adopt stored generation' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Adopt stored generation' }))
    await waitFor(() =>
      expect(
        screen.getByText('Embedding generation adopted and readiness was rescanned.')
      ).toBeTruthy()
    )
    expect(state.embeddingMigrationCalls).toEqual(['legacy:model:256'])
  } finally {
    state.readinessScan = originalScan
    window.confirm = originalConfirm
  }
})

test('embedding adoption reports a follow-up mismatch instead of claiming readiness', async () => {
  const originalScan = state.readinessScan
  const originalConfirm = window.confirm
  state.readinessScan = () =>
    Promise.resolve({
      scanned_at_unix_seconds: 1785000000,
      platform: 'macos',
      tools_ready: true,
      core: {
        passed: false,
        query_mode: 'extractive',
        embedding_generation: {
          stored: 'legacy:model:256',
          configured: 'openai:http://127.0.0.1:6999/v1:model:256',
        },
        checks: [
          {
            name: 'embedding-index',
            passed: false,
            detail: 'generation mismatch',
          },
        ],
      },
      core_error: null,
      tools: [],
    })
  window.confirm = () => true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Adopt stored generation' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Adopt stored generation' }))
    await waitFor(() =>
      expect(
        screen.getByText(
          'Embedding generation was adopted, but the follow-up readiness scan still reports a mismatch.'
        )
      ).toBeTruthy()
    )
  } finally {
    state.readinessScan = originalScan
    window.confirm = originalConfirm
  }
})

test('completed installers trigger one shell-owned post-install readiness scan', async () => {
  const originalConfirm = window.confirm
  state.installerJob = null
  window.confirm = () => true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() => expect(screen.getByText('uv is not installed')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    await waitFor(() => expect(screen.getByText('Installing uv')).toBeTruthy())

    state.installerJob = {
      ...state.installerJob!,
      status: 'succeeded',
      summary: 'uv installed',
      completed_at_unix_seconds: 1785000010,
      exit_code: 0,
    }
    await waitFor(() => expect(screen.getByText('Readiness: ready')).toBeTruthy(), {
      timeout: 2_500,
    })
  } finally {
    state.installerJob = null
    window.confirm = originalConfirm
  }
})

test('local embedding readiness explains the approval-gated runtime installer', async () => {
  const originalConfirm = window.confirm
  const originalScan = state.readinessScan
  let confirmation = ''
  state.readinessScan = () =>
    Promise.resolve({
      scanned_at_unix_seconds: 1785000000,
      platform: 'macos',
      tools_ready: false,
      core: null,
      core_error: null,
      tools: [
        {
          id: 'embedding-runtime',
          label: 'Local embedding runtime',
          required: true,
          available: false,
          path: null,
          version: null,
          install_supported: true,
          detail: 'Install the text-embeddings-inference runtime with Homebrew.',
        },
      ],
    })
  window.confirm = (message) => {
    confirmation = message ?? ''
    return false
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() => expect(screen.getByText(/text-embeddings-inference runtime/)).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    expect(confirmation).toContain('text-embeddings-inference runtime with Homebrew')
    expect(state.installerJob).toBeNull()
  } finally {
    state.readinessScan = originalScan
    window.confirm = originalConfirm
  }
})

test('installer progress survives settings section changes', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.installerJob = null
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Run readiness scan' }))
    await waitFor(() => expect(screen.getByText('uv is not installed')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    await waitFor(() => expect(screen.getByText('Installing uv')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Readiness' }))
    await waitFor(() => expect(screen.getByText('Installing uv')).toBeTruthy())
    expect(screen.getByText('Status: running')).toBeTruthy()

    // The shell owns the installer snapshot, so leaving Settings does not
    // discard progress or stop native status polling.
    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    expect(screen.getByText('Install: uv · running')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: 'Open installer status for uv' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    await waitFor(() => expect(screen.getByText('Installing uv')).toBeTruthy())
  } finally {
    window.confirm = originalConfirm
    state.installerJob = null
  }
})

test('saving settings with restart_required triggers a background restart and clears the notice on success', async () => {
  const originalSettings = state.settings
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.settings = {
    ...desktopSettings,
    restart_required: true,
    embedding: {
      ...desktopSettings.embedding,
      provider: 'local',
      model: 'qwen/Qwen3-Embedding-0.6B',
      base_url: 'http://127.0.0.1:6999/v1',
    },
  }
  state.serviceRestartCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], { target: { value: 'Alpha' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))

    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))
    await waitFor(() =>
      expect(screen.queryByText('A service restart is still required.')).toBeNull()
    )
    await flushDesktopBootstrap()
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
    state.serviceRestartCalls = 0
  }
})

test('successful aggregate restart clears the saved-settings notice', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  window.confirm = () => true
  state.settings = { ...originalSettings, restart_required: true }
  state.serviceRestartCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    expect(screen.getByText('A service restart is still required.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Open services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Restart all' }))
      await Promise.resolve()
    })
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))
    expect(screen.queryByText('A service restart is still required.')).toBeNull()
  } finally {
    state.settings = originalSettings
    window.confirm = originalConfirm
  }
})

test('a failed background restart after saving names the failure and offers recovery', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  const originalServiceAction = state.serviceAction
  window.confirm = () => true
  state.serviceRestartCalls = 0
  state.settings = {
    ...desktopSettings,
    restart_required: true,
    embedding: {
      ...desktopSettings.embedding,
      provider: 'local',
      model: 'qwen/Qwen3-Embedding-0.6B',
      base_url: 'http://127.0.0.1:6999/v1',
    },
  }
  state.serviceAction = () => Promise.reject(new Error('embedding service failed to restart'))
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Workspaces' }))
    fireEvent.change(screen.getAllByLabelText('Display name')[0], { target: { value: 'Alpha' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))

    // The failure is named instead of claiming the services are restarting.
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))
    await waitFor(() =>
      expect(
        screen.getByText(/Settings saved, but the service restart failed: embedding service failed/)
      ).toBeTruthy()
    )
    expect(
      screen.queryByText('Settings saved. Affected services are restarting in the background.')
    ).toBeNull()
    expect(screen.getByRole('alert')).toBeTruthy()
    const openServices = screen.getByRole('button', { name: 'Open services' })
    expect(openServices).toBeTruthy()
    fireEvent.click(openServices)
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())

    // Retry restart recovers once the native action succeeds again.
    state.serviceAction = () => Promise.resolve(installedServiceReport)
    fireEvent.click(screen.getByRole('button', { name: 'Retry restart' }))
    await waitFor(() => expect(state.serviceRestartCalls).toBe(2))
    await waitFor(() => expect(screen.queryByText(/service restart failed/)).toBeNull())
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
    state.serviceAction = originalServiceAction
    state.serviceRestartCalls = 0
  }
})

test('a failed source toggle restart is reported with a manual recovery path', async () => {
  const originalConfirm = window.confirm
  const originalSettings = state.settings
  const originalServiceAction = state.serviceAction
  const originalApplySettingsUpdate = state.applySettingsUpdate
  window.confirm = () => true
  state.applySettingsUpdate = true
  state.serviceRestartCalls = 0
  state.settings = {
    ...desktopSettings,
    restart_required: true,
    sources: [{ ...workSource, enabled: false }],
  }
  state.serviceAction = () => Promise.reject(new Error('sync service failed to restart'))
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByLabelText('Open sources'))
    const toggle = await screen.findByRole('switch', { name: 'Enable work-code' })
    fireEvent.click(toggle)

    // The toggle save reports restart_required, so the shell restarts the
    // affected services in the background and names the failure on error.
    await waitFor(() => expect(state.saveSettingsCalls).toBe(1))
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))
    await waitFor(() =>
      expect(
        screen.getByText(
          /Source setting saved, but the service restart failed \(sync service failed to restart\)/
        )
      ).toBeTruthy()
    )
    // The status bar keeps the failed activity visible and links to Services.
    expect(screen.getByRole('button', { name: 'Open service activity' })).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
    state.settings = originalSettings
    state.serviceAction = originalServiceAction
    state.applySettingsUpdate = originalApplySettingsUpdate
    state.serviceRestartCalls = 0
  }
})

test('services settings keeps repair available for a partial core install', async () => {
  const original = serviceReport.services.map((service) => ({ ...service }))
  serviceReport.services[0] = {
    ...serviceReport.services[0],
    installed: true,
    loaded: true,
    state: 'running',
  }
  serviceReport.services[1] = {
    ...serviceReport.services[1],
    installed: true,
    loaded: true,
    state: 'running',
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Install core services/ })).toBeTruthy()
    )
  } finally {
    serviceReport.services.splice(0, serviceReport.services.length, ...original)
  }
})

test('services settings surfaces a non-zero last exit as a failed service', async () => {
  const original = serviceReport.services[1]
  serviceReport.services[1] = {
    ...original,
    installed: true,
    loaded: true,
    state: 'exited',
    last_exit_status: 1,
  }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByText(/last exit 1/)).toBeTruthy())
    expect(screen.getByText(/exited/)).toBeTruthy()
  } finally {
    serviceReport.services[1] = original
  }
})

test('services settings disables aggregate actions when the platform backend is unavailable', async () => {
  const originalSupported = serviceReport.supported
  serviceReport.supported = false
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Services' }))
    await waitFor(() => expect(screen.getByText(/not supported on macos/)).toBeTruthy())
    for (const label of ['Start all', 'Stop all', 'Restart all']) {
      expect(screen.getByRole('button', { name: label }).hasAttribute('disabled')).toBe(true)
    }
  } finally {
    serviceReport.supported = originalSupported
  }
})

test('Google source settings expose env-backed token credentials', async () => {
  state.settings = { ...desktopSettings, sources: [googleSource] }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    await waitFor(() =>
      expect(screen.getByText('Google token path environment variable')).toBeTruthy()
    )
    expect(screen.getByLabelText('Source label')).toBeTruthy()
    expect(screen.getByText('Google token path value')).toBeTruthy()
  } finally {
    state.settings = desktopSettings
  }
})

test('Google source authorization action starts a tracked browser job', async () => {
  const originalSettings = state.settings
  const originalConfirm = window.confirm
  state.settings = { ...desktopSettings, sources: [googleSource] }
  state.authorizationCalls = []
  window.confirm = () => true
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Authorize' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Authorize' }))
    await waitFor(() => expect(state.authorizationCalls).toEqual(['personal-drive']))
    expect(screen.getByText('personal-drive · authorization · running')).toBeTruthy()
    expect(screen.getByText(/Waiting for Google authorization/)).toBeTruthy()
  } finally {
    state.settings = originalSettings
    state.sourceJob = null
    state.authorizationCalls = []
    window.confirm = originalConfirm
  }
})

test('Google authorization accepts a token path supplied through the configured environment variable', async () => {
  const originalSettings = state.settings
  state.settings = { ...desktopSettings, sources: [googleEnvOnlySource] }
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    const authorize = await screen.findByRole('button', { name: 'Authorize' })
    expect(authorize.hasAttribute('disabled')).toBe(false)
  } finally {
    state.settings = originalSettings
  }
})

test('running source jobs stay visible in the shell after leaving the settings view', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.settings = { ...desktopSettings, sources: [workSource] }
    render(<App />)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy()
    )

    // No jobs have started yet, so no shell indicator is shown.
    expect(screen.queryByText(/active source job/)).toBeNull()

    // Start a bounded validation from the settings sources section.
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    await waitFor(() => expect(screen.getByText(/1 enabled · 1 configured/)).toBeTruthy())
    expect(screen.getByText('Content limit (characters)')).toBeTruthy()
    expect(screen.getByText('Duration limit (seconds)')).toBeTruthy()
    expect(screen.getByText('Document labels')).toBeTruthy()
    expect(screen.getByText('Document ACL labels')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Validate' }))
    await waitFor(() => expect(screen.getByText('work-code · validation · running')).toBeTruthy())

    // Leaving the settings view must not hide the running job: the status
    // bar indicator and the read-only source-panel strip keep it visible.
    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    expect(screen.getByText('1 active source job')).toBeTruthy()
    const activeJobs = screen.getByRole('button', { name: 'Open active source jobs' })
    expect(activeJobs).toBeTruthy()
    expect(activeJobs.getAttribute('title')).toBeNull()
    expect(activeJobs.getAttribute('data-tooltip')).toContain('work-code · validation')

    fireEvent.click(activeJobs)
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Inbox' })).toBeTruthy())
    expect(screen.getByRole('heading', { name: 'Active source jobs' })).toBeTruthy()
    const cancel = screen.getByRole('button', { name: 'Cancel work work-code validation' })
    fireEvent.click(cancel)
    await waitFor(() => expect((cancel as HTMLButtonElement).disabled).toBe(true))
  } finally {
    window.confirm = originalConfirm
    state.settings = desktopSettings
    state.sourceJob = null
    state.installerJob = null
  }
})

test('completed source jobs refresh source health without waiting for the status interval', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  state.settings = { ...desktopSettings, sources: [workSource] }
  state.sourceJob = null
  state.statusCalls = 0
  try {
    render(<App />)
    await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
    const initialStatusCalls = state.statusCalls

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Validate' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Validate' }))
    await waitFor(() => expect(state.sourceJob?.status).toBe('running'))

    state.sourceJob = {
      ...state.sourceJob!,
      status: 'succeeded',
      summary: 'Validation succeeded.',
      completed_at_unix_seconds: state.sourceJob!.started_at_unix_seconds + 1,
      exit_code: 0,
    }
    await waitFor(() => expect(state.statusCalls).toBeGreaterThan(initialStatusCalls), {
      timeout: 3_000,
    })
  } finally {
    window.confirm = originalConfirm
    state.settings = desktopSettings
    state.sourceJob = null
  }
})

test('hindsight status section remains explicit about being optional', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Hindsight' }))
  expect(screen.getByText('Hindsight memory sidecar')).toBeTruthy()
  expect(screen.getByText(/intentionally not wired into normal ingestion/i)).toBeTruthy()
  expect(screen.getByDisplayValue('Optional sidecar')).toBeTruthy()
  expect(screen.getByDisplayValue('Disabled')).toBeTruthy()
  expect(screen.getByLabelText('Enabled')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Check connection' }))
  await waitFor(() => expect(screen.getByText(/Health: disabled/)).toBeTruthy())
  expect(screen.getByRole('button', { name: 'Open Hindsight status' })).toBeTruthy()

  // The shell retains the native health result when Settings is unmounted.
  fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Hindsight' }))
  await waitFor(() => expect(screen.getByText(/Health: disabled/)).toBeTruthy())
  expect(screen.getByText(/health snapshot is retained/i)).toBeTruthy()
})

test('honcho settings section exposes a disabled-by-default session sidecar', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Honcho' }))
  expect(screen.getByText('Honcho memory sidecar')).toBeTruthy()
  expect(screen.getByText(/not the source of truth/i)).toBeTruthy()
  expect(screen.getByDisplayValue('https://api.honcho.dev')).toBeTruthy()
  expect(screen.getByDisplayValue('default')).toBeTruthy()
  expect(screen.getAllByDisplayValue('cortana')).toHaveLength(2)
  expect(screen.getByLabelText('Enabled')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Check connection' }))
  await waitFor(() => expect(screen.getByText(/Health: disabled/)).toBeTruthy())
  expect(screen.getByRole('button', { name: 'Open Honcho status' })).toBeTruthy()
})

test('local runtime section opens active secret file path in desktop', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: updatesButtonName })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))
  await waitFor(() => expect(screen.getByText('Local runtime')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Open secret file' }))
  await waitFor(() => expect(state.openSecretFileCalls).toBe(1))
  expect(
    screen.getByText('Opened the active secret file in your default application.')
  ).toBeTruthy()
})
