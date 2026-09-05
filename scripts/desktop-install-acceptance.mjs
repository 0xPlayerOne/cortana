#!/usr/bin/env node

import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { spawnSync } from 'node:child_process'
import { basename, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

import { redactEvidence, validateEvidenceOutputPath } from './desktop-package-acceptance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const COMMAND_TIMEOUT_MS = 120_000
const MAX_OUTPUT_LENGTH = 1_000

const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({ platform: 'macOS', architecture: 'arm64' }),
  'x86_64-unknown-linux-gnu': Object.freeze({ platform: 'Linux', architecture: 'x64' }),
  'x86_64-pc-windows-msvc': Object.freeze({ platform: 'Windows', architecture: 'x64' }),
})

function redactInstallOutput(value, root) {
  return redactEvidence(
    String(value ?? '')
      .replaceAll(root, '[INSTALL_ROOT]')
      .replace(
        /(?:\/(?:Users|private|home|tmp|var|runner|workspace|builds|opt\/runner)\/[^\s\n]+|[A-Za-z]:\\[^\s\n]+|\\\\[^\s\n]+)/g,
        '[PATH]'
      )
  )
}

export function describeInstallTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported installer target: ${target}`)
  return { ...descriptor, target }
}

export function buildWindowsMsiInstallArguments({ msiPath, installDirectory, logPath }) {
  return [
    '/i',
    resolve(msiPath),
    '/quiet',
    '/norestart',
    `INSTALLDIR=${resolve(installDirectory)}`,
    '/l*v',
    resolve(logPath),
  ]
}

export function buildWindowsMsiUninstallArguments({ msiPath, logPath }) {
  return ['/x', resolve(msiPath), '/quiet', '/norestart', '/l*v', resolve(logPath)]
}

function requireExecutable(path, label) {
  if (!existsSync(path)) throw new Error(`${label} is missing: ${path}`)
  try {
    accessSync(path, constants.X_OK)
  } catch {
    throw new Error(`${label} is not executable: ${path}`)
  }
}

export function validateArchiveRoot(archiveRoot, version) {
  const root = resolve(archiveRoot)
  if (!existsSync(root)) throw new Error(`release archive directory does not exist: ${root}`)
  requireExecutable(resolve(root, 'install.sh'), 'release installer')
  requireExecutable(resolve(root, 'bin', 'cortana'), 'release binary')
  if (!existsSync(resolve(root, 'share', 'cortana', 'web', 'index.html')))
    throw new Error('release workspace is missing: share/cortana/web/index.html')
  const wheelDirectory = resolve(root, 'dist')
  const wheels = existsSync(wheelDirectory)
    ? readdirSync(wheelDirectory).filter((name) => /^cortana_brain-[^/]+\.whl$/.test(name))
    : []
  if (wheels.length !== 1) {
    throw new Error(`release connector wheel count must be exactly one, found ${wheels.length}`)
  }
  if (version && !wheels[0].startsWith(`cortana_brain-${version}-`)) {
    throw new Error(`release connector wheel version mismatch: expected ${version}`)
  }
  return {
    root,
    installer: resolve(root, 'install.sh'),
    core: resolve(root, 'bin', 'cortana'),
    wheel: resolve(wheelDirectory, wheels[0]),
  }
}

export function buildInstallEnvironment({ root, baseEnvironment = process.env }) {
  const stateRoot = resolve(root)
  const configHome = resolve(stateRoot, 'config')
  const dataHome = resolve(stateRoot, 'data')
  const prefix = resolve(stateRoot, 'prefix')
  const tempHome = resolve(stateRoot, 'tmp')
  const appData = resolve(stateRoot, 'appdata')
  const localAppData = resolve(stateRoot, 'localappdata')
  const config = resolve(configHome, 'cortana', 'config.toml')
  for (const directory of [
    stateRoot,
    configHome,
    dataHome,
    prefix,
    tempHome,
    appData,
    localAppData,
  ]) {
    mkdirSync(directory, { recursive: true })
  }
  const environment = {}
  for (const key of ['PATH', 'LANG', 'LC_ALL', 'SSL_CERT_DIR', 'SSL_CERT_FILE']) {
    if (baseEnvironment[key]) environment[key] = baseEnvironment[key]
  }
  return {
    ...environment,
    HOME: stateRoot,
    USERPROFILE: stateRoot,
    TMPDIR: tempHome,
    TEMP: tempHome,
    TMP: tempHome,
    APPDATA: appData,
    LOCALAPPDATA: localAppData,
    XDG_CONFIG_HOME: configHome,
    XDG_DATA_HOME: dataHome,
    CORTANA_CONFIG: config,
    CORTANA_INSTALL_PREFIX: prefix,
    CORTANA_INSTALL_SERVICE: '0',
    CORTANA_ENABLE_SYNC_SERVICE: '0',
    CORTANA_INSTALL_AGENT_INTEGRATIONS: '0',
  }
}

function runCommand(command, args, { cwd, env, root }) {
  const startedAt = Date.now()
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
    timeout: COMMAND_TIMEOUT_MS,
    windowsHide: true,
  })
  const output = `${result.stdout ?? ''}${result.stderr ?? ''}`
  const evidence = {
    status: result.error || result.status !== 0 ? 'failed' : 'passed',
    exit_code: result.status,
    duration_ms: Date.now() - startedAt,
    stdout: redactInstallOutput(result.stdout, root),
    stderr: redactInstallOutput(result.stderr, root),
    output: redactInstallOutput(output, root).slice(0, MAX_OUTPUT_LENGTH),
  }
  if (result.error || result.status !== 0) {
    const detail = result.error?.message ?? `exit=${result.status ?? 'null'}`
    throw new Error(`command failed: ${redactInstallOutput(detail, root)}`)
  }
  return {
    evidence,
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
  }
}

function installedPaths(environment) {
  const prefix = environment.CORTANA_INSTALL_PREFIX
  return {
    binary: resolve(prefix, 'bin', 'cortana'),
    web: resolve(prefix, 'share', 'cortana', 'web', 'index.html'),
    connector: resolve(prefix, 'share', 'cortana', 'venv', 'bin', 'cortana-connectors'),
    config: environment.CORTANA_CONFIG,
    schedule: resolve(prefix, 'share', 'cortana', 'service-schedule.toml'),
  }
}

function inspectInstalledState(environment) {
  const paths = installedPaths(environment)
  for (const [label, path] of Object.entries({
    binary: paths.binary,
    web: paths.web,
    connector: paths.connector,
    config: paths.config,
  })) {
    if (!existsSync(path)) throw new Error(`installed ${label} is missing`)
  }
  const config = readFileSync(paths.config, 'utf8')
  const querySection = config.match(/(?:^|\n)\[query\]\n([\s\S]*?)(?=\n\[|$)/)?.[1] ?? ''
  const queryOnlyDefault = !/^\s*synthesis_enabled\s*=\s*true\s*$/m.test(querySection)
  const noSourceOrScheduleSideEffects =
    !/^\s*\[\[sources\]\]\s*$/m.test(config) && !existsSync(paths.schedule)
  return {
    query_only_default: queryOnlyDefault,
    no_source_or_schedule_side_effects: noSourceOrScheduleSideEffects,
    connector_installed_by_explicit_installer: true,
  }
}

function runInstalledCore(core, version, environment, root) {
  const versionRun = runCommand(core, ['--version'], { cwd: ROOT, env: environment, root })
  const reportedVersion = versionRun.stdout.trim().split(/\r?\n/)[0]
  if (reportedVersion !== `cortana ${version}`) {
    throw new Error(`installed core version mismatch: expected cortana ${version}`)
  }
  const evaluation = runCommand(
    core,
    ['--config', environment.CORTANA_CONFIG, '--offline', 'eval'],
    { cwd: ROOT, env: environment, root }
  )
  let report
  try {
    report = JSON.parse(evaluation.stdout)
  } catch {
    throw new Error('installed core offline evaluation was not JSON')
  }
  if (report?.passed !== true) throw new Error('installed core offline evaluation did not pass')
  return {
    reported_version: reportedVersion,
    offline_evaluation: 'passed',
    command: { version: versionRun.evidence, evaluation: evaluation.evidence },
  }
}

function findInstalledFile(root, fileName) {
  if (!existsSync(root)) return null
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = resolve(root, entry.name)
    if (entry.isDirectory()) {
      const nested = findInstalledFile(path, fileName)
      if (nested) return nested
    } else if (entry.isFile() && entry.name.toLowerCase() === fileName.toLowerCase()) {
      return path
    }
  }
  return null
}

export function runWindowsMsiAcceptance({ target, version, msiPath }) {
  if (process.platform !== 'win32') {
    throw new Error('Windows MSI acceptance must run on Windows')
  }
  const descriptor = describeInstallTarget(target)
  const msi = resolve(msiPath)
  const expectedName = `Cortana_${version}_x64_en-US.msi`
  if (basename(msi) !== expectedName) {
    throw new Error(`Windows MSI filename mismatch: expected ${expectedName}`)
  }
  if (!existsSync(msi)) throw new Error(`Windows MSI does not exist: ${msi}`)

  const stateRoot = mkdtempSync(resolve(tmpdir(), 'cortana-msi-install-acceptance-'))
  const environment = buildInstallEnvironment({ root: stateRoot })
  const installDirectory = resolve(stateRoot, 'installed')
  const installLog = resolve(stateRoot, 'msiexec-install.log')
  const uninstallLog = resolve(stateRoot, 'msiexec-uninstall.log')
  mkdirSync(installDirectory, { recursive: true })
  environment.CORTANA_INSTALL_PREFIX = installDirectory
  writeFileSync(environment.CORTANA_CONFIG, '[query]\n', { mode: 0o600 })

  let installAttempted = false
  let evidence
  let failure
  let uninstall
  let cleanup
  try {
    installAttempted = true
    const installResult = runCommand(
      'msiexec.exe',
      buildWindowsMsiInstallArguments({ msiPath: msi, installDirectory, logPath: installLog }),
      { cwd: stateRoot, env: environment, root: stateRoot }
    )
    const desktop = findInstalledFile(installDirectory, 'cortana-desktop.exe')
    const corePath = findInstalledFile(installDirectory, 'cortana.exe')
    const webIndex = findInstalledFile(installDirectory, 'index.html')
    if (!desktop) throw new Error('Windows MSI installed application executable is missing')
    if (!corePath) throw new Error('Windows MSI installed packaged core is missing')
    if (!webIndex) throw new Error('Windows MSI installed web assets are missing')
    const core = runInstalledCore(corePath, version, environment, stateRoot)
    evidence = {
      schema_version: 1,
      status: 'passed',
      target: descriptor,
      version,
      installation_type: resolveAcceptanceInstallationType({
        published: 'published-release-installer',
        prospective: 'prospective-source-installer',
      }),
      cases: [
        'clean-install-msi',
        'installed-version',
        'installed-web-assets',
        'installed-core-sidecar',
        'installed-core-offline-evaluation',
        'clean-uninstall-msi',
      ],
      install: installResult.evidence,
      uninstall: null,
      first_run: {
        status: 'not_exercised',
        limitation: 'desktop first-run UI requires a separate packaged host lane',
      },
      core,
      paths: {
        desktop: 'installed/cortana-desktop.exe',
        core: 'installed/cortana.exe',
        web: 'installed/**/index.html',
        config: 'config/cortana/config.toml',
      },
      services: {
        status: 'not_exercised',
        reason: 'MSI installer acceptance does not install background services',
      },
      scope: {
        provider_network: 'not_requested',
        installer_dependency_network: 'may_be_used',
        external_services: 'not_started',
        state: 'isolated-temporary-directory',
      },
      limitations: [
        'This lane verifies Windows MSI installation, packaged files, core evaluation, and clean uninstall.',
        'Desktop first-run UI, OAuth, services, tray, native dialogs, updater lifecycle, recovery UI, accessibility, and OS trust require separate acceptance evidence.',
      ],
      reviewer: 'automated acceptance lane',
      generated_at: new Date().toISOString(),
    }
  } catch (error) {
    failure = error
  }

  if (installAttempted) {
    try {
      const uninstallResult = runCommand(
        'msiexec.exe',
        buildWindowsMsiUninstallArguments({ msiPath: msi, logPath: uninstallLog }),
        { cwd: stateRoot, env: environment, root: stateRoot }
      )
      const remaining = ['cortana-desktop.exe', 'cortana.exe', 'index.html'].filter((fileName) =>
        findInstalledFile(installDirectory, fileName)
      )
      if (remaining.length > 0) {
        throw new Error(`Windows MSI uninstall left installed files: ${remaining.join(', ')}`)
      }
      uninstall = uninstallResult.evidence
    } catch (error) {
      if (!failure) failure = error
    }
  }

  try {
    rmSync(stateRoot, { recursive: true, force: true })
    cleanup = { status: 'passed', state_root_removed: !existsSync(stateRoot) }
    if (!cleanup.state_root_removed) throw new Error('Windows MSI acceptance state was not removed')
  } catch (error) {
    if (!failure) failure = error
  }
  if (failure) throw failure
  if (!evidence || !uninstall || !cleanup)
    throw new Error('Windows MSI acceptance did not produce complete evidence')
  evidence.uninstall = uninstall
  evidence.cleanup = cleanup
  return evidence
}

export function installFailureEvidence({ target, version, error }) {
  return {
    schema_version: 1,
    status: 'failed',
    ...(target ? { target: failureTargetMetadata(target) } : {}),
    ...(version ? { version } : {}),
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-release-installer',
      prospective: 'prospective-source-installer',
    }),
    error: redactEvidence(error instanceof Error ? error.message : error),
    generated_at: new Date().toISOString(),
  }
}

function failureTargetMetadata(target) {
  try {
    return describeInstallTarget(target)
  } catch {
    return { target }
  }
}

export function runInstallAcceptance({ target, version, archiveRoot, msiPath }) {
  if (target === 'x86_64-pc-windows-msvc') {
    return runWindowsMsiAcceptance({ target, version, msiPath })
  }
  const descriptor = describeInstallTarget(target)
  const archive = validateArchiveRoot(archiveRoot, version)
  const stateRoot = mkdtempSync(resolve(tmpdir(), 'cortana-install-acceptance-'))
  let evidence
  let failure
  let cleanup
  try {
    const environment = buildInstallEnvironment({ root: stateRoot })
    const install = runCommand(archive.installer, [], {
      cwd: archive.root,
      env: environment,
      root: stateRoot,
    })
    const state = inspectInstalledState(environment)
    if (!state.query_only_default)
      throw new Error('installed first run enabled synthesis by default')
    if (!state.no_source_or_schedule_side_effects)
      throw new Error('installed first run created a source or service schedule')
    const core = runInstalledCore(
      join(environment.CORTANA_INSTALL_PREFIX, 'bin', 'cortana'),
      version,
      environment,
      stateRoot
    )
    evidence = {
      schema_version: 1,
      status: 'passed',
      target: descriptor,
      version,
      installation_type: resolveAcceptanceInstallationType({
        published: 'published-release-installer',
        prospective: 'prospective-source-installer',
      }),
      cases: [
        'clean-install-first-run',
        'installed-version',
        'installed-web-assets',
        'explicit-connector-install',
        'query-only-default',
        'no-implicit-source-or-schedule-side-effects',
        'installed-core-offline-evaluation',
      ],
      install: install.evidence,
      first_run: state,
      core,
      paths: {
        binary: 'prefix/bin/cortana',
        web: 'prefix/share/cortana/web/index.html',
        connector: 'prefix/share/cortana/venv/bin/cortana-connectors',
        config: 'config/cortana/config.toml',
      },
      services: {
        status: 'not_exercised',
        reason: 'installer acceptance disables service installation in the isolated prefix',
      },
      scope: {
        provider_network: 'not_requested',
        installer_dependency_network: 'may_be_used',
        external_services: 'not_started',
        state: 'isolated-temporary-directory',
      },
      limitations: [
        'This lane verifies the release archive installer and installed core in an isolated prefix.',
        'OAuth, source authorization, services, tray, native dialogs, updater lifecycle, recovery UI, uninstall, accessibility, and OS trust require separate acceptance evidence.',
      ],
      reviewer: 'automated acceptance lane',
      generated_at: new Date().toISOString(),
    }
  } catch (error) {
    failure = error
  } finally {
    try {
      rmSync(stateRoot, { recursive: true, force: true })
      cleanup = { status: 'passed', state_root_removed: !existsSync(stateRoot) }
      if (!cleanup.state_root_removed) throw new Error('release installer state was not removed')
    } catch (error) {
      if (!failure) failure = error
    }
  }
  if (failure) throw failure
  if (!evidence || !cleanup) throw new Error('release installer did not produce complete evidence')
  evidence.cleanup = cleanup
  return evidence
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
  const values = parseArguments(args)
  const target = values.target || process.env.CORTANA_DESKTOP_TARGET
  const version = values.version || process.env.CORTANA_RELEASE_VERSION
  const archiveRoot = values['archive-dir'] || process.env.CORTANA_RELEASE_ARCHIVE_ROOT
  const msiPath = values.msi || process.env.CORTANA_WINDOWS_MSI
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output =
    values.output || resolve(evidenceDirectory, `${target || 'unknown'}-installer.json`)
  if (!target || !version || (target === 'x86_64-pc-windows-msvc' ? !msiPath : !archiveRoot)) {
    throw new Error(
      'usage: desktop-install-acceptance.mjs --target TARGET --version VERSION (--archive-dir DIR | --msi FILE) [--evidence-dir DIR] [--output FILE]'
    )
  }
  mkdirSync(evidenceDirectory, { recursive: true })
  const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
  const evidence = runInstallAcceptance({ target, version, archiveRoot, msiPath })
  writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 })
  console.log(`desktop install acceptance passed: ${evidencePath}`)
  return evidence
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  let values = {}
  let argumentError
  try {
    values = parseArguments(process.argv.slice(2))
  } catch (error) {
    argumentError = error
  }
  const target = values.target || process.env.CORTANA_DESKTOP_TARGET
  const version = values.version || process.env.CORTANA_RELEASE_VERSION
  const msiPath = values.msi || process.env.CORTANA_WINDOWS_MSI
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output =
    values.output || resolve(evidenceDirectory, `${target || 'unknown'}-installer.json`)
  try {
    if (argumentError) throw argumentError
    main(process.argv.slice(2))
  } catch (error) {
    const message = redactEvidence(error instanceof Error ? error.message : error)
    try {
      mkdirSync(evidenceDirectory, { recursive: true })
      const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
      writeFileSync(
        evidencePath,
        `${JSON.stringify(installFailureEvidence({ target, version, error: message }), null, 2)}\n`,
        { mode: 0o600 }
      )
      console.error(`${message}; failure evidence: ${evidencePath}`)
    } catch (evidenceError) {
      console.error(message)
      console.error(
        redactEvidence(evidenceError instanceof Error ? evidenceError.message : evidenceError)
      )
    }
    process.exitCode = 1
  }
}
