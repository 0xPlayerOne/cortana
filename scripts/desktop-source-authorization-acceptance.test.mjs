import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import {
  AUTHORIZATION_CASES,
  AUTHORIZATION_PROVIDERS,
  buildAuthorizationPlan,
  redactAuthorizationOutput,
  snapshotTree,
  summarizeAuthorizationEvidence,
} from './desktop-source-authorization-acceptance.mjs'

test('authorization plan covers every packaged provider safety preflight', () => {
  const plan = buildAuthorizationPlan({
    core: '/release/bin/cortana',
    config: '/tmp/isolated/config.toml',
    sources: Object.fromEntries(
      AUTHORIZATION_PROVIDERS.map((provider) => [provider, `${provider}-source`])
    ),
    missingSources: Object.fromEntries(
      AUTHORIZATION_PROVIDERS.map((provider) => [provider, `${provider}-missing-token`])
    ),
  })

  expect(plan.map((step) => step.name)).toEqual([
    'unknown-source-fails-closed',
    'google-missing-token-destination-fails-closed',
    'github-missing-token-destination-fails-closed',
    'discord-missing-token-destination-fails-closed',
    'slack-missing-token-destination-fails-closed',
    'google-malformed-oauth-client-fails-closed',
    'github-malformed-oauth-client-fails-closed',
    'discord-malformed-oauth-client-fails-closed',
    'slack-malformed-oauth-client-fails-closed',
  ])
  expect(plan.every((step) => step.args[0] === '/release/bin/cortana')).toBe(true)
  expect(plan.every((step) => step.args.includes('--config'))).toBe(true)
})

test('authorization evidence requires all provider failure cases and an unchanged state root', () => {
  const evidence = summarizeAuthorizationEvidence({
    target: 'aarch64-apple-darwin',
    version: '0.56.3',
    steps: AUTHORIZATION_CASES.map((name) => ({
      name,
      status: 'passed',
      ...(name === 'authorization-no-sync-side-effect' ? {} : { expected_failure: true }),
    })),
    stateChanged: false,
  })

  expect(evidence.status).toBe('passed')
  expect(evidence.cases).toEqual(AUTHORIZATION_CASES)
  expect(evidence.scope.state).toBe('isolated-temporary-directory')
  expect(evidence.scope.provider_network).toBe('not_requested')
})

test('authorization evidence fails closed when a provider case is missing', () => {
  const evidence = summarizeAuthorizationEvidence({
    target: 'x86_64-unknown-linux-gnu',
    version: '0.56.3',
    steps: AUTHORIZATION_CASES.filter(
      (name) => name !== 'slack-malformed-oauth-client-fails-closed'
    ).map((name) => ({ name, status: 'passed' })),
    stateChanged: false,
  })

  expect(evidence.status).toBe('failed')
  expect(evidence.failures).toContain(
    'missing or failed authorization case: slack-malformed-oauth-client-fails-closed'
  )
})

test('authorization evidence requires provider commands to be expected failures', () => {
  const evidence = summarizeAuthorizationEvidence({
    target: 'aarch64-apple-darwin',
    version: '0.56.3',
    steps: AUTHORIZATION_CASES.map((name) => ({
      name,
      status: 'passed',
      ...(name === 'authorization-no-sync-side-effect' ? {} : { expected_failure: false }),
    })),
    stateChanged: false,
  })

  expect(evidence.status).toBe('failed')
  expect(evidence.failures).toContain(
    'provider authorization case did not record an expected failure: google-malformed-oauth-client-fails-closed'
  )
})

test('authorization output redacts credentials and machine-specific paths', () => {
  expect(
    redactAuthorizationOutput(
      'OAuth client token=super-secret at /Users/example/.config/cortana/client.json'
    )
  ).toBe('OAuth client token=[REDACTED] at [PATH]')
})

test('authorization state snapshots ignore only the pre-created Rosetta runtime cache', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-source-authorization-snapshot-test-'))
  try {
    const rosettaCache = join(root, '.cache', 'rosetta')
    mkdirSync(rosettaCache, { recursive: true })
    const before = snapshotTree(root)
    writeFileSync(join(rosettaCache, 'runtime-marker'), 'emulator cache')
    const after = snapshotTree(root)

    expect(after).toEqual(before)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
