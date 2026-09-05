import { expect, test } from 'bun:test'
import { join } from 'node:path'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'

import {
  CONTROL_PLANE_CASES,
  buildControlPlanePlan,
  buildIsolatedEnvironment,
  assertDirectorySnapshotUnchanged,
  describeControlPlaneTarget,
  snapshotDirectory,
  summarizeControlPlane,
} from './desktop-control-plane-acceptance.mjs'

test('control-plane plan keeps every packaged step offline and explicitly scoped', () => {
  const plan = buildControlPlanePlan({
    core: '/tmp/cortana/bin/cortana',
    config: '/tmp/cortana/config-1.toml',
    restoreConfig: '/tmp/cortana/config-2.toml',
    dataDirectory: '/tmp/cortana/data-1',
    restoreDataDirectory: '/tmp/cortana/data-2',
    fixture: '/tmp/cortana/fixture.jsonl',
    snapshot: '/tmp/cortana/snapshot.sqlite3',
    invalidSnapshot: '/tmp/cortana/invalid.sqlite3',
    auditExport: '/tmp/cortana/audit.jsonl',
  })

  expect(plan.map((step) => step.name)).toEqual(CONTROL_PLANE_CASES)
  for (const step of plan) {
    expect(step.command).toBe('/tmp/cortana/bin/cortana')
    expect(step.args).toContain('--offline')
    expect(step.args).toContain('--config')
  }
  expect(plan.find((step) => step.name === 'backup').args).toContain(
    '/tmp/cortana/snapshot.sqlite3'
  )
  expect(plan.find((step) => step.name === 'restore').args).toContain('--force')
  expect(plan.find((step) => step.name === 'recovery-invalid-restore')).toMatchObject({
    expected_failure: true,
    args: expect.arrayContaining(['/tmp/cortana/invalid.sqlite3', '--force']),
  })
})

test('control-plane summary fails closed unless every required step passes', () => {
  const steps = CONTROL_PLANE_CASES.map((name) => ({
    name,
    status: 'passed',
    duration_ms: 1,
  }))
  expect(
    summarizeControlPlane({
      target: 'x86_64-unknown-linux-gnu',
      version: '0.56.3',
      steps,
      recovery: { invalid_restore_preserved_index: true },
    })
  ).toMatchObject({
    status: 'passed',
    installation_type: 'published-package-control-plane',
    target: {
      target: 'x86_64-unknown-linux-gnu',
      platform: 'Linux',
      architecture: 'x64',
    },
    cases: CONTROL_PLANE_CASES,
    recovery: { invalid_restore_preserved_index: true },
    scope: {
      network: 'not-requested',
      network_enforcement: 'not-asserted',
    },
  })

  const incomplete = steps.slice(0, -1)
  expect(
    summarizeControlPlane({
      target: 'x86_64-unknown-linux-gnu',
      version: '0.56.3',
      steps: incomplete,
      recovery: { invalid_restore_preserved_index: true },
    })
  ).toMatchObject({
    status: 'failed',
    failures: ['missing required control-plane case: post-restore-search'],
  })
})

test('control-plane target metadata is limited to supported release lanes', () => {
  expect(describeControlPlaneTarget('aarch64-apple-darwin')).toEqual({
    target: 'aarch64-apple-darwin',
    platform: 'macOS',
    architecture: 'arm64',
  })
  expect(() => describeControlPlaneTarget('unknown-target')).toThrow(
    'unsupported control-plane target'
  )
})

test('recovery snapshot detects any active-index mutation', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-control-plane-recovery-test-'))
  try {
    const data = join(root, 'data')
    const nested = join(data, 'nested')
    const index = join(data, 'cortana.sqlite3')
    const sidecar = join(nested, 'index-shm')
    mkdirSync(nested, { recursive: true })
    writeFileSync(index, 'stable index')
    writeFileSync(sidecar, 'stable sidecar')

    const snapshot = snapshotDirectory(data)
    expect(snapshot).toHaveLength(2)
    expect(() => assertDirectorySnapshotUnchanged(data, snapshot)).not.toThrow()

    writeFileSync(index, 'mutated index')
    expect(() => assertDirectorySnapshotUnchanged(data, snapshot)).toThrow(
      'recovery index changed after the rejected restore'
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('control-plane failure evidence redacts common runner and workspace paths', () => {
  const steps = CONTROL_PLANE_CASES.map((name) => ({
    name,
    status: 'passed',
    duration_ms: 1,
  }))
  const evidence = summarizeControlPlane({
    target: 'x86_64-unknown-linux-gnu',
    version: '0.56.3',
    steps,
    failures: ['/tmp/runner/failure /home/runner/work/repo /var/folders/secret /Users/private'],
    recovery: { invalid_restore_preserved_index: true },
  })

  expect(evidence.failures).toEqual(['[PATH] [PATH] [PATH] [PATH]'])
})

test('control-plane environment allowlists runner state and strips credentials and endpoints', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-control-plane-environment-test-'))
  try {
    const environment = buildIsolatedEnvironment({
      root,
      configPath: join(root, 'config.toml'),
      baseEnvironment: {
        PATH: '/usr/bin',
        CORTANA_TOKEN: 'must-not-propagate',
        CORTANA_BASE_URL: 'https://must-not-propagate.invalid',
        HOME: '/private/user-home',
      },
    })
    expect(environment.PATH).toBe('/usr/bin')
    expect(environment.CORTANA_TOKEN).toBeUndefined()
    expect(environment.CORTANA_BASE_URL).toBeUndefined()
    expect(environment.HOME).toBe(root)
    expect(environment.CORTANA_CONFIG).toBe(join(root, 'config.toml'))
    expect(environment.TMPDIR).toBe(join(root, 'tmp'))
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
