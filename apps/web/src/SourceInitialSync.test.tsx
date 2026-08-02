import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopSettings } from './test/fixtures'
import type {
  DesktopInitialSyncPlan,
  DesktopSettings,
  DesktopSourceJob,
  InitialSyncBudget,
  SourceSettings,
} from './types'
import { INITIAL_SYNC_BUDGETS } from './types'

afterEach(cleanup)

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
  runningJob: null as DesktopSourceJob | null,
  pollCount: 0,
}

beforeEach(() => {
  state.settings = settingsWith(workSource)
  state.planCalls = []
  state.executeCalls = []
  state.validationCalls = []
  state.planOverrides = {}
  state.planError = null
  state.runningJob = null
  state.pollCount = 0
})

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getDesktopSettings: () => Promise.resolve(state.settings),
  getDesktopInfo: () =>
    Promise.resolve({
      desktop_version: '0.11.4',
      backend_origin: 'http://127.0.0.1:7331',
      autostart_enabled: false,
      platform: 'macos',
    }),
  getDesktopUpdate: () => Promise.reject(new Error('Updates unavailable')),
  getDesktopServices: () => Promise.reject(new Error('Services unavailable')),
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
  cancelDesktopSourceValidation: () => Promise.resolve(jobFor('small', 'cancelled')),
}))

const { SettingsView } = await import('./components/SettingsView')

function openSources() {
  render(<SettingsView onSaved={() => {}} initialSection="sources" />)
}

test('a shared active source job locks source actions until it finishes', async () => {
  const activeJob = {
    ...jobFor('small', 'running'),
    operation: 'trial-sync' as const,
    summary: 'Guarded trial sync is running.',
  }
  render(<SettingsView onSaved={() => {}} initialSection="sources" sourceJobs={[activeJob]} />)

  await waitFor(() => expect(screen.getByRole('button', { name: 'Initial sync' })).toBeTruthy())
  for (const label of ['Validate', 'Trial sync', 'Initial sync', 'Remove work-code']) {
    expect((screen.getByRole('button', { name: label }) as HTMLButtonElement).disabled).toBe(true)
  }
  expect((screen.getByRole('button', { name: 'Add source' }) as HTMLButtonElement).disabled).toBe(
    true
  )
  expect((screen.getByLabelText(/^Source name/) as HTMLInputElement).disabled).toBe(true)
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
