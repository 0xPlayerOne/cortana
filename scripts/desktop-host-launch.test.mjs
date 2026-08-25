import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import { expect, test } from 'bun:test'

import {
  buildIsolatedEnvironment,
  describeHostTarget,
  hostFailureEvidence,
  runHostLaunch,
  writeIsolatedConfig,
} from './desktop-host-launch.mjs'

test('host target metadata is limited to published desktop lanes', () => {
  expect(describeHostTarget('aarch64-apple-darwin')).toMatchObject({
    platform: 'macOS',
    architecture: 'arm64',
  })
  expect(describeHostTarget('x86_64-unknown-linux-gnu')).toMatchObject({
    platform: 'Linux',
    architecture: 'x64',
  })
  expect(describeHostTarget('x86_64-pc-windows-msvc')).toMatchObject({
    platform: 'Windows',
    architecture: 'x64',
  })
  expect(() => describeHostTarget('x86_64-unknown-freebsd')).toThrow('unsupported host target')
})

test('isolated host environment omits inherited credential-shaped variables', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-host-env-test-'))
  try {
    const environment = buildIsolatedEnvironment({
      root,
      configPath: join(root, 'config.toml'),
      baseEnvironment: {
        PATH: '/usr/bin',
        CORTANA_TEST_TOKEN: 'secret-value',
        API_KEY: 'secret-value',
      },
    })
    expect(environment).toMatchObject({
      CORTANA_CONFIG: join(root, 'config.toml'),
      HOME: root,
      XDG_CONFIG_HOME: join(root, 'xdg-config'),
      XDG_DATA_HOME: join(root, 'xdg-data'),
    })
    expect(environment.CORTANA_TEST_TOKEN).toBeUndefined()
    expect(environment.API_KEY).toBeUndefined()
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('isolated config is disposable and points runtime data inside the host root', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-host-config-test-'))
  try {
    const configPath = writeIsolatedConfig(root)
    expect(configPath).toBe(join(root, 'config.toml'))
    expect(readFileSync(configPath, 'utf8')).toContain(
      `data_dir = ${JSON.stringify(join(root, 'data'))}`
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('host launch requires the packaged process to survive the startup window', async () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-host-launch-test-'))
  const app = join(root, 'fake-app.mjs')
  try {
    writeFileSync(
      app,
      "process.stderr.write('host-launch-fixture\\n'); setTimeout(() => {}, 5000)\n"
    )
    chmodSync(app, 0o755)
    const result = await runHostLaunch({
      executable: process.execPath,
      args: [app],
      env: buildIsolatedEnvironment({ root, configPath: join(root, 'config.toml') }),
      stableMs: 100,
      timeoutMs: 2_000,
    })
    expect(result).toMatchObject({ status: 'passed', process: 'started-and-stopped' })
    expect(result.stderr).toContain('host-launch-fixture')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('host launch rejects credential-shaped environment keys', async () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-host-env-reject-test-'))
  const app = join(root, 'fake-app.mjs')
  try {
    writeFileSync(app, 'setTimeout(() => {}, 5000)\n')
    await expect(
      runHostLaunch({
        executable: process.execPath,
        args: [app],
        env: { PATH: '/usr/bin', API_KEY: 'must-not-forward' },
        stableMs: 100,
        timeoutMs: 2_000,
      })
    ).rejects.toThrow('credential-shaped key')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('host failure evidence is bounded and redacted', () => {
  expect(
    hostFailureEvidence({
      target: 'x86_64-pc-windows-msvc',
      version: '0.37.1',
      error: 'token=hidden-value; process failed',
    })
  ).toMatchObject({
    schema_version: 1,
    status: 'failed',
    error: 'token=[REDACTED]; process failed',
  })
})
