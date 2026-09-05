import { expect, test } from 'bun:test'
import { join } from 'node:path'

import {
  SERVICE_STATUS_CASES,
  buildServiceStatusPlan,
  describeServiceStatusTarget,
  summarizeServiceStatus,
  validateServiceReport,
} from './desktop-service-status-acceptance.mjs'

function report(overrides = {}) {
  return {
    platform: 'macos',
    supported: true,
    services: ['embedding', 'server', 'sync', 'backup', 'vault'].map((name) => ({
      name,
      label: `ai.cortana.${name}`,
      installed: false,
      loaded: false,
      state: null,
      pid: null,
      last_exit_status: null,
    })),
    ...overrides,
  }
}

test('service status plan is read-only and does not include lifecycle actions', () => {
  const plan = buildServiceStatusPlan({ core: '/tmp/cortana/bin/cortana' })
  expect(plan).toEqual([
    {
      name: 'packaged-service-status',
      command: '/tmp/cortana/bin/cortana',
      args: ['service', 'status', '--json'],
    },
  ])
  expect(plan.flatMap((step) => step.args)).not.toContain('install')
  expect(plan.flatMap((step) => step.args)).not.toContain('start')
  expect(plan.flatMap((step) => step.args)).not.toContain('stop')
  expect(plan.flatMap((step) => step.args)).not.toContain('restart')
  expect(plan.flatMap((step) => step.args)).not.toContain('uninstall')
})

test('service status validation requires the target platform and complete schema', () => {
  expect(describeServiceStatusTarget('aarch64-apple-darwin')).toEqual({
    target: 'aarch64-apple-darwin',
    platform: 'macOS',
    architecture: 'arm64',
  })
  expect(validateServiceReport(report(), 'aarch64-apple-darwin')).toEqual({
    supported: true,
    service_count: 5,
  })
  expect(() =>
    validateServiceReport(report({ platform: 'linux' }), 'aarch64-apple-darwin')
  ).toThrow('platform mismatch')
  expect(() =>
    validateServiceReport(report({ services: report().services.slice(1) }), 'aarch64-apple-darwin')
  ).toThrow('all managed services')
  expect(() =>
    validateServiceReport(
      report({ services: report().services.map((service) => ({ ...service, pid: -1 })) }),
      'aarch64-apple-darwin'
    )
  ).toThrow('fields were invalid')
})

test('service status summary fails closed on state mutation or missing service manager result', () => {
  const steps = [{ name: 'packaged-service-status', status: 'passed', duration_ms: 1 }]
  expect(
    summarizeServiceStatus({
      target: 'x86_64-unknown-linux-gnu',
      version: '0.56.3',
      steps,
      serviceManagerSupported: false,
      stateUnchanged: true,
    })
  ).toMatchObject({
    status: 'passed',
    installation_type: 'published-package-service-status',
    cases: SERVICE_STATUS_CASES,
    service_manager: { supported: false, operation: 'status-only' },
    state_unchanged: true,
    scope: {
      provider_network: 'not-requested',
      external_services: 'not_started',
      service_mutation: 'not-requested',
    },
  })
  expect(
    summarizeServiceStatus({
      target: 'x86_64-unknown-linux-gnu',
      version: '0.56.3',
      steps,
      serviceManagerSupported: undefined,
      stateUnchanged: false,
    })
  ).toMatchObject({
    status: 'failed',
    failures: [
      'service manager support was not reported',
      'read-only service status changed isolated state',
    ],
  })
})

test('service status summary keeps evidence paths out of its JSON contract', () => {
  const evidence = summarizeServiceStatus({
    target: 'x86_64-unknown-linux-gnu',
    version: '0.56.3',
    steps: [{ name: 'packaged-service-status', status: 'passed', duration_ms: 1 }],
    serviceManagerSupported: true,
    stateUnchanged: true,
    failures: [join('/Users/private', 'token=secret')],
  })
  expect(JSON.stringify(evidence)).not.toContain('/Users/private')
  expect(JSON.stringify(evidence)).not.toContain('secret')
})
