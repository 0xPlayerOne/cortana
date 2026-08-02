import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { act, cleanup, renderHook } from '@testing-library/react'

import type { DesktopSourceJob } from './types'

afterEach(cleanup)

function jobOf(
  id: string,
  status: DesktopSourceJob['status'],
  overrides: Partial<DesktopSourceJob> = {}
): DesktopSourceJob {
  return {
    id,
    operation: 'validation',
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    status,
    summary: `${id} summary`,
    log: '',
    started_at_unix_seconds: 1785000000,
    completed_at_unix_seconds: status === 'running' ? null : 1785000100,
    exit_code: status === 'running' ? null : 0,
    retryable: status === 'failed' || status === 'cancelled',
    writes_indexed_data: false,
    budget: null,
    ...overrides,
  }
}

// Capture the real api module, then register a mock so the hook polls the
// exact native status boundary like the Tauri bridge would.
const realApi = await import('./api')

const state = {
  statusCalls: [] as string[],
  polled: new Map<string, DesktopSourceJob | Error>(),
  pending: null as Promise<DesktopSourceJob> | null,
  recovered: [] as DesktopSourceJob[],
}

beforeEach(() => {
  state.statusCalls = []
  state.polled.clear()
  state.pending = null
  state.recovered = []
})

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getDesktopSourceJobs: () => Promise.resolve(state.recovered),
  getDesktopSourceValidation: (id: string) => {
    state.statusCalls.push(id)
    if (state.pending) return state.pending
    const result = state.polled.get(id)
    if (result instanceof Error) return Promise.reject(result)
    if (!result) return Promise.reject(new Error('source job was not found'))
    return Promise.resolve(result)
  },
}))

const {
  activeJobIds,
  activeJobs,
  describeSourceJobProgress,
  describeSourceJob,
  dropJob,
  isActiveJob,
  isMissingJobError,
  MAX_SOURCE_JOB_SNAPSHOTS,
  mergeJobSnapshots,
  recentCompletedJobs,
  sourceJobAttention,
  sourceJobBudgetSeconds,
  sourceJobElapsedSeconds,
  upsertJob,
  useSourceJobs,
} = await import('./sourceJobs')

test('upsertJob prepends new ids and replaces existing snapshots in place', () => {
  const first = jobOf('a', 'running')
  const second = jobOf('b', 'running')
  const updated = jobOf('a', 'succeeded', { summary: 'done' })

  const withNew = upsertJob([], first)
  expect(withNew.map((job) => job.id)).toEqual(['a'])

  const withSecond = upsertJob(withNew, second)
  expect(withSecond.map((job) => job.id)).toEqual(['b', 'a'])

  const replaced = upsertJob(withSecond, updated)
  expect(replaced.map((job) => job.id)).toEqual(['a', 'b'])
  expect(replaced[0]).toEqual(updated)
})

test('upsertJob does not regress a terminal snapshot to an older active poll', () => {
  const terminal = jobOf('a', 'succeeded')
  const staleActive = jobOf('a', 'running', { completed_at_unix_seconds: null })

  expect(upsertJob([terminal], staleActive)).toEqual([terminal])
})

test('upsertJob does not replace a newer terminal snapshot with an older completion', () => {
  const newer = jobOf('a', 'succeeded', { completed_at_unix_seconds: 1785000200 })
  const older = jobOf('a', 'failed', { completed_at_unix_seconds: 1785000100 })

  expect(upsertJob([newer], older)).toEqual([newer])
})

test('mergeJobSnapshots refreshes remembered ids without accepting stale recovery data', () => {
  const remembered = jobOf('job-1', 'running')
  const recovered = jobOf('job-1', 'succeeded', {
    summary: 'recovered completion',
    completed_at_unix_seconds: 1785000200,
  })
  expect(mergeJobSnapshots([remembered], [recovered])[0]).toEqual(recovered)

  const newer = jobOf('job-1', 'succeeded', {
    summary: 'newer renderer completion',
    completed_at_unix_seconds: 1785000300,
  })
  expect(mergeJobSnapshots([newer], [recovered])[0]).toEqual(newer)
})

test('upsertJob keeps the snapshot list bounded to the newest entries', () => {
  let jobs: DesktopSourceJob[] = []
  for (let index = 0; index < MAX_SOURCE_JOB_SNAPSHOTS + 3; index += 1) {
    jobs = upsertJob(jobs, jobOf(`job-${index}`, 'succeeded'))
  }
  expect(jobs).toHaveLength(MAX_SOURCE_JOB_SNAPSHOTS)
  expect(jobs[0]?.id).toBe(`job-${MAX_SOURCE_JOB_SNAPSHOTS + 2}`)
  expect(jobs.some((job) => job.id === 'job-0')).toBe(false)
})

