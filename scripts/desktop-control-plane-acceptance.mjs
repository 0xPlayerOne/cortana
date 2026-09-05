#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const COMMAND_TIMEOUT_MS = 60_000
const MAX_OUTPUT_LENGTH = 1_000
const DRILL_PREFIX = 'cortana-package-control-plane-'
const QUERY = 'verified backup restore drill'
const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({ platform: 'macOS', architecture: 'arm64' }),
  'x86_64-unknown-linux-gnu': Object.freeze({ platform: 'Linux', architecture: 'x64' }),
  'x86_64-pc-windows-msvc': Object.freeze({ platform: 'Windows', architecture: 'x64' }),
})
const SAFE_ENVIRONMENT_KEYS = Object.freeze([
  'PATH',
  'PATHEXT',
  'COMSPEC',
  'LANG',
  'LC_ALL',
  'SystemRoot',
  'WINDIR',
])

export const CONTROL_PLANE_CASES = Object.freeze([
  'init',
  'ingest',
  'search',
  'context',
  'audit-export',
  'backup',
  'init-restore',
  'restore',
  'recovery-invalid-restore',
  'verify',
  'post-restore-search',
])

export function describeControlPlaneTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported control-plane target: ${target}`)
  return { ...descriptor, target }
}

function redactEvidence(value) {
  return String(value)
    .replace(
      /\b(password|passwd|token|secret|api[_-]?key|private[_-]?key)\s*=\s*[^\s,;]+/gi,
      (_, key) => `${key}=[REDACTED]`
    )
    .replace(
      /(["']?)(password|passwd|token|secret|api[_-]?key|private[_-]?key)\1\s*:\s*(["']?)([^,"'\s}]+)\3/gi,
      (_, quote, key, valueQuote) => `${quote}${key}${quote}:${valueQuote}[REDACTED]${valueQuote}`
    )
    .replace(
      /(?:\/(?:Users|private|home|tmp|var|runner|workspace|builds|opt\/runner)\/[^\s\n]+|[A-Za-z]:\\[^\s\n]+|\\\\[^\s\n]+)/g,
      '[PATH]'
    )
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .slice(0, MAX_OUTPUT_LENGTH)
}

function executableOutput(result) {
  return `${result.stdout ?? ''}${result.stderr ?? ''}`
}

function commandFailure(step, result) {
  const detail = result.error
    ? result.error.message
    : `exit=${result.status ?? 'null'} signal=${result.signal ?? 'null'} output=${executableOutput(result)}`
  return new Error(`${step.name} failed: ${redactEvidence(detail)}`)
}

function assertOutput(step, expected) {
  if (!step.output.includes(expected)) {
    throw new Error(`${step.name} did not report expected marker: ${expected}`)
  }
}

function assertNonEmptyFile(path, label) {
  if (!existsSync(path) || readFileSync(path).length === 0) {
    throw new Error(`${label} was not created`)
  }
}

export function buildIsolatedEnvironment({ root, configPath, baseEnvironment = process.env }) {
  const stateRoot = resolve(root)
  const config = resolve(configPath)
  const configHome = resolve(stateRoot, 'xdg-config')
  const dataHome = resolve(stateRoot, 'xdg-data')
  const appData = resolve(stateRoot, 'appdata')
  const localAppData = resolve(stateRoot, 'local-appdata')
  const temporary = resolve(stateRoot, 'tmp')
  for (const directory of [stateRoot, configHome, dataHome, appData, localAppData, temporary]) {
    mkdirSync(directory, { recursive: true })
  }

  const environment = Object.fromEntries(
    SAFE_ENVIRONMENT_KEYS.flatMap((key) =>
      baseEnvironment[key] ? [[key, baseEnvironment[key]]] : []
    )
  )
  return {
    ...environment,
    HOME: stateRoot,
    USERPROFILE: stateRoot,
    APPDATA: appData,
    LOCALAPPDATA: localAppData,
    TEMP: temporary,
    TMP: temporary,
    TMPDIR: temporary,
    XDG_CONFIG_HOME: configHome,
    XDG_DATA_HOME: dataHome,
    CORTANA_CONFIG: config,
  }
}

function hashFile(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

export function snapshotDirectory(path) {
  const entries = []

  function visit(directory, relativeDirectory = '') {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((left, right) =>
      left.name.localeCompare(right.name)
    )) {
      const entryPath = join(directory, entry.name)
      const relativePath = relativeDirectory ? join(relativeDirectory, entry.name) : entry.name
      if (entry.isDirectory()) {
        visit(entryPath, relativePath)
      } else if (entry.isFile()) {
        entries.push({
          path: relativePath,
          sha256: hashFile(entryPath),
          bytes: statSync(entryPath).size,
        })
      } else {
        throw new Error(`recovery index contains unsupported entry: ${relativePath}`)
      }
    }
  }

  visit(path)
  return entries
}

export function assertDirectorySnapshotUnchanged(path, expected, label = 'recovery index') {
  const actual = snapshotDirectory(path)
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${label} changed after the rejected restore`)
  }
}

