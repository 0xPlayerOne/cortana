#!/usr/bin/env node

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

import { redactEvidence, validateEvidenceOutputPath } from './desktop-package-acceptance.mjs'
import {
  buildIsolatedEnvironment,
  inspectFirstRunState,
  writeIsolatedConfig,
} from './desktop-host-launch.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const TARGET = 'aarch64-apple-darwin'
const DEFAULT_TIMEOUT_MS = 30_000
const PROBE_TIMEOUT_MS = 5_000
const POLL_INTERVAL_MS = 250
const MAX_OUTPUT_LENGTH = 1_000
const MACOS_APP_EXECUTABLE_SUFFIX = join('Contents', 'MacOS', 'cortana-desktop')

export const SWIFT_SCREEN_WINDOW_PROBE = `
import CoreGraphics
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let present = windows.contains {
  let owner = String(describing: $0[kCGWindowOwnerName as String] ?? "")
  let name = String(describing: $0[kCGWindowName as String] ?? "")
  let layer = String(describing: $0[kCGWindowLayer as String] ?? "")
  return owner == "Cortana" && name == "Cortana" && layer == "0"
}
print(present ? "screen_window=true" : "screen_window=false")
`

export const APPLESCRIPT_PROBE = `
tell application "System Events"
  if not (exists process "cortana-desktop") then return "error=process-not-found"
  tell process "cortana-desktop"
    set windowText to "false"
    if exists window "Cortana" then set windowText to "true"
    set trayPresent to false
    set runtimePresent to false
    set corpusPresent to false
    set ingestionPresent to false
    set sourceJobsPresent to false
    set showPresent to false
    set quitPresent to false
    repeat with menuBarRef in menu bars
      repeat with itemRef in menu bar items of menuBarRef
        try
          if (description of itemRef as text) is "status menu" then
            set trayPresent to true
            click itemRef
            delay 0.2
            repeat with menuRef in menus of itemRef
              repeat with menuItemRef in menu items of menuRef
                try
                  set menuName to name of menuItemRef as text
                  if menuName starts with "Runtime: " then set runtimePresent to true
                  if menuName starts with "Corpus: " then set corpusPresent to true
                  if menuName starts with "Ingestion: " then set ingestionPresent to true
                  if menuName starts with "Source jobs: " then set sourceJobsPresent to true
                  if menuName is "Show Cortana" then set showPresent to true
                  if menuName is "Quit Cortana Desktop" then set quitPresent to true
                end try
              end repeat
            end repeat
            key code 53
            exit repeat
          end if
        end try
      end repeat
      if trayPresent then exit repeat
    end repeat
    set trayText to "false"
    if trayPresent then set trayText to "true"
    set menuText to ""
    if runtimePresent then set menuText to menuText & "Runtime: status" & tab
    if corpusPresent then set menuText to menuText & "Corpus: status" & tab
    if ingestionPresent then set menuText to menuText & "Ingestion: status" & tab
    if sourceJobsPresent then set menuText to menuText & "Source jobs: status" & tab
    if showPresent then set menuText to menuText & "Show Cortana" & tab
    if quitPresent then set menuText to menuText & "Quit Cortana Desktop"
    return "window=" & windowText & linefeed & ¬
      "tray=" & trayText & linefeed & ¬
      "menu=" & menuText
  end tell
end tell
`

export const APPLESCRIPT_TRAY_CYCLE = `
tell application "System Events"
  if not (exists process "cortana-desktop") then return "error=process-not-found-before-cycle"
  tell process "cortana-desktop"
    set frontmost to true
    keystroke "w" using {command down}
  end tell
  delay 1
  set processRunning to exists process "cortana-desktop"
  tell process "cortana-desktop"
    set windowAfterClose to exists window "Cortana"
    set trayItem to missing value
    repeat with menuBarRef in menu bars
      repeat with itemRef in menu bar items of menuBarRef
        try
          if (description of itemRef as text) is "status menu" then
            set trayItem to itemRef
            exit repeat
          end if
        end try
      end repeat
      if trayItem is not missing value then exit repeat
    end repeat
    if trayItem is missing value then
      set processText to "false"
      if processRunning then set processText to "true"
      set closeText to "false"
      if windowAfterClose then set closeText to "true"
      return "process=" & processText & linefeed & ¬
        "window_after_close=" & closeText & linefeed & ¬
        "tray_reopen=false"
    end if
    click trayItem
    delay 0.2
    click menu item "Show Cortana" of menu 1 of trayItem
  end tell
  delay 1
  tell process "cortana-desktop"
    set windowAfterShow to exists window "Cortana"
  end tell
  set processText to "false"
  if processRunning then set processText to "true"
  set closeText to "false"
  if windowAfterClose then set closeText to "true"
  set showText to "false"
  if windowAfterShow then set showText to "true"
  return "process=" & processText & linefeed & ¬
    "window_after_close=" & closeText & linefeed & ¬
    "tray_reopen=" & showText
end tell
`

