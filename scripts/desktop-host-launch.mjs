#!/usr/bin/env node

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { basename, isAbsolute, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { redactEvidence, validateEvidenceOutputPath } from './desktop-package-acceptance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const MAX_OUTPUT_LENGTH = 1_000
const DEFAULT_STABLE_MS = 2_000
const DEFAULT_TIMEOUT_MS = 20_000
const MAX_TIMEOUT_MS = 120_000

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
  'SSL_CERT_DIR',
  'SSL_CERT_FILE',
  'SystemRoot',
  'WINDIR',
  'DISPLAY',
  'WAYLAND_DISPLAY',
  'GDK_BACKEND',
  'WEBKIT_DISABLE_DMABUF_RENDERER',
])

const CASES = Object.freeze(['isolated-user-state', 'packaged-process-startup'])

export function describeHostTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported host target: ${target}`)
  return { platform: descriptor.platform, architecture: descriptor.architecture, target }
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

  const environment = {}
  for (const key of SAFE_ENVIRONMENT_KEYS) {
    if (baseEnvironment[key]) environment[key] = baseEnvironment[key]
  }
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

export function writeIsolatedConfig(root) {
  const stateRoot = resolve(root)
  const dataDirectory = resolve(stateRoot, 'data')
  const configPath = resolve(stateRoot, 'config.toml')
  mkdirSync(dataDirectory, { recursive: true })
  writeFileSync(configPath, `[runtime]\ndata_dir = ${JSON.stringify(dataDirectory)}\n\n[query]\n`, {
    mode: 0o600,
  })
  return configPath
}

function boundedOutput(stream) {
  let output = ''
  stream?.on('data', (chunk) => {
    if (output.length < MAX_OUTPUT_LENGTH) {
      output += chunk.toString().slice(0, MAX_OUTPUT_LENGTH - output.length)
    }
  })
  return () => redactEvidence(output)
}

async function waitForExit(child, timeoutMs = 2_000) {
  if (child.exitCode !== null || child.signalCode !== null) return
  await new Promise((resolveWait) => {
    let finished = false
    const finish = () => {
      if (finished) return
      finished = true
      clearTimeout(timer)
      resolveWait()
    }
    const timer = setTimeout(finish, timeoutMs)
    child.once('exit', finish)
  })
}

async function terminateProcess(child) {
  if (child.exitCode !== null || child.signalCode !== null || !child.pid) return
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], {
      stdio: 'ignore',
      windowsHide: true,
    })
  } else {
    try {
      process.kill(-child.pid, 'SIGTERM')
    } catch (error) {
      if (error?.code !== 'ESRCH') child.kill('SIGTERM')
    }
  }
  await waitForExit(child)
  if (child.exitCode === null && child.signalCode === null) {
    if (process.platform === 'win32') {
      spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], {
        stdio: 'ignore',
        windowsHide: true,
      })
    } else {
      try {
        process.kill(-child.pid, 'SIGKILL')
      } catch (error) {
        if (error?.code !== 'ESRCH') child.kill('SIGKILL')
      }
    }
  }
}

export async function runHostLaunch({
  executable,
  args = [],
  env,
  cwd,
  stableMs = DEFAULT_STABLE_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  if (!existsSync(executable)) throw new Error(`host executable does not exist: ${executable}`)
  if (!env || typeof env !== 'object' || Array.isArray(env)) {
    throw new Error('host launch requires an explicit isolated environment')
  }
  const credentialKey = Object.keys(env).find((key) =>
    /(?:password|passwd|token|secret|api[_-]?key|private[_-]?key)/i.test(key)
  )
  if (credentialKey)
    throw new Error(`host launch environment contains credential-shaped key: ${credentialKey}`)
  if (!Array.isArray(args) || args.some((argument) => typeof argument !== 'string')) {
    throw new Error('host launch arguments must be an array of strings')
  }
  if (!Number.isSafeInteger(stableMs) || stableMs < 50 || stableMs > MAX_TIMEOUT_MS) {
    throw new Error(`stable startup window must be between 50 and ${MAX_TIMEOUT_MS} milliseconds`)
  }
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < stableMs || timeoutMs > MAX_TIMEOUT_MS) {
    throw new Error(
      `host launch timeout must be between stable window and ${MAX_TIMEOUT_MS} milliseconds`
    )
  }

  const child = spawn(executable, args, {
    cwd,
    env,
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  const stdout = boundedOutput(child.stdout)
  const stderr = boundedOutput(child.stderr)
  const startedAt = Date.now()

  return new Promise((resolveResult, rejectResult) => {
    let settled = false
    const settleFailure = async (error) => {
      if (settled) return
      settled = true
      clearTimeout(stableTimer)
      clearTimeout(timeoutTimer)
      await terminateProcess(child)
      rejectResult(new Error(`${error}; stderr=${stderr()}`))
    }
    const settleSuccess = async () => {
      if (settled) return
      settled = true
      clearTimeout(stableTimer)
      clearTimeout(timeoutTimer)
      await terminateProcess(child)
      resolveResult({
        status: 'passed',
        process: 'started-and-stopped',
        startup_ms: Date.now() - startedAt,
        stdout: stdout(),
        stderr: stderr(),
      })
    }
    const stableTimer = setTimeout(() => void settleSuccess(), stableMs)
    const timeoutTimer = setTimeout(
      () => void settleFailure(`host process did not reach stable startup within ${timeoutMs}ms`),
      timeoutMs
    )
    child.once(
      'error',
      (error) => void settleFailure(`host process failed to start: ${error.message}`)
    )
    child.once('exit', (code, signal) => {
      if (!settled) {
        void settleFailure(
          `host process exited before stable startup (code=${code ?? 'null'}, signal=${signal ?? 'null'})`
        )
      }
    })
  })
}

function findExecutable(command) {
  if (isAbsolute(command) || command.includes(sep)) {
    if (!existsSync(command)) throw new Error(`host launcher does not exist: ${command}`)
    return command
  }
  const resolver = process.platform === 'win32' ? 'where' : 'which'
  const result = spawnSync(resolver, [command], { encoding: 'utf8', windowsHide: true })
  const resolved = result.status === 0 ? result.stdout.trim().split(/\r?\n/)[0] : ''
  if (!resolved) throw new Error(`host launcher was not found: ${command}`)
  return resolved
}

function readProjectVersions() {
  const readVersion = (path) => JSON.parse(readFileSync(resolve(ROOT, path), 'utf8')).version
  const connector = readFileSync(resolve(ROOT, 'pyproject.toml'), 'utf8').match(
    /^version\s*=\s*["']([^"']+)["']/m
  )?.[1]
  if (!connector) throw new Error('connector project version is missing from pyproject.toml')
  return {
    application: readVersion('apps/desktop/src-tauri/tauri.conf.json'),
    web: readVersion('apps/web/package.json'),
    connector,
  }
}

function sourceVersionMatches(versions, version) {
  return Object.values(versions).every((candidate) => candidate === version)
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

function jsonStringArray(raw, name) {
  if (!raw) return []
  let parsed
  try {
    parsed = JSON.parse(raw)
  } catch (error) {
    throw new Error(`${name} must be a JSON string array: ${redactEvidence(error)}`)
  }
  if (
    !Array.isArray(parsed) ||
    parsed.length > 32 ||
    parsed.some((value) => typeof value !== 'string')
  ) {
    throw new Error(`${name} must be a JSON string array with at most 32 entries`)
  }
  return parsed
}

function positiveInteger(raw, fallback, name) {
  if (!raw) return fallback
  const value = Number(raw)
  if (!Number.isSafeInteger(value) || value < 1 || value > MAX_TIMEOUT_MS) {
    throw new Error(`${name} must be an integer between 1 and ${MAX_TIMEOUT_MS}`)
  }
  return value
}

export function hostFailureEvidence({ target, version, error }) {
  return {
    schema_version: 1,
    status: 'failed',
    ...(target ? { target: { target } } : {}),
    ...(version ? { version } : {}),
    error: redactEvidence(error instanceof Error ? error.message : error),
    generated_at: new Date().toISOString(),
  }
}

export async function runHostAcceptance(options) {
  const target = options.target
  const version = options.version
  const descriptor = describeHostTarget(target)
  const versions = readProjectVersions()
  const sourceVersionMatch = sourceVersionMatches(versions, version)
  const allowSourceVersionDrift =
    options.allowSourceVersionDrift === true ||
    options.allowSourceVersionDrift === 'true' ||
    process.env.CORTANA_ALLOW_SOURCE_VERSION_DRIFT === 'true'
  if (!sourceVersionMatch && !allowSourceVersionDrift) {
    throw new Error(`project version mismatch: ${redactEvidence(JSON.stringify(versions))}`)
  }
  const app = resolve(options.app)
  if (!existsSync(app)) throw new Error(`host application does not exist: ${app}`)
  const stateRoot = mkdtempSync(resolve(tmpdir(), 'cortana-host-acceptance-'))
  try {
    const configPath = writeIsolatedConfig(stateRoot)
    const environment = buildIsolatedEnvironment({ root: stateRoot, configPath })
    const appArgs = options.appArgs ?? []
    const launcher = options.launcher ? findExecutable(options.launcher) : app
    const launcherArgs = options.launcher
      ? [...(options.launcherArgs ?? []), app, ...appArgs]
      : appArgs
    const launch = await runHostLaunch({
      executable: launcher,
      args: launcherArgs,
      env: environment,
      stableMs: options.stableMs,
      timeoutMs: options.timeoutMs,
    })
    const cases = [...CASES]
    if (!sourceVersionMatch) cases.push('source-project-version-drift-recorded')
    return {
      schema_version: 1,
      status: 'passed',
      target: descriptor,
      version,
      installation_type: 'published-package-host-launch',
      component_versions: {
        application: version,
        web: version,
        connector: version,
      },
      verifier_project_versions: versions,
      source_project_version_match: sourceVersionMatch,
      cases,
      host: {
        ...launch,
        application: basename(app),
        launcher: basename(launcher),
        isolated_state: true,
      },
      gui: 'process startup only; interactive GUI controls require host automation',
      known_limitations: [
        'does not exercise interactive GUI controls, native dialogs, OAuth, services, updater lifecycle, or OS trust',
      ],
      reviewer: 'automated CI',
      generated_at: new Date().toISOString(),
    }
  } finally {
    rmSync(stateRoot, { recursive: true, force: true })
  }
}

export async function main(args = process.argv.slice(2)) {
  const values = parseArguments(args)
  const target = values.target || process.env.CORTANA_DESKTOP_TARGET
  const version = values.version || process.env.CORTANA_RELEASE_VERSION
  const app = values.app || process.env.CORTANA_PACKAGED_APP
  const allowSourceVersionDrift = values['allow-source-version-drift']
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output = values.output || resolve(evidenceDirectory, `${target || 'unknown'}-host.json`)
  if (!target || !version || !app) {
    throw new Error(
      'usage: desktop-host-launch.mjs --target TARGET --version VERSION --app PATH [--launcher COMMAND] [--launcher-args JSON] [--app-args JSON] [--evidence-dir DIR] [--output FILE]'
    )
  }
  mkdirSync(evidenceDirectory, { recursive: true })
  const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
  const evidence = await runHostAcceptance({
    target,
    version,
    app,
    allowSourceVersionDrift,
    launcher: values.launcher,
    launcherArgs: jsonStringArray(values['launcher-args'], '--launcher-args'),
    appArgs: jsonStringArray(values['app-args'], '--app-args'),
    stableMs: positiveInteger(values['stable-ms'], DEFAULT_STABLE_MS, '--stable-ms'),
    timeoutMs: positiveInteger(values['timeout-ms'], DEFAULT_TIMEOUT_MS, '--timeout-ms'),
  })
  writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(`desktop host launch passed: ${evidencePath}`)
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
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output = values.output || resolve(evidenceDirectory, `${target || 'unknown'}-host.json`)
  try {
    if (argumentError) throw argumentError
    await main(process.argv.slice(2))
  } catch (error) {
    const message = redactEvidence(error instanceof Error ? error.message : error)
    try {
      mkdirSync(evidenceDirectory, { recursive: true })
      const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
      writeFileSync(
        evidencePath,
        `${JSON.stringify(hostFailureEvidence({ target, version, error: message }), null, 2)}\n`
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