test('dropJob removes only the requested id', () => {
  const jobs = [jobOf('a', 'running'), jobOf('b', 'succeeded'), jobOf('c', 'running')]
  const dropped = dropJob(jobs, 'b')
  expect(dropped.map((job) => job.id)).toEqual(['a', 'c'])
  expect(dropped).not.toBe(jobs)
})

test('activeJobs and activeJobIds filter to running and cancelling only', () => {
  const jobs = [
    jobOf('running', 'running'),
    jobOf('cancelling', 'cancelling'),
    jobOf('succeeded', 'succeeded'),
    jobOf('failed', 'failed'),
    jobOf('cancelled', 'cancelled'),
  ]
  expect(activeJobs(jobs).map((job) => job.id)).toEqual(['running', 'cancelling'])
  expect(activeJobIds(jobs)).toEqual(['running', 'cancelling'])
  expect(activeJobs([])).toEqual([])
  expect(isActiveJob(jobOf('x', 'running'))).toBe(true)
  expect(isActiveJob(jobOf('x', 'cancelling'))).toBe(true)
  expect(isActiveJob(jobOf('x', 'succeeded'))).toBe(false)
})

test('recentCompletedJobs keeps terminal snapshots for operational history', () => {
  const jobs = [
    jobOf('running', 'running'),
    jobOf('cancelled', 'cancelled'),
    jobOf('failed', 'failed'),
    jobOf('succeeded', 'succeeded'),
  ]
  expect(recentCompletedJobs(jobs).map((job) => job.id)).toEqual([
    'cancelled',
    'failed',
    'succeeded',
  ])
})

test('sourceJobAttention reports only the latest terminal failure per source', () => {
  const jobs = [
    jobOf('work-success', 'succeeded', { source: 'work-code' }),
    jobOf('personal-failure', 'failed', { source: 'personal-mail' }),
    jobOf('personal-old-success', 'succeeded', { source: 'personal-mail' }),
    jobOf('active', 'running', { source: 'special-code' }),
  ]
  expect(sourceJobAttention(jobs).map((job) => job.id)).toEqual(['personal-failure'])

  expect(
    sourceJobAttention([
      jobs[0]!,
      jobOf('personal-new-success', 'succeeded', { source: 'personal-mail' }),
      jobs[1]!,
    ])
  ).toEqual([])
})

test('sourceJobAttention keeps duplicate source names isolated by workspace', () => {
  const jobs = [
    jobOf('work-failure', 'failed', { source: 'notes', project: 'work' }),
    jobOf('personal-failure', 'failed', { source: 'notes', project: 'personal' }),
    jobOf('work-old-success', 'succeeded', { source: 'notes', project: 'work' }),
  ]

  expect(sourceJobAttention(jobs).map((job) => job.id)).toEqual([
    'work-failure',
    'personal-failure',
  ])
})

test('sourceJobAttention suppresses an older failure while a newer retry is active', () => {
  const activeRetry = jobOf('new-retry', 'running', { source: 'notes' })
  const oldFailure = jobOf('old-failure', 'failed', { source: 'notes' })

  expect(sourceJobAttention([activeRetry, oldFailure])).toEqual([])
})

test('source job progress reports native fixed budgets without claiming a percentage', () => {
  const validation = jobOf('validation', 'running', {
    started_at_unix_seconds: 1_000,
    budget: null,
  })
  const trial = jobOf('trial', 'running', {
    operation: 'trial-sync',
    started_at_unix_seconds: 1_000,
  })
  const initial = jobOf('initial', 'running', {
    operation: 'initial-sync',
    budget: 'medium',
    started_at_unix_seconds: 1_000,
  })
  const authorization = jobOf('authorization', 'running', {
    operation: 'authorization',
    started_at_unix_seconds: 1_000,
  })
  expect(sourceJobBudgetSeconds(validation)).toBe(60)
  expect(sourceJobBudgetSeconds(trial)).toBe(300)
  expect(sourceJobBudgetSeconds(initial)).toBe(1_800)
  expect(sourceJobBudgetSeconds(authorization)).toBe(300)
  expect(sourceJobElapsedSeconds(validation, 1_065)).toBe(65)
  expect(describeSourceJobProgress(validation, 1_065)).toBe('1m 5s / 1m')
  expect(describeSourceJobProgress(initial, 1_065)).toBe('1m 5s / 30m')
  expect(describeSourceJobProgress(authorization, 1_065)).toBe('1m 5s / 5m')
})

