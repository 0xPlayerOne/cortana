import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopInfo, desktopSettings } from './test/fixtures'
import type {
  DesktopInitialSyncPlan,
  DesktopServiceReport,
  DesktopSettings,
  DesktopSourceJob,
  InitialSyncBudget,
  SourceSettings,
} from './types'
import { INITIAL_SYNC_BUDGETS } from './types'

// SettingsView owns several asynchronous native-status effects. Let the
// final microtask settle before unmounting so a promise from one test cannot
// update a component while the next test is rendering.
afterEach(async () => {
  await new Promise((resolve) => setTimeout(resolve, 0))
  cleanup()
})

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

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

function settingsWith(source: SourceSettings): DesktopSettings {
  return { ...desktopSettings, sources: [source] }
}

function planFor(
  budget: InitialSyncBudget,
  overrides?: Partial<DesktopInitialSyncPlan>
): DesktopInitialSyncPlan {
  const tier = INITIAL_SYNC_BUDGETS.find((item) => item.budget === budget)!
  return {
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    enabled: true,
    budget,
    budget_documents: tier.documents,
    budget_bytes: tier.bytes,
    budget_seconds: tier.seconds,
    writes_indexed_data: true,
    requires_validation: true,
    validation_covers_budget: true,
    plan_id: `plan-${budget}-1`,
    ...overrides,
  }
}

function jobFor(budget: InitialSyncBudget, status: DesktopSourceJob['status']): DesktopSourceJob {
  return {
    id: 'source-1-1',
    operation: 'initial-sync',
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    status,
    summary:
      status === 'running'
        ? 'Guarded initial sync may index up to 100 documents or 25 MiB for at most 15 minutes. Reconciliation is disabled.'
        : status === 'cancelled'
          ? 'Guarded initial sync was cancelled. Committed batches remain indexed; reconciliation did not run.'
          : 'Guarded initial sync completed within its selected budget without deletion reconciliation.',
    log: '',
    started_at_unix_seconds: 1785000000,
    completed_at_unix_seconds: status === 'running' ? null : 1785000100,
    exit_code: status === 'running' ? null : 0,
    retryable: status === 'cancelled' || status === 'failed',
    writes_indexed_data: true,
    budget,
  }
}

// Capture the real api module, then register a mock so each test controls the
// native command boundary exactly like the Tauri bridge would.
const realApi = await import('./api')

const state = {
  settings: settingsWith(workSource),
  planCalls: [] as Array<{ source: string; budget: InitialSyncBudget }>,
  executeCalls: [] as Array<{ source: string; budget: InitialSyncBudget; planId: string }>,
  validationCalls: [] as Array<{ source: string; budget?: InitialSyncBudget }>,
  planOverrides: {} as Partial<DesktopInitialSyncPlan>,
  planError: null as Error | null,
  settingsLoadError: null as Error | null,
  runningJob: null as DesktopSourceJob | null,
  cancelCalls: [] as string[],
  cancelDeferred: null as Deferred<DesktopSourceJob> | null,
  pollCount: 0,
  servicesCalls: 0,
  serviceRefreshAfterAction: null as Deferred<DesktopServiceReport> | null,
  serviceActionResponse: null as Deferred<DesktopServiceReport> | null,
  serviceRefreshReports: [] as DesktopServiceReport[],
}