const REQUIRED_MENU_LABELS = Object.freeze({
  runtime: /^Runtime: /,
  corpus: /^Corpus: /,
  ingestion: /^Ingestion: /,
  sourceJobs: /^Source jobs: /,
  show: 'Show Cortana',
  quit: 'Quit Cortana Desktop',
})

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

function positiveInteger(raw, fallback) {
  if (!raw) return fallback
  const value = Number(raw)
  if (!Number.isSafeInteger(value) || value < 1 || value > 120_000) {
    throw new Error('timeout must be an integer between 1 and 120000 milliseconds')
  }
  return value
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

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds))
}

export function resolveMacosBundlePath(app) {
  const path = resolve(app)
  if (path.endsWith('.app')) return path
  if (path.endsWith(MACOS_APP_EXECUTABLE_SUFFIX)) {
    return resolve(path, '..', '..', '..')
  }
  return null
}

export function buildMacosLaunch(app) {
  const path = resolve(app)
  const bundle = resolveMacosBundlePath(path)
  if (bundle) {
    return { command: 'open', args: ['-n', bundle], mode: 'launch-services' }
  }
  return { command: path, args: [], mode: 'executable' }
}

function appExecutable(app) {
  const path = resolve(app)
  const bundle = resolveMacosBundlePath(path)
  if (bundle) {
    const bundledExecutable = join(bundle, MACOS_APP_EXECUTABLE_SUFFIX)
    if (existsSync(bundledExecutable)) return bundledExecutable
    throw new Error(`macOS Desktop application executable does not exist: ${bundledExecutable}`)
  }
  if (existsSync(path)) return path
  throw new Error(`macOS Desktop application does not exist: ${path}`)
}

function bundleVersion(app) {
  const bundle = resolveMacosBundlePath(app) ?? resolve(app)
  const info = join(bundle, 'Contents', 'Info.plist')
  if (!existsSync(info)) return null
  const result = spawnSync(
    'plutil',
    ['-extract', 'CFBundleShortVersionString', 'raw', '-o', '-', info],
    {
      encoding: 'utf8',
    }
  )
  if (result.status !== 0) throw new Error('read macOS application bundle version')
  const version = result.stdout.trim()
  if (!version) throw new Error('macOS application bundle version is empty')
  return version
}

function desktopProcessIds() {
  const result = spawnSync('pgrep', ['-x', 'cortana-desktop'], { encoding: 'utf8' })
  if (result.error)
    throw new Error(`inspect existing Cortana Desktop processes: ${result.error.message}`)
  if (result.status === 1) return []
  if (result.status !== 0) throw new Error('inspect existing Cortana Desktop processes')
  return result.stdout
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((value) => Number(value))
    .filter((value) => Number.isSafeInteger(value) && value > 0)
}

function assertNoExistingDesktopProcess() {
  if (desktopProcessIds().length > 0) {
    throw new Error('close existing Cortana Desktop processes before running native acceptance')
  }
}

async function waitForDesktopProcess(timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const pids = desktopProcessIds()
    if (pids.length === 1) return pids[0]
    await delay(POLL_INTERVAL_MS)
  }
  throw new Error(
    `Cortana Desktop process was not registered by LaunchServices within ${timeoutMs}ms`
  )
}

function processIsAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error?.code !== 'ESRCH'
  }
}

