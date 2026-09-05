#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

import { buildIsolatedEnvironment, snapshotDirectory } from './desktop-control-plane-acceptance.mjs'
import { redactEvidence } from './desktop-package-acceptance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const COMMAND_TIMEOUT_MS = 60_000
const DRILL_PREFIX = 'cortana-package-service-status-'
const SERVICE_NAMES = Object.freeze(['embedding', 'server', 'sync', 'backup', 'vault'])
const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({ platform: 'macOS', architecture: 'arm64' }),
  'x86_64-unknown-linux-gnu': Object.freeze({ platform: 'Linux', architecture: 'x64' }),
  'x86_64-pc-windows-msvc': Object.freeze({ platform: 'Windows', architecture: 'x64' }),
})
const SERVICE_PLATFORM = Object.freeze({
  macOS: 'macos',
  Linux: 'linux',
  Windows: 'windows',
})

export const SERVICE_STATUS_CASES = Object.freeze([
  'packaged-service-status',
  'complete-managed-service-set',
  'read-only-state-unchanged',
  'no-mutating-action-requested',
])

export function describeServiceStatusTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported service-status target: ${target}`)
  return { ...descriptor, target }
}

export function buildServiceStatusPlan({ core }) {
  return [
    {
      name: 'packaged-service-status',
      command: core,
      args: ['service', 'status', '--json'],
    },
  ]
}

function boundedOutput(value) {
  return redactEvidence(String(value ?? ''))
    .replace(
      /(?:\/(?:Users|private|home|tmp|var|runner|workspace|builds|opt\/runner)\/[^\s\n]+|[A-Za-z]:\\[^\s\n]+|\\\\[^\s\n]+)/g,
      '[PATH]'
    )
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .slice(0, 1_000)
}

function commandFailure(result) {
  if (result.error) return boundedOutput(result.error.message)
  return boundedOutput(`exit=${result.status ?? 'null'} signal=${result.signal ?? 'null'}`)
}

function nullableString(value) {
  return value === null || typeof value === 'string'
}

function nullableInteger(value) {
  return value === null || (Number.isSafeInteger(value) && value >= 0)
}

export function validateServiceReport(report, target) {
  const descriptor = describeServiceStatusTarget(target)
  if (!report || typeof report !== 'object' || Array.isArray(report)) {
    throw new Error('service status report was not an object')
  }
  if (report.platform !== SERVICE_PLATFORM[descriptor.platform]) {
    throw new Error(`service status platform mismatch for ${target}`)
  }
  if (typeof report.supported !== 'boolean') {
    throw new Error('service status supported flag was invalid')
  }
  if (!Array.isArray(report.services) || report.services.length !== SERVICE_NAMES.length) {
    throw new Error('service status report did not contain all managed services')
  }
  const names = report.services.map((service) => service?.name)
  if (
    new Set(names).size !== SERVICE_NAMES.length ||
    !SERVICE_NAMES.every((name) => names.includes(name))
  ) {
    throw new Error('service status report contained unsupported or duplicate services')
  }
  for (const service of report.services) {
    if (
      service.label !== `ai.cortana.${service.name}` ||
      typeof service.installed !== 'boolean' ||
      typeof service.loaded !== 'boolean' ||
      !nullableString(service.state) ||
      !nullableInteger(service.pid) ||
      !nullableInteger(service.last_exit_status)
    ) {
      throw new Error(`service status fields were invalid for ${service.name}`)
    }
  }
  return {
    supported: report.supported,
    service_count: report.services.length,
  }
}

export function summarizeServiceStatus({
  target,
  version,
  steps,
  serviceManagerSupported,
  stateUnchanged,
  failures = [],
}) {
  const passedSteps = new Set(
    steps.filter((step) => step.status === 'passed').map((step) => step.name)
  )
  const allFailures = failures.map(boundedOutput)
  for (const name of SERVICE_STATUS_CASES) {
    if (name === 'packaged-service-status' && !passedSteps.has(name)) {
      allFailures.push(`missing required service-status case: ${name}`)
    }
  }
  if (serviceManagerSupported !== true && serviceManagerSupported !== false) {
    allFailures.push('service manager support was not reported')
  }
  if (stateUnchanged !== true) allFailures.push('read-only service status changed isolated state')
  return {
    schema_version: 1,
    status: allFailures.length === 0 ? 'passed' : 'failed',
    target: describeServiceStatusTarget(target),
    version,
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-package-service-status',
      prospective: 'prospective-source-service-status',
    }),
    cases: allFailures.length === 0 ? [...SERVICE_STATUS_CASES] : [...passedSteps],
    steps: steps.map(({ name, status, duration_ms }) => ({ name, status, duration_ms })),
    service_manager: {
      supported: serviceManagerSupported,
      operation: 'status-only',
    },
    state_unchanged: stateUnchanged === true,
    scope: {
      provider_network: 'not-requested',
      external_services: 'not_started',
      state: 'isolated-temporary-directory',
      service_mutation: 'not-requested',
    },
    limitations: [
      'This lane queries the packaged service manager status contract only; it does not install, start, stop, restart, or uninstall a service.',
      'A supported=false result records an unavailable service manager on the runner and is not evidence of successful service operation.',
      'Tray behavior, autostart, service lifecycle, crash recovery, and schedule installation require separate operator-controlled platform acceptance.',
    ],
    failures: allFailures,
  }
}

export function runServiceStatusAcceptance({ core, target, version, evidenceDirectory }) {
  if (!existsSync(core)) throw new Error(`packaged core does not exist: ${core}`)
  mkdirSync(evidenceDirectory, { recursive: true })
  const drillDirectory = mkdtempSync(join(tmpdir(), DRILL_PREFIX))
  const environment = buildIsolatedEnvironment({
    root: drillDirectory,
    configPath: join(drillDirectory, 'config.toml'),
  })
  const steps = []
  const failures = []
  let serviceManagerSupported
  let stateUnchanged = false

  try {
    const versionResult = spawnSync(core, ['--version'], {
      cwd: ROOT,
      encoding: 'utf8',
      env: environment,
      timeout: COMMAND_TIMEOUT_MS,
      windowsHide: true,
    })
    if (versionResult.error || versionResult.status !== 0) {
      failures.push(`packaged core version probe failed: ${commandFailure(versionResult)}`)
    } else if (versionResult.stdout?.trim() !== `cortana ${version}`) {
      failures.push(`packaged core version mismatch for ${version}`)
    }

    const stateBefore = JSON.stringify(snapshotDirectory(drillDirectory))
    if (failures.length === 0) {
      const [step] = buildServiceStatusPlan({ core })
      const startedAt = Date.now()
      const result = spawnSync(step.command, step.args, {
        cwd: ROOT,
        encoding: 'utf8',
        env: environment,
        timeout: COMMAND_TIMEOUT_MS,
        windowsHide: true,
      })
      const durationMs = Date.now() - startedAt
      if (result.error || result.status !== 0) {
        failures.push(`${step.name} failed: ${commandFailure(result)}`)
        steps.push({ name: step.name, status: 'failed', duration_ms: durationMs })
      } else {
        try {
          const report = JSON.parse(result.stdout)
          const summary = validateServiceReport(report, target)
          serviceManagerSupported = summary.supported
          steps.push({ name: step.name, status: 'passed', duration_ms: durationMs })
        } catch (error) {
          failures.push(error instanceof Error ? error.message : String(error))
          steps.push({ name: step.name, status: 'failed', duration_ms: durationMs })
        }
      }
    }
    stateUnchanged = JSON.stringify(snapshotDirectory(drillDirectory)) === stateBefore
    const evidence = summarizeServiceStatus({
      target,
      version,
      steps,
      serviceManagerSupported,
      stateUnchanged,
      failures,
    })
    const output = resolve(evidenceDirectory, `desktop-service-status-${target}.json`)
    writeFileSync(output, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 })
    return { ...evidence, output }
  } finally {
    rmSync(drillDirectory, { recursive: true, force: true })
  }
}

function parseArguments(args) {
  const values = {}
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]
    if (!argument.startsWith('--')) throw new Error(`unexpected argument: ${argument}`)
    const key = argument.slice(2)
    const value = args[index + 1]
    if (!value || value.startsWith('--')) throw new Error(`missing value for --${key}`)
    values[key] = value
    index += 1
  }
  return values
}

export function main(args = process.argv.slice(2)) {
  try {
    const values = parseArguments(args)
    const target = values.target
    const version = values.version
    const core = values.core
    const evidenceDirectory = values['evidence-dir']
    if (!target || !version || !core || !evidenceDirectory) {
      throw new Error(
        'usage: desktop-service-status-acceptance.mjs --target TARGET --version VERSION --core FILE --evidence-dir DIR'
      )
    }
    const evidence = runServiceStatusAcceptance({
      core: resolve(core),
      target,
      version,
      evidenceDirectory: resolve(evidenceDirectory),
    })
    console.log(`desktop service-status acceptance ${evidence.status}: ${evidence.output}`)
    if (evidence.status !== 'passed') process.exitCode = 1
    return evidence
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
    return null
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main()