beforeEach(() => {
  state.settings = settingsWith(workSource)
  state.planCalls = []
  state.executeCalls = []
  state.validationCalls = []
  state.planOverrides = {}
  state.planError = null
  state.settingsLoadError = null
  state.runningJob = null
  state.cancelCalls = []
  state.cancelDeferred = null
  state.pollCount = 0
  state.servicesCalls = 0
  state.serviceRefreshAfterAction = null
  state.serviceActionResponse = null
  state.serviceRefreshReports = []
})

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getDesktopSettings: () =>
    state.settingsLoadError
      ? Promise.reject(state.settingsLoadError)
      : Promise.resolve(state.settings),
  getDesktopInfo: () => Promise.resolve(desktopInfo),
  getDesktopSchedule: () =>
    Promise.resolve({ sync_interval_seconds: 900, backup_interval_seconds: 86400 }),
  saveDesktopSchedule: (schedule: {
    sync_interval_seconds: number
    backup_interval_seconds: number
  }) => Promise.resolve(schedule),
  getDesktopUpdate: () => Promise.reject(new Error('Updates unavailable')),
  getDesktopServices: () => {
    state.servicesCalls += 1
    if (state.servicesCalls === 1) {
      const stale = state.serviceRefreshReports[0]
      return Promise.resolve(stale)
    }
    return state.serviceRefreshAfterAction
      ? state.serviceRefreshAfterAction.promise
      : Promise.resolve(state.serviceRefreshReports.at(-1) ?? state.serviceRefreshReports[0])
  },
  runDesktopServicesActionAll: () => {
    if (!state.serviceActionResponse) return Promise.reject(new Error('Service action unavailable'))
    return state.serviceActionResponse.promise
  },
  getRuntimeAudit: () => Promise.resolve([]),
  getDesktopAudit: () => Promise.resolve([]),
  planDesktopInitialSync: (source: string, budget: InitialSyncBudget) => {
    state.planCalls.push({ source, budget })
    if (state.planError) return Promise.reject(state.planError)
    return Promise.resolve(planFor(budget, { source, ...state.planOverrides }))
  },
  startDesktopInitialSync: (source: string, budget: InitialSyncBudget, planId: string) => {
    state.executeCalls.push({ source, budget, planId })
    if (!state.runningJob) return Promise.reject(new Error('initial sync failed to start'))
    return Promise.resolve({ ...state.runningJob, source, budget })
  },
  startDesktopSourceValidation: (source: string, budget?: InitialSyncBudget) => {
    state.validationCalls.push({ source, budget })
    return Promise.resolve({
      ...jobFor(budget || 'small', 'running'),
      id: 'source-1-2',
      operation: 'validation',
      writes_indexed_data: false,
    })
  },
  getDesktopSourceValidation: () => {
    state.pollCount += 1
    if (!state.runningJob) return Promise.reject(new Error('job missing'))
    if (state.runningJob.status === 'running' && state.pollCount > 1) {
      return Promise.resolve({
        ...state.runningJob,
        status: 'succeeded',
        summary:
          'Guarded initial sync completed within its selected budget without deletion reconciliation.',
      })
    }
    return Promise.resolve(state.runningJob)
  },
  cancelDesktopSourceValidation: (id: string) => {
    state.cancelCalls.push(id)
    return state.cancelDeferred?.promise ?? Promise.resolve(jobFor('small', 'cancelled'))
  },
}))

const { SettingsView } = await import('./components/SettingsView')

function oldServicesReport(): DesktopServiceReport {
  return {
    platform: 'macos',
    supported: true,
    services: [
      {
        name: 'server',
        label: 'ai.cortana.server',
        installed: true,
        loaded: false,
        state: null,
        pid: null,
        last_exit_status: null,
      },
      {
        name: 'embedding',
        label: 'ai.cortana.embedding',
        installed: true,
        loaded: false,
        state: null,
        pid: null,
        last_exit_status: null,
      },
    ],
  }
}

function runningServicesReport(): DesktopServiceReport {
  return {
    platform: 'macos',
    supported: true,
    services: [
      {
        name: 'server',
        label: 'ai.cortana.server',
        installed: true,
        loaded: true,
        state: 'running',
        pid: 12345,
        last_exit_status: null,
      },
      {
        name: 'embedding',
        label: 'ai.cortana.embedding',
        installed: true,
        loaded: true,
        state: 'running',
        pid: 12346,
        last_exit_status: null,
      },
    ],
  }
}

function openSources() {
  render(<SettingsView onSaved={() => {}} initialSection="sources" />)
}

async function openAdvancedSource(index = 0) {
  const triggers = await screen.findAllByRole('button', { name: /Advanced source settings/ })
  fireEvent.click(triggers[index])
}

test('service action result is not overwritten by stale local refresh', async () => {
  const originalSetInterval = window.setInterval
  const originalClearInterval = window.clearInterval
  const originalConfirm = window.confirm
  let poll: (() => void) | undefined
  state.servicesCalls = 0
  state.serviceRefreshReports = [oldServicesReport()]
  state.serviceRefreshAfterAction = deferred<DesktopServiceReport>()
  state.serviceActionResponse = deferred<DesktopServiceReport>()
  const staleRefresh = state.serviceRefreshAfterAction!
  const stale = oldServicesReport()
  const fresh = runningServicesReport()

  window.setInterval = ((callback: () => void) => {
    poll = callback
    return 1 as unknown as number
  }) as typeof window.setInterval
  window.clearInterval = (() => undefined) as typeof window.clearInterval
  window.confirm = () => true

  try {
    render(
      <SettingsView onSaved={() => {}} initialSection="services" desktopSettings={state.settings} />
    )

    await waitFor(() => expect(screen.getByText('0 loaded')).toBeTruthy())
    if (!poll) throw new Error('Expected local services poll callback to be registered')
    act(() => {
      poll?.()
    })
    fireEvent.click(screen.getByRole('button', { name: 'Restart all' }))
    await act(async () => {
      state.serviceActionResponse?.resolve(fresh)
    })

    // The service action and the invalidated refresh are intentionally
    // overlapped; allow a slower CI renderer to settle without weakening the
    // stale-result assertion.
    await waitFor(() => expect(screen.getByText(/PID 12345/)).toBeTruthy(), { timeout: 3000 })

    await act(async () => {
      staleRefresh.resolve(stale)
    })

    await waitFor(() => expect(screen.getByText(/PID 12345/)).toBeTruthy())
  } finally {
    window.setInterval = originalSetInterval
    window.clearInterval = originalClearInterval
    window.confirm = originalConfirm
  }
})