export function buildControlPlanePlan({
  core,
  config,
  restoreConfig,
  dataDirectory,
  restoreDataDirectory,
  fixture,
  snapshot,
  invalidSnapshot,
  auditExport,
}) {
  return [
    {
      name: 'init',
      command: core,
      args: ['init', '--offline', '--config', config, '--data-dir', dataDirectory],
    },
    {
      name: 'ingest',
      command: core,
      args: ['ingest', '--offline', '--config', config, fixture],
    },
    {
      name: 'search',
      command: core,
      args: ['search', '--offline', '--config', config, QUERY, '--limit', '3'],
    },
    {
      name: 'context',
      command: core,
      args: ['context', '--offline', '--config', config, QUERY, '--limit', '3'],
    },
    {
      name: 'audit-export',
      command: core,
      args: ['audit', 'export', '--offline', '--config', config, auditExport],
    },
    {
      name: 'backup',
      command: core,
      args: ['backup', '--offline', '--config', config, snapshot],
    },
    {
      name: 'init-restore',
      command: core,
      args: ['init', '--offline', '--config', restoreConfig, '--data-dir', restoreDataDirectory],
    },
    {
      name: 'restore',
      command: core,
      args: ['restore', '--offline', '--config', restoreConfig, snapshot, '--force'],
    },
    {
      name: 'recovery-invalid-restore',
      command: core,
      args: ['restore', '--offline', '--config', restoreConfig, invalidSnapshot, '--force'],
      expected_failure: true,
    },
    {
      name: 'verify',
      command: core,
      args: ['verify', '--offline', '--config', restoreConfig],
    },
    {
      name: 'post-restore-search',
      command: core,
      args: ['search', '--offline', '--config', restoreConfig, QUERY, '--limit', '3'],
    },
  ]
}

function runStep(step, environment) {
  const startedAt = Date.now()
  const result = spawnSync(step.command, step.args, {
    cwd: ROOT,
    encoding: 'utf8',
    env: environment,
    timeout: COMMAND_TIMEOUT_MS,
    windowsHide: true,
  })
  const expectedFailure = step.expected_failure === true
  const failedToExecute = Boolean(result.error)
  const returnedFailure = !failedToExecute && result.status !== 0
  const completed = {
    name: step.name,
    status:
      (!expectedFailure && !failedToExecute && !returnedFailure) ||
      (expectedFailure && returnedFailure)
        ? 'passed'
        : 'failed',
    duration_ms: Date.now() - startedAt,
    output: executableOutput(result),
  }
  if (completed.status !== 'passed') {
    if (expectedFailure && !failedToExecute && !returnedFailure) {
      throw new Error(`${step.name} unexpectedly succeeded`)
    }
    throw commandFailure(step, result)
  }
  return completed
}

export function summarizeControlPlane({
  target,
  version,
  steps,
  preflight = 'passed',
  failures = [],
  recovery = {},
}) {
  const seen = new Set(steps.filter((step) => step.status === 'passed').map((step) => step.name))
  const allFailures = failures.map(redactEvidence)
  if (preflight !== 'passed') allFailures.unshift('packaged core version preflight failed')
  for (const name of CONTROL_PLANE_CASES) {
    if (!seen.has(name)) allFailures.push(`missing required control-plane case: ${name}`)
  }
  for (const step of steps) {
    if (step.status !== 'passed') allFailures.push(`${step.name} did not pass`)
  }
  if (recovery.invalid_restore_preserved_index !== true) {
    allFailures.push('invalid restore did not prove active index preservation')
  }
  return {
    schema_version: 1,
    status: allFailures.length === 0 ? 'passed' : 'failed',
    target: describeControlPlaneTarget(target),
    version,
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-package-control-plane',
      prospective: 'prospective-source-control-plane',
    }),
    preflight,
    cases: CONTROL_PLANE_CASES.filter((name) => seen.has(name)),
    steps: steps.map(({ name, status, duration_ms }) => ({ name, status, duration_ms })),
    failures: allFailures,
    recovery: {
      invalid_restore_preserved_index: recovery.invalid_restore_preserved_index === true,
    },
    scope: {
      network: 'not-requested',
      network_enforcement: 'not-asserted',
      external_services: 'not_started',
      state: 'isolated-temporary-directory',
    },
    limitations: [
      'This lane verifies packaged core control-plane behavior only.',
      'The offline flag prevents provider-backed work in this plan; the runner network is not OS-isolated by this lane.',
      'Desktop GUI, native dialogs, services, tray, OAuth, updater lifecycle, accessibility, recovery UI, uninstall, and OS trust require separate acceptance evidence.',
    ],
  }
}

