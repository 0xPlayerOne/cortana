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
  SourceSettings,
} from './types'

afterEach(cleanup)

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

const state = {
  settings: desktopSettings as DesktopSettings,
  sourceJob: null as DesktopSourceJob | null,
  statusCalls: 0,
  serviceInstallCalls: 0,
  serviceRestartCalls: 0,
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
  installDesktopServices: () => {
    state.serviceInstallCalls += 1
    return Promise.resolve(installedServiceReport)
  },
  runDesktopServicesActionAll: (action: 'start' | 'stop' | 'restart') => {
    if (action === 'restart') state.serviceRestartCalls += 1
    return Promise.resolve(installedServiceReport)
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
  getRuntimeAudit: (limit: number) => Promise.resolve(runtimeAuditEvents.slice(0, limit)),
  getDesktopAudit: (limit: number) => Promise.resolve(desktopAuditEvents.slice(0, limit)),
  getDesktopUpdate: () => Promise.resolve(desktopUpdate),
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
  getDesktopSourceValidation: (id: string) => {
    if (!state.sourceJob || state.sourceJob.id !== id) {
      return Promise.reject(new Error('source job was not found'))
    }
    return Promise.resolve(state.sourceJob)
  },
}))

const { App } = await import('./App')

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
    const activeJobs = screen.getByLabelText('Active source jobs')
    expect(activeJobs).toBeTruthy()
    expect(activeJobs.textContent).toContain('work-code · validation · running')
  } finally {
    window.confirm = originalConfirm
    state.settings = desktopSettings
    state.sourceJob = null
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
})