test('standalone updater failures stay visible instead of being swallowed', async () => {
  render(
    <SettingsView onSaved={() => {}} initialSection="updates" desktopSettings={state.settings} />
  )

  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain('Updates unavailable')
  )
})

test('settings bridge failures expose a retry action', async () => {
  state.settingsLoadError = new Error('settings bridge unavailable')
  const loaded = { value: null as DesktopSettings | null }
  render(
    <SettingsView
      onSaved={() => {}}
      onLoaded={(next) => {
        loaded.value = next
      }}
      initialSection="readiness"
    />
  )

  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain('settings bridge unavailable')
  )
  state.settingsLoadError = null
  fireEvent.click(screen.getByRole('button', { name: 'Retry settings' }))
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy())
  expect(loaded.value).toEqual(state.settings)
})

test('a shared active source job locks source actions until it finishes', async () => {
  const activeJob = {
    ...jobFor('small', 'running'),
    operation: 'trial-sync' as const,
    summary: 'Guarded trial sync is running.',
  }
  render(<SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[activeJob]} />)

  await openAdvancedSource()
  await waitFor(() => expect(screen.getByText('work-code · trial-sync · running')).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: /Cancel/ }))
  await waitFor(() => expect(state.cancelCalls).toEqual(['source-1-1']))
  for (const label of ['Test connection', 'Trial sync', 'Initial sync', 'Remove work-code']) {
    if (label === 'Remove work-code') {
      expect((screen.getByRole('button', { name: label }) as HTMLButtonElement).disabled).toBe(true)
    } else {
      expect(screen.queryByRole('button', { name: label })).toBeNull()
    }
  }
  expect((screen.getByRole('button', { name: 'Add source' }) as HTMLButtonElement).disabled).toBe(
    false
  )
  expect((screen.getByLabelText(/^Source name/) as HTMLInputElement).disabled).toBe(true)
  expect(screen.queryByLabelText('Workspace')).toBeNull()
})

test('standalone source polling pauses while Settings is backgrounded', async () => {
  const originalConfirm = window.confirm
  const visibilityDescriptor = Object.getOwnPropertyDescriptor(document, 'visibilityState')
  const setVisibility = (value: 'hidden' | 'visible') => {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value,
    })
    document.dispatchEvent(new Event('visibilitychange'))
  }
  window.confirm = () => true
  state.runningJob = jobFor('small', 'running')
  try {
    render(
      <SettingsView onSaved={() => {}} initialSection="sources" desktopSettings={state.settings} />
    )
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Test connection' })).toBeTruthy()
    )

    act(() => setVisibility('hidden'))
    fireEvent.click(screen.getByRole('button', { name: 'Test connection' }))
    await waitFor(() => expect(state.validationCalls).toHaveLength(1))
    await new Promise((resolve) => setTimeout(resolve, 800))
    expect(state.pollCount).toBe(0)

    act(() => setVisibility('visible'))
    await waitFor(() => expect(state.pollCount).toBeGreaterThan(0), { timeout: 1_500 })
  } finally {
    window.confirm = originalConfirm
    if (visibilityDescriptor)
      Object.defineProperty(document, 'visibilityState', visibilityDescriptor)
  }
})

test('an active source job locks only that source configuration', async () => {
  const otherSource = {
    ...workSource,
    name: 'personal-notes',
    project: 'personal',
    root: '/Users/you/Notes',
  }
  state.settings = {
    ...settingsWith(workSource),
    sources: [workSource, otherSource],
  }
  const activeJob = {
    ...jobFor('small', 'running'),
    operation: 'trial-sync' as const,
    summary: 'Guarded trial sync is running.',
  }
  render(<SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[activeJob]} />)

  await waitFor(() => expect(screen.getByText(/Settings for work-code are locked/)).toBeTruthy())
  await openAdvancedSource()
  const workNames = screen.getAllByLabelText(/^Source name/) as HTMLInputElement[]
  expect(workNames).toHaveLength(1)
  expect(workNames[0].disabled).toBe(true)

  fireEvent.click(screen.getByRole('tab', { name: /Personal/ }))
  await openAdvancedSource()
  const personalName = screen.getByLabelText(/^Source name/) as HTMLInputElement
  expect(personalName.disabled).toBe(false)
  expect(
    (screen.getByRole('button', { name: 'Remove personal-notes' }) as HTMLButtonElement).disabled
  ).toBe(false)
  expect((screen.getByRole('button', { name: 'Add source' }) as HTMLButtonElement).disabled).toBe(
    false
  )
})