export function runControlPlaneAcceptance({ core, target, version, evidenceDirectory }) {
  if (!existsSync(core)) throw new Error(`packaged core does not exist: ${core}`)
  mkdirSync(evidenceDirectory, { recursive: true })
  const drillDirectory = mkdtempSync(join(tmpdir(), DRILL_PREFIX))
  const config = join(drillDirectory, 'config-1.toml')
  const restoreConfig = join(drillDirectory, 'config-2.toml')
  const dataDirectory = join(drillDirectory, 'data-1')
  const restoreDataDirectory = join(drillDirectory, 'data-2')
  const fixture = join(drillDirectory, 'fixture.jsonl')
  const snapshot = join(drillDirectory, 'snapshot.sqlite3')
  const auditExport = join(drillDirectory, 'audit.jsonl')
  const invalidSnapshot = join(drillDirectory, 'invalid.sqlite3')
  const environment = buildIsolatedEnvironment({ root: drillDirectory, configPath: config })
  const steps = []
  const failures = []
  let recovery = { invalid_restore_preserved_index: false }
  let restoredIndexSnapshot
  let preflight = 'passed'

  writeFileSync(
    fixture,
    [
      JSON.stringify({
        source: 'control-plane-acceptance',
        source_id: 'package-drill-001',
        title: 'Cortana packaged control plane',
        content:
          'The packaged control plane verifies initialization, bounded ingestion, retrieval, audit export, verified backup, restore, and post-restore verification.',
      }),
      JSON.stringify({
        source: 'control-plane-acceptance',
        source_id: 'package-drill-002',
        title: 'Offline package bounds',
        content: 'All package acceptance state is isolated in a disposable temporary directory.',
      }),
    ].join('\n') + '\n',
    { mode: 0o600 }
  )
  writeFileSync(invalidSnapshot, 'not a sqlite database\n', { mode: 0o600 })

  try {
    const versionResult = spawnSync(core, ['--version'], {
      cwd: ROOT,
      encoding: 'utf8',
      env: environment,
      timeout: COMMAND_TIMEOUT_MS,
      windowsHide: true,
    })
    const reportedVersion = versionResult.stdout?.trim()
    if (
      versionResult.error ||
      versionResult.status !== 0 ||
      reportedVersion !== `cortana ${version}`
    ) {
      preflight = 'failed'
      failures.push(
        redactEvidence(
          `expected packaged core version cortana ${version}, got ${reportedVersion || executableOutput(versionResult)}`
        )
      )
    }

    if (preflight === 'passed') {
      const plan = buildControlPlanePlan({
        core,
        config,
        restoreConfig,
        dataDirectory,
        restoreDataDirectory,
        fixture,
        snapshot,
        invalidSnapshot,
        auditExport,
      })
      for (const step of plan) {
        try {
          const result = runStep(step, environment)
          steps.push(result)
          if (
            step.name === 'search' ||
            step.name === 'context' ||
            step.name === 'post-restore-search'
          ) {
            assertOutput(result, 'package-drill-001')
          }
          if (step.name === 'audit-export') {
            assertNonEmptyFile(auditExport, 'audit export')
            const audit = readFileSync(auditExport, 'utf8')
            if (audit.includes(QUERY) || audit.includes('packaged control plane')) {
              throw new Error('audit export contains query or document content')
            }
            assertOutput(result, 'audit')
          }
          if (step.name === 'ingest')
            assertNonEmptyFile(join(dataDirectory, 'cortana.sqlite3'), 'primary index')
          if (step.name === 'backup') {
            assertNonEmptyFile(snapshot, 'backup snapshot')
            assertOutput(result, 'backup verified')
          }
          if (step.name === 'restore') {
            assertNonEmptyFile(join(restoreDataDirectory, 'cortana.sqlite3'), 'restored index')
            assertOutput(result, 'database restored')
            restoredIndexSnapshot = snapshotDirectory(restoreDataDirectory)
          }
          if (step.name === 'recovery-invalid-restore') {
            if (!restoredIndexSnapshot) {
              throw new Error('restored index snapshot was not captured before recovery check')
            }
            assertDirectorySnapshotUnchanged(
              restoreDataDirectory,
              restoredIndexSnapshot,
              'restored index'
            )
            recovery = { invalid_restore_preserved_index: true }
          }
          if (step.name === 'verify') assertOutput(result, 'database verified')
        } catch (error) {
          failures.push(redactEvidence(error instanceof Error ? error.message : error))
          break
        }
      }
    }

    const evidence = summarizeControlPlane({
      target,
      version,
      steps,
      preflight,
      failures,
      recovery,
    })
    const output = resolve(evidenceDirectory, `desktop-control-plane-${target}.json`)
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
        'usage: desktop-control-plane-acceptance.mjs --target TARGET --version VERSION --core FILE --evidence-dir DIR'
      )
    }
    const evidence = runControlPlaneAcceptance({
      core: resolve(core),
      target,
      version,
      evidenceDirectory: resolve(evidenceDirectory),
    })
    console.log(`desktop control-plane acceptance ${evidence.status}: ${evidence.output}`)
    if (evidence.status !== 'passed') process.exitCode = 1
    return evidence
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
    return null
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main()