function runAppleScript(script, timeoutMs = PROBE_TIMEOUT_MS) {
  const child = spawn('osascript', ['-e', script], {
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let output = ''
  child.stdout?.on('data', (chunk) => {
    if (output.length < MAX_OUTPUT_LENGTH) {
      output += chunk.toString().slice(0, MAX_OUTPUT_LENGTH - output.length)
    }
  })
  const stderr = boundedOutput(child.stderr)
  return new Promise((resolveResult, rejectResult) => {
    let settled = false
    const timer = setTimeout(() => {
      if (settled) return
      settled = true
      child.kill('SIGTERM')
      rejectResult(
        new Error(`AppleScript probe timed out after ${timeoutMs}ms; stderr=${stderr()}`)
      )
    }, timeoutMs)
    const finish = (callback) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      callback()
    }
    child.once('error', (error) =>
      finish(() => rejectResult(new Error(`start AppleScript probe: ${error.message}`)))
    )
    child.once('exit', (code, signal) => {
      finish(() => {
        if (code !== 0) {
          rejectResult(
            new Error(
              `AppleScript probe failed (code=${code ?? 'null'}, signal=${signal ?? 'null'}): ${stderr()}`
            )
          )
        } else {
          // The probe is a private, fixed-format protocol. Preserve its
          // line/tab delimiters so the parser can distinguish fields; only
          // process/error output is passed through redactEvidence().
          resolveResult(output)
        }
      })
    })
  })
}

export function parseProbeOutput(output) {
  const values = {}
  for (const line of String(output).split(/\r?\n/)) {
    const separator = line.indexOf('=')
    if (separator < 1) continue
    values[line.slice(0, separator)] = line.slice(separator + 1)
  }
  return {
    windowPresent: values.window === 'true',
    trayPresent: values.tray === 'true',
    menuLabels: (values.menu ?? '').split('\t').filter(Boolean),
  }
}

export function parseScreenWindowProbeOutput(output) {
  const line = String(output)
    .split(/\r?\n/)
    .find((value) => value === 'screen_window=true' || value === 'screen_window=false')
  if (!line) return null
  return line === 'screen_window=true'
}

function inspectScreenWindow() {
  if (process.platform !== 'darwin') return null
  const result = spawnSync('swift', ['-e', SWIFT_SCREEN_WINDOW_PROBE], {
    encoding: 'utf8',
    timeout: 10_000,
  })
  if (result.error || result.status !== 0) return null
  return parseScreenWindowProbeOutput(result.stdout)
}

export function parseTrayCycleOutput(output) {
  const values = {}
  for (const line of String(output).split(/\r?\n/)) {
    const separator = line.indexOf('=')
    if (separator < 1) continue
    values[line.slice(0, separator)] = line.slice(separator + 1)
  }
  return {
    processRunning: values.process === 'true',
    windowAfterClose: values.window_after_close === 'true',
    trayReopen: values.tray_reopen === 'true',
  }
}

export function summarizeTrayMenu(menuLabels) {
  const labels = Array.isArray(menuLabels) ? menuLabels : []
  return {
    runtimeStatus: labels.some((label) => REQUIRED_MENU_LABELS.runtime.test(label)),
    corpusStatus: labels.some((label) => REQUIRED_MENU_LABELS.corpus.test(label)),
    ingestionStatus: labels.some((label) => REQUIRED_MENU_LABELS.ingestion.test(label)),
    sourceJobsStatus: labels.some((label) => REQUIRED_MENU_LABELS.sourceJobs.test(label)),
    show: labels.includes(REQUIRED_MENU_LABELS.show),
    quit: labels.includes(REQUIRED_MENU_LABELS.quit),
  }
}

export function buildLifecycleEvidence({ version, app, startupMs, firstRun, probe, cycle }) {
  const tray = summarizeTrayMenu(probe.menuLabels)
  const passed =
    version &&
    app &&
    Number.isSafeInteger(startupMs) &&
    startupMs >= 0 &&
    firstRun?.no_implicit_connector_install === true &&
    firstRun?.query_only_default === true &&
    firstRun?.no_implicit_side_effects === true &&
    probe.windowPresent &&
    probe.trayPresent &&
    Object.values(tray).every(Boolean) &&
    cycle.processRunning &&
    !cycle.windowAfterClose &&
    cycle.trayReopen
  return {
    schema_version: 1,
    status: passed ? 'passed' : 'failed',
    target: { target: TARGET, platform: 'macOS', architecture: 'arm64' },
    version,
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-package-macos-native-lifecycle',
      prospective: 'prospective-source-macos-native-lifecycle',
    }),
    application: basename(app),
    host: {
      status: passed ? 'passed' : 'failed',
      startup_ms: startupMs,
      isolated_state: true,
    },
    first_run: firstRun,
    cases: [
      'isolated-user-state',
      'packaged-process-startup',
      'native-tray-status',
      'close-to-tray',
      'tray-reopen',
    ],
    tray,
    window: {
      initial: probe.windowPresent ? 'present' : 'absent',
      after_close: cycle.windowAfterClose ? 'present' : 'hidden',
      after_show: cycle.trayReopen ? 'present' : 'absent',
    },
    scope: {
      state: 'isolated-temporary-directory',
      provider_network: 'not_requested',
      external_services: 'not_started',
    },
    limitations: [
      'does not exercise renderer controls, native file dialogs, OAuth, OS service installation, autostart, updater lifecycle, recovery UI, screen readers, or OS trust',
      'supplemental native lifecycle evidence; not a substitute for the three-target private Desktop acceptance record',
    ],
  }
}

async function terminateProcess(child, applicationPid = child?.pid) {
  if (applicationPid && applicationPid !== child?.pid) {
    try {
      process.kill(applicationPid, 'SIGTERM')
    } catch (error) {
      if (error?.code !== 'ESRCH') throw error
    }
    await delay(500)
    if (processIsAlive(applicationPid)) {
      process.kill(applicationPid, 'SIGKILL')
    }
  }
  if (!child || child.exitCode !== null || child.signalCode !== null || !child.pid) return
  child.kill('SIGTERM')
  await delay(500)
  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
  }
}