test('initial sync plans a fixed budget and displays the native limits', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))

    await waitFor(() => expect(screen.getByText('Guided initial sync')).toBeTruthy())
    await waitFor(() =>
      expect(screen.getByText('100 documents · 25 MiB · 15 minutes')).toBeTruthy()
    )
    expect(state.planCalls).toEqual([{ source: 'work-code', budget: 'small' }])
    expect(screen.getByText('Required at equal or larger limits')).toBeTruthy()
    expect(screen.getByText('Disabled')).toBeTruthy()
    expect(screen.getByText('Yes — committed batches become searchable')).toBeTruthy()

    // Switching the tier requests a fresh native plan for the new budget only.
    fireEvent.click(screen.getByRole('radio', { name: /Medium/ }))
    await waitFor(() =>
      expect(screen.getByText('500 documents · 64 MiB · 30 minutes')).toBeTruthy()
    )
    expect(state.planCalls.at(-1)).toEqual({ source: 'work-code', budget: 'medium' })
  } finally {
    window.confirm = originalConfirm
  }
})

test('execution requires an explicit confirmation and the plan id', async () => {
  const originalConfirm = window.confirm
  try {
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Start initial sync' })).toBeTruthy()
    )

    // Declining the confirmation must not reach the native command.
    window.confirm = () => false
    fireEvent.click(screen.getByRole('button', { name: 'Start initial sync' }))
    await waitFor(() => expect(state.executeCalls).toHaveLength(0))

    // Accepting it executes only with the plan issued by the plan request.
    window.confirm = () => true
    fireEvent.click(screen.getByRole('button', { name: 'Start initial sync' }))
    await waitFor(() => expect(state.executeCalls).toHaveLength(1))
    expect(state.executeCalls[0]).toEqual({
      source: 'work-code',
      budget: 'small',
      planId: 'plan-small-1',
    })
  } finally {
    window.confirm = originalConfirm
  }
})

test('editing the selected source invalidates its initial-sync plan', async () => {
  openSources()
  await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
  await waitFor(() => expect(screen.getByText('Guided initial sync')).toBeTruthy())
  await waitFor(() => expect(screen.getByText('100 documents · 25 MiB · 15 minutes')).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Close' }))
  await waitFor(() => expect(screen.queryByText('Guided initial sync')).toBeNull())
  await openAdvancedSource()

  await act(async () => {
    fireEvent.change(screen.getByLabelText(/^Source name/), { target: { value: 'work-code-v2' } })
  })
  expect(screen.queryByText('Guided initial sync')).toBeNull()
  expect(state.planCalls).toHaveLength(1)
})

test('a failed plan surfaces the native error and offers no start action', async () => {
  state.planError = new Error('configured source `work-code` was not found')
  openSources()
  await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))

  await waitFor(() =>
    expect(screen.getByText('configured source `work-code` was not found')).toBeTruthy()
  )
  expect(screen.queryByRole('button', { name: 'Start initial sync' })).toBeNull()
})

test('initial sync flow disappears safely when its source is reloaded away', async () => {
  const view = render(
    <SettingsView onSaved={() => {}} initialSection="sources" desktopSettings={state.settings} />
  )
  await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
  await waitFor(() => expect(screen.getByText('Guided initial sync')).toBeTruthy())

  view.rerender(
    <SettingsView
      onSaved={() => {}}
      initialSection="sources"
      desktopSettings={{ ...state.settings, sources: [] }}
    />
  )
  await waitFor(() => expect(screen.queryByText('Guided initial sync')).toBeNull())
})

test('a plan without validation coverage gates the start behind budget validation', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.planOverrides = { validation_covers_budget: false }
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))

    await waitFor(() =>
      expect(screen.getByText(/latest validation used smaller limits/)).toBeTruthy()
    )
    expect(
      (screen.getByRole('button', { name: 'Start initial sync' }) as HTMLButtonElement).disabled
    ).toBe(true)

    fireEvent.click(screen.getByRole('button', { name: 'Validate for this budget' }))
    await waitFor(() => expect(state.validationCalls).toHaveLength(1))
    expect(state.validationCalls[0]).toEqual({ source: 'work-code', budget: 'small' })
  } finally {
    window.confirm = originalConfirm
  }
})

