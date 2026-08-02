import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { demoStatus } from './demo'
import {
  desktopAuditEvents,
  desktopInfo,
  desktopSettings,
  desktopUpdate,
  runtimeAuditEvents,
} from './test/fixtures'
import type {
  DesktopServiceReport,
  DesktopSettings,
  DesktopSourceJob,
  DesktopInstallJob,
  SourceSettings,
} from './types'

afterEach(cleanup)
afterEach(() => window.localStorage.removeItem('cortana.workspace-selection.v1'))

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

const state = {
  settings: desktopSettings as DesktopSettings,
  sourceJob: null as DesktopSourceJob | null,
  authorizationCalls: [] as string[],
  statusCalls: 0,
  serviceInstallCalls: 0,
  serviceRestartCalls: 0,
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

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getStatus: () => {
    state.statusCalls += 1
    return Promise.resolve(demoStatus)
  },
  getDocuments: () => Promise.resolve({ documents: [], next_cursor: null }),
  getAnswer: () => Promise.reject(new Error('Answer request failed (503)')),
  getDocument: () => Promise.reject(new Error('Document unavailable')),
  getContext: () => Promise.reject(new Error('Context retrieval failed (503)')),
  getDesktopSettings: () => Promise.resolve(state.settings),
  getDesktopInfo: () => Promise.resolve(desktopInfo),
  getDesktopServices: () => Promise.resolve(serviceReport),
  getDesktopSourceJobs: () => Promise.resolve([]),
  installDesktopServices: () => {
    state.serviceInstallCalls += 1
    return Promise.resolve(installedServiceReport)
  },
  runDesktopServicesActionAll: (action: 'start' | 'stop' | 'restart') => {
    if (action === 'restart') state.serviceRestartCalls += 1
    return state.serviceAction ? state.serviceAction() : Promise.resolve(installedServiceReport)
  },
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
  getDesktopUpdate: () => Promise.resolve(desktopUpdate),
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

test('desktop shell restores a workspace scope and clears stale selections', async () => {
  window.localStorage.setItem('cortana.workspace-selection.v1', 'work')
  render(<App />)
  await waitFor(() => {
    expect((screen.getByRole('combobox') as HTMLSelectElement).value).toBe('work')
  })
  expect(screen.queryByRole('button', { name: /^personal-drive/ })).toBeNull()

  cleanup()
  window.localStorage.setItem('cortana.workspace-selection.v1', 'missing')
  render(<App />)
  await waitFor(() => {
    expect((screen.getByRole('combobox') as HTMLSelectElement).value).toBe('')
    expect(window.localStorage.getItem('cortana.workspace-selection.v1')).toBeNull()
  })
})

test('desktop settings navigation opens the audit trail and renders both event sources', async () => {
  render(<App />)

  // Desktop chrome: version and updates shortcut live in the footer.
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ })).toBeTruthy()
  )

  // Rail navigation into the settings view.
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  )
  expect(screen.getByText('Control plane')).toBeTruthy()
  const save = screen.getByRole('button', { name: 'Save changes' })
  expect(save).toBeTruthy()
  expect(save.hasAttribute('disabled')).toBe(true)

  // Section navigation into the audit trail.
  fireEvent.click(screen.getByRole('button', { name: 'Audit' }))
  await waitFor(() => expect(screen.getByText('2 runtime · 1 Desktop events')).toBeTruthy())
  expect(screen.getByText('Runtime retrieval')).toBeTruthy()
  expect(screen.getByText('Desktop actions')).toBeTruthy()
  expect(screen.getByText('brain_answer')).toBeTruthy()
  expect(screen.getByText('brain_documents')).toBeTruthy()
  expect(screen.getByText('settings_saved')).toBeTruthy()

  // Refreshing keeps the audit list stable.
  fireEvent.click(screen.getByRole('button', { name: /Refresh/ }))
  await waitFor(() => expect(screen.getByText('2 runtime · 1 Desktop events')).toBeTruthy())
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

test('desktop shell does not present a stale service report after refresh failure', () => {
  render(
    <ServiceHealthIndicator
      report={installedServiceReport}
      error="service status transport failed"
      onOpen={() => {}}
    />
  )

  expect(screen.getByText('Services: unavailable')).toBeTruthy()
  expect(
    screen.getByRole('button', { name: 'Open service health' }).getAttribute('title')
  ).toContain('service status transport failed')
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
    ).toContain('workspace-1')

    fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove source-1' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add source' }))
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

    fireEvent.click(screen.getByRole('button', { name: 'Discard' }))
    await waitFor(() =>
      expect((screen.getAllByLabelText('Display name')[0] as HTMLInputElement).value).toBe('Work')
    )
    expect(screen.queryByRole('button', { name: 'Discard' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(true)
  } finally {
    window.confirm = originalConfirm
  }
})

test('the footer updates shortcut opens the updates section directly', async () => {
  render(<App />)
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ })).toBeTruthy()
  )

  fireEvent.click(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ }))
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
    fireEvent.click(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ }))
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
    fireEvent.click(screen.getByRole('button', { name: /Install core services/ }))
    await waitFor(() => expect(state.serviceInstallCalls).toBe(1))
    expect(screen.getByText('3 loaded')).toBeTruthy()
    expect(screen.getByText(/sync service remains absent/)).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
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

    resolveScan?.({
      scanned_at_unix_seconds: 1785000000,
      platform: 'macos',
      tools_ready: true,
      core: null,
      core_error: null,
      tools: [],
    })
    await waitFor(() => expect(screen.getByText('Readiness: ready')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Open readiness activity' }))
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'System readiness' })).toBeTruthy()
    )
    expect(screen.getByText(/Last checked/)).toBeTruthy()
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
    expect(screen.getByText('A service restart is required.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Open services' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Services' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Restart all' }))
    await waitFor(() => expect(state.serviceRestartCalls).toBe(1))
    expect(screen.queryByText('A service restart is required.')).toBeNull()
  } finally {
    state.settings = originalSettings
    window.confirm = originalConfirm
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
      expect(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ })).toBeTruthy()
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
    expect(activeJobs.getAttribute('title')).toContain('work-code · validation')

    fireEvent.click(activeJobs)
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Inbox' })).toBeTruthy())
    expect(screen.getByRole('heading', { name: 'Active source jobs' })).toBeTruthy()
    const cancel = screen.getByRole('button', { name: 'Cancel work-code validation' })
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
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ })).toBeTruthy()
  )

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
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Cortana 0\.11\.2 · Updates/ })).toBeTruthy()
  )

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