test('isMissingJobError matches only the native missing-job message', () => {
  expect(isMissingJobError(new Error('source job was not found'))).toBe(true)
  expect(isMissingJobError(new Error('backend offline'))).toBe(false)
  expect(isMissingJobError('source job was not found')).toBe(false)
  expect(isMissingJobError(null)).toBe(false)
})

test('describeSourceJob includes the source operation and current state', () => {
  expect(describeSourceJob(jobOf('job-1', 'running'))).toBe('work-code · validation · running')
})

test('the hook polls only active ids and keeps the latest snapshot', async () => {
  const running = jobOf('job-1', 'running')
  state.polled.set('job-1', running)
  const { result } = renderHook(() => useSourceJobs())

  // A terminal snapshot must be remembered but never polled.
  act(() => {
    result.current.remember(running)
    result.current.remember(jobOf('job-2', 'succeeded'))
  })
  expect(result.current.jobs).toHaveLength(2)

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(state.statusCalls).toContain('job-1')
  expect(state.statusCalls).not.toContain('job-2')

  // The next poll replaces the retained snapshot in place.
  state.polled.set('job-1', { ...running, status: 'cancelling', summary: 'cancelling now' })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(result.current.jobs[0]?.status).toBe('cancelling')
  expect(result.current.jobs[0]?.summary).toBe('cancelling now')
  expect(result.current.jobs).toHaveLength(2)
})

test('the hook recovers native source-job snapshots on mount', async () => {
  state.recovered = [jobOf('recovered-running', 'running'), jobOf('recovered-done', 'succeeded')]
  const { result, unmount } = renderHook(() => useSourceJobs())

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  expect(result.current.jobs.map((job) => job.id)).toEqual(['recovered-running', 'recovered-done'])
  unmount()
})

test('the hook lets recovery refresh a job remembered during mount', async () => {
  state.recovered = [
    jobOf('remembered-done', 'succeeded', {
      summary: 'recovered completion',
      completed_at_unix_seconds: 1785000200,
    }),
  ]
  const { result, unmount } = renderHook(() => useSourceJobs())
  act(() => result.current.remember(jobOf('remembered-done', 'running')))

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  expect(result.current.jobs[0]?.status).toBe('succeeded')
  expect(result.current.jobs[0]?.summary).toBe('recovered completion')
  unmount()
})

test('the hook drops an id on a missing-job error and retains snapshots on transient errors', async () => {
  state.polled.set('job-1', jobOf('job-1', 'running'))
  const { result } = renderHook(() => useSourceJobs())
  act(() => {
    result.current.remember(jobOf('job-1', 'running'))
  })

  // A transient status failure keeps the last snapshot.
  state.polled.set('job-1', new Error('backend offline'))
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(result.current.jobs.map((job) => job.id)).toEqual(['job-1'])

  // The native missing-job error drops the id entirely.
  state.polled.set('job-1', new Error('source job was not found'))
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(result.current.jobs).toEqual([])
  // After the drop there are no active ids left, so polling stops.
  expect(state.statusCalls).toEqual(['job-1', 'job-1'])
})

test('polling failure for an active source job surfaces a transient snapshot error', async () => {
  const running = jobOf('job-1', 'running')
  const { result } = renderHook(() => useSourceJobs())

  act(() => {
    result.current.remember(running)
  })
  state.polled.set('job-1', new Error('source job status transport failed'))

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(result.current.error).toBe('source job status transport failed')
  expect(result.current.jobs.map((job) => job.id)).toEqual(['job-1'])

  state.polled.set(
    'job-1',
    jobOf('job-1', 'succeeded', {
      summary: 'Validation succeeded.',
      completed_at_unix_seconds: 1785000100,
      exit_code: 0,
    })
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1200))
  })
  expect(result.current.error).toBe('')
  expect(result.current.jobs[0]?.status).toBe('succeeded')
})

test('the hook does not overlap source status polls while one batch is pending', async () => {
  const running = jobOf('job-1', 'running')
  let resolvePending: ((job: DesktopSourceJob) => void) | undefined
  state.pending = new Promise((resolve) => {
    resolvePending = resolve
  })
  const { result, unmount } = renderHook(() => useSourceJobs())
  act(() => result.current.remember(running))

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 2200))
  })
  expect(state.statusCalls).toEqual(['job-1'])

  resolvePending?.(running)
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  unmount()
})