test('shared source-job snapshots unlock the initial-sync plan without local polling', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.planOverrides = { validation_covers_budget: false }
    const view = render(
      <SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[]} />
    )
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
    await waitFor(() =>
      expect(screen.getByText(/latest validation used smaller limits/)).toBeTruthy()
    )

    fireEvent.click(screen.getByRole('button', { name: 'Validate for this budget' }))
    await waitFor(() => expect(state.validationCalls).toHaveLength(1))
    const running = {
      ...jobFor('small', 'running'),
      id: 'source-1-2',
      operation: 'validation' as const,
      writes_indexed_data: false,
    }
    const succeeded = {
      ...running,
      status: 'succeeded' as const,
      completed_at_unix_seconds: 1785000100,
      exit_code: 0,
      summary: 'Source validation succeeded within the selected budget.',
    }

    // App owns the poller in production. Simulate its snapshots arriving at
    // the same SettingsView instance and ensure validation completion requests
    // a new native plan rather than relying on the old local timer.
    view.rerender(
      <SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[running]} />
    )
    await waitFor(() => expect(screen.getByText('work-code · validation · running')).toBeTruthy())
    view.rerender(
      <SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[succeeded]} />
    )
    await waitFor(() => expect(state.planCalls.length).toBeGreaterThan(1))
  } finally {
    window.confirm = originalConfirm
  }
})

test('evicting a shared source job clears its stale local snapshot', async () => {
  const activeJob = {
    ...jobFor('small', 'running'),
    operation: 'trial-sync' as const,
    summary: 'Guarded trial sync is running.',
  }
  const view = render(
    <SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[activeJob]} />
  )

  await waitFor(() => expect(screen.getByText('work-code · trial-sync · running')).toBeTruthy())
  view.rerender(<SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[]} />)
  await waitFor(() => expect(screen.queryByText('work-code · trial-sync · running')).toBeNull())
})

test('execution shows running progress, cancellation, and a succeeded result', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.runningJob = jobFor('small', 'running')
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Start initial sync' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Start initial sync' }))

    await waitFor(() => expect(screen.getByText('work-code · initial-sync · running')).toBeTruthy())
    expect(screen.getByRole('button', { name: /Cancel/ })).toBeTruthy()

    // The second native status poll completes the job.
    await waitFor(
      () => expect(screen.getByText('work-code · initial-sync · succeeded')).toBeTruthy(),
      { timeout: 5000 }
    )
    expect(state.pollCount).toBeGreaterThan(1)
    expect(
      screen.queryByText(
        'Guarded initial sync completed within its selected budget without deletion reconciliation.'
      )
    ).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
  }
})

test('cancelling a running initial sync keeps the native cancelled summary', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.runningJob = jobFor('small', 'running')
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Start initial sync' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Start initial sync' }))
    await waitFor(() => expect(screen.getByText('work-code · initial-sync · running')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: /Cancel/ }))
    await waitFor(() =>
      expect(screen.getByText('work-code · initial-sync · cancelled')).toBeTruthy()
    )
    expect(
      screen.getByText(
        'Guarded initial sync was cancelled. Committed batches remain indexed; reconciliation did not run.'
      )
    ).toBeTruthy()
  } finally {
    window.confirm = originalConfirm
  }
})

test('source-job cancellation disables duplicate clicks while native cancellation is pending', async () => {
  const originalConfirm = window.confirm
  window.confirm = () => true
  try {
    state.runningJob = jobFor('small', 'running')
    state.cancelDeferred = deferred<DesktopSourceJob>()
    openSources()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Initial sync' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Start initial sync' })).toBeTruthy()
    )
    fireEvent.click(screen.getByRole('button', { name: 'Start initial sync' }))
    await waitFor(() => expect(screen.getByText('work-code · initial-sync · running')).toBeTruthy())

    const cancel = screen.getByRole('button', { name: /Cancel/ })
    fireEvent.click(cancel)
    await waitFor(() => expect((cancel as HTMLButtonElement).disabled).toBe(true))
    expect(state.cancelCalls).toHaveLength(1)

    fireEvent.click(cancel)
    expect(state.cancelCalls).toHaveLength(1)

    state.cancelDeferred.resolve(jobFor('small', 'cancelled'))
    await waitFor(() =>
      expect(screen.getByText('work-code · initial-sync · cancelled')).toBeTruthy()
    )
  } finally {
    window.confirm = originalConfirm
  }
})