async function waitForReady(timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = 'process not exposed to macOS Accessibility'
  while (Date.now() < deadline) {
    try {
      const output = await runAppleScript(APPLESCRIPT_PROBE)
      const probe = parseProbeOutput(output)
      if (probe.windowPresent && probe.trayPresent) return probe
      lastError = `Cortana window or status item is not ready (window=${probe.windowPresent}, tray=${probe.trayPresent}, menu=${JSON.stringify(summarizeTrayMenu(probe.menuLabels))})`
    } catch (error) {
      lastError = redactEvidence(error instanceof Error ? error.message : error)
    }
    await delay(POLL_INTERVAL_MS)
  }
  throw new Error(`${lastError} within ${timeoutMs}ms`)
}

export async function runMacosLifecycleAcceptance({
  version,
  app,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  if (process.platform !== 'darwin')
    throw new Error('macOS native lifecycle acceptance requires macOS')
  if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) throw new Error('version must be plain semver')
  assertNoExistingDesktopProcess()
  const application = appExecutable(app)
  const packagedVersion = bundleVersion(app)
  if (packagedVersion && packagedVersion !== version) {
    throw new Error(`application bundle version ${packagedVersion} does not match ${version}`)
  }
  const stateRoot = mkdtempSync(join(tmpdir(), 'cortana-macos-lifecycle-'))
  const configPath = writeIsolatedConfig(stateRoot)
  const environment = buildIsolatedEnvironment({ root: stateRoot, configPath })
  const launch = buildMacosLaunch(application)
  const child = spawn(launch.command, launch.args, {
    cwd: ROOT,
    env: environment,
    // Keep the launcher in the current login session. LaunchServices owns the
    // actual GUI process when a packaged .app is supplied, which is required
    // for macOS Accessibility to expose its windows.
    detached: false,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const stdout = boundedOutput(child.stdout)
  const stderr = boundedOutput(child.stderr)
  const startedAt = Date.now()
  let applicationPid = child.pid
  try {
    if (launch.mode === 'launch-services') {
      applicationPid = await waitForDesktopProcess(timeoutMs)
    }
    const probe = await waitForReady(timeoutMs)
    const startupMs = Date.now() - startedAt
    const cycle = parseTrayCycleOutput(await runAppleScript(APPLESCRIPT_TRAY_CYCLE, timeoutMs))
    const firstRun = inspectFirstRunState(stateRoot, configPath)
    const evidence = buildLifecycleEvidence({
      version,
      app: application,
      startupMs,
      firstRun,
      probe,
      cycle,
    })
    if (evidence.status !== 'passed') {
      evidence.error = 'native lifecycle assertions failed'
    }
    return evidence
  } catch (error) {
    const screenWindowPresent = inspectScreenWindow()
    return {
      schema_version: 1,
      status: 'failed',
      target: { target: TARGET, platform: 'macOS', architecture: 'arm64' },
      version,
      installation_type: resolveAcceptanceInstallationType({
        published: 'published-package-macos-native-lifecycle',
        prospective: 'prospective-source-macos-native-lifecycle',
      }),
      application: basename(application),
      error: redactEvidence(error instanceof Error ? error.message : error),
      diagnostics: {
        accessibility_window_present: false,
        screen_window_present: screenWindowPresent,
      },
      stdout: stdout(),
      stderr: stderr(),
      limitations: [
        'native lifecycle evidence could not be collected; do not interpret this as renderer or package acceptance',
      ],
    }
  } finally {
    await terminateProcess(child, applicationPid)
    rmSync(stateRoot, { recursive: true, force: true })
  }
}

export async function main(args = process.argv.slice(2)) {
  const values = parseArguments(args)
  const version = values.version
  const app = values.app
  if (!version || !app) {
    throw new Error(
      'usage: desktop-macos-lifecycle-acceptance.mjs --version VERSION --app PATH [--evidence-dir DIR] [--output FILE] [--timeout-ms N]'
    )
  }
  const evidenceDirectory = values['evidence-dir'] || resolve(ROOT, 'artifacts/desktop-acceptance')
  const output = values.output || resolve(evidenceDirectory, 'macos-native-lifecycle.json')
  mkdirSync(evidenceDirectory, { recursive: true })
  const outputPath = validateEvidenceOutputPath(evidenceDirectory, output)
  const evidence = await runMacosLifecycleAcceptance({
    version,
    app,
    timeoutMs: positiveInteger(values['timeout-ms'], DEFAULT_TIMEOUT_MS),
  })
  writeFileSync(outputPath, `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(`macOS native lifecycle ${evidence.status}: ${outputPath}`)
  if (evidence.status !== 'passed') process.exitCode = 1
  return evidence
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main()
  } catch (error) {
    console.error(redactEvidence(error instanceof Error ? error.message : error))
    process.exitCode = 1
  }
}
