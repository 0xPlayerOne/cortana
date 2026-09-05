#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  accessSync,
  constants,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

import { validateEvidenceOutputPath } from './desktop-package-acceptance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const COMMAND_TIMEOUT_MS = 60_000
const MAX_OUTPUT_LENGTH = 1_000

const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({ platform: 'macOS', architecture: 'arm64' }),
  'x86_64-unknown-linux-gnu': Object.freeze({ platform: 'Linux', architecture: 'x64' }),
  'x86_64-pc-windows-msvc': Object.freeze({ platform: 'Windows', architecture: 'x64' }),
})

export const AUTHORIZATION_PROVIDERS = Object.freeze(['google', 'github', 'discord', 'slack'])

export const AUTHORIZATION_CASES = Object.freeze([
  'unknown-source-fails-closed',
  'google-missing-token-destination-fails-closed',
  'github-missing-token-destination-fails-closed',
  'discord-missing-token-destination-fails-closed',
  'slack-missing-token-destination-fails-closed',
  'google-malformed-oauth-client-fails-closed',
  'github-malformed-oauth-client-fails-closed',
  'discord-malformed-oauth-client-fails-closed',
  'slack-malformed-oauth-client-fails-closed',
  'authorization-no-sync-side-effect',
])

const PROVIDER_COMMANDS = Object.freeze({
  google: 'authorize-google',
  github: 'authorize-github',
  discord: 'authorize-discord',
  slack: 'authorize-slack',
})

const MALFORMED_CLIENT_MARKERS = Object.freeze({
  google: 'OAuth client JSON must contain credentials',
  github: 'GitHub OAuth client must contain client_id',
  discord: 'Discord OAuth client JSON must contain client_id',
  slack: 'Slack OAuth client JSON must contain',
})

const MISSING_TOKEN_MARKERS = Object.freeze({
  google: 'requires a token file or token path environment variable',
  github: 'GitHub OAuth requires a token file destination',
  discord: 'requires a Discord RPC token file',
  slack: 'requires a token file for browser authorization',
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
])

export function describeAuthorizationTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported authorization target: ${target}`)
  return { ...descriptor, target }
}

export function redactAuthorizationOutput(value) {
  return String(value ?? '')
    .replace(
      /\b(password|passwd|token|secret|api[_-]?key|private[_-]?key)\s*=\s*[^\s,;]+/gi,
      (_, key) => `${key}=[REDACTED]`
    )
    .replace(
      /(?:\/(?:Users|private|home|tmp|var|runner|workspace|builds|opt\/runner)\/[^\s\n]+|[A-Za-z]:\\[^\s\n]+|\\\\[^\s\n]+)/g,
      '[PATH]'
    )
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .slice(0, MAX_OUTPUT_LENGTH)
}

export function buildAuthorizationPlan({ core, config, sources, missingSources = {} }) {
  const steps = [
    {
      name: 'unknown-source-fails-closed',
      provider: 'google',
      args: [core, '--config', config, PROVIDER_COMMANDS.google, 'missing-source'],
      expected_output: 'configured source missing-source was not found',
    },
  ]

  for (const provider of AUTHORIZATION_PROVIDERS) {
    steps.push({
      name: `${provider}-missing-token-destination-fails-closed`,
      provider,
      args: [core, '--config', config, PROVIDER_COMMANDS[provider], missingSources[provider]],
      expected_output: MISSING_TOKEN_MARKERS[provider],
    })
  }

  for (const provider of AUTHORIZATION_PROVIDERS) {
    steps.push({
      name: `${provider}-malformed-oauth-client-fails-closed`,
      provider,
      args: [core, '--config', config, PROVIDER_COMMANDS[provider], sources[provider]],
      expected_output: MALFORMED_CLIENT_MARKERS[provider],
    })
  }

  return steps
}

function buildIsolatedEnvironment(root, config) {
  const stateRoot = resolve(root)
  const configHome = resolve(stateRoot, 'config-home')
  const dataHome = resolve(stateRoot, 'data-home')
  const appData = resolve(stateRoot, 'appdata')
  const localAppData = resolve(stateRoot, 'localappdata')
  const temporary = resolve(stateRoot, 'tmp')
  const rosettaCache = resolve(stateRoot, '.cache', 'rosetta')
  for (const directory of [configHome, dataHome, appData, localAppData, temporary, rosettaCache]) {
    mkdirSync(directory, { recursive: true })
  }

  const environment = Object.fromEntries(
    SAFE_ENVIRONMENT_KEYS.flatMap((key) => (process.env[key] ? [[key, process.env[key]]] : []))
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

function tomlString(value) {
  return JSON.stringify(value)
}

function writeFixture(root) {
  const sources = Object.fromEntries(
    AUTHORIZATION_PROVIDERS.map((provider) => [provider, `${provider}-source`])
  )
  const missingSources = Object.fromEntries(
    AUTHORIZATION_PROVIDERS.map((provider) => [provider, `${provider}-missing-token`])
  )
  const dataDirectory = resolve(root, 'data')
  const config = resolve(root, 'config.toml')
  mkdirSync(dataDirectory, { recursive: true })

  const sourceKinds = {
    google: 'google-drive',
    github: 'github',
    discord: 'discord',
    slack: 'slack',
  }
  const sourceBlocks = AUTHORIZATION_PROVIDERS.flatMap((provider) => {
    const client = resolve(root, `${provider}-oauth-client.json`)
    const token = resolve(root, `${provider}-token.json`)
    const missingClient = resolve(root, `${provider}-missing-token-oauth-client.json`)
    writeFileSync(client, '{"client_id":', { mode: 0o600 })
    writeFileSync(missingClient, '{"client_id":', { mode: 0o600 })
    writeFileSync(token, '', { mode: 0o600 })
    const providerFields =
      provider === 'github'
        ? ['repositories = ["adea-ai/cortana"]']
        : provider === 'discord'
          ? ['channels = ["123456789012345678"]']
          : provider === 'slack'
            ? ['channels = ["C0123456789"]']
            : []
    const configuredSource = [
      '[[sources]]',
      `name = ${tomlString(sources[provider])}`,
      `kind = ${tomlString(sourceKinds[provider])}`,
      'project = "acceptance"',
      `token = ${tomlString(token)}`,
      `oauth_client = ${tomlString(client)}`,
      ...providerFields,
      '',
    ].join('\n')
    const missingTokenSource = [
      '[[sources]]',
      `name = ${tomlString(missingSources[provider])}`,
      `kind = ${tomlString(sourceKinds[provider])}`,
      'enabled = false',
      'project = "acceptance"',
      `oauth_client = ${tomlString(missingClient)}`,
      ...providerFields,
      '',
    ].join('\n')
    return [configuredSource, missingTokenSource]
  })

  writeFileSync(
    config,
    [
      `[runtime]`,
      `data_dir = ${tomlString(dataDirectory)}`,
      '',
      '[query]',
      '',
      ...sourceBlocks,
    ].join('\n'),
    { mode: 0o600 }
  )
  return { config, sources, missingSources }
}

const ROSETTA_CACHE_PATH = join('.cache', 'rosetta')

export function snapshotTree(root) {
  const entries = []
  const visit = (path, relativePath) => {
    if (
      relativePath === ROSETTA_CACHE_PATH ||
      relativePath.startsWith(`${ROSETTA_CACHE_PATH}${sep}`)
    ) {
      return
    }
    const metadata = lstatSync(path)
    if (metadata.isDirectory()) {
      entries.push(`${relativePath}/`)
      for (const entry of readdirSync(path).sort()) {
        const childRelativePath = join(relativePath, entry)
        if (
          childRelativePath === ROSETTA_CACHE_PATH ||
          childRelativePath.startsWith(`${ROSETTA_CACHE_PATH}${sep}`)
        ) {
          continue
        }
        visit(join(path, entry), childRelativePath)
      }
      return
    }
    if (metadata.isFile()) {
      const digest = createHash('sha256').update(readFileSync(path)).digest('hex')
      entries.push(`${relativePath}:${metadata.mode.toString(8)}:${digest}`)
      return
    }
    entries.push(`${relativePath}:special`)
  }
  visit(root, '.')
  return entries.sort().join('\n')
}

function requireCore(core) {
  if (!existsSync(core)) throw new Error(`packaged core does not exist: ${core}`)
  try {
    accessSync(core, constants.X_OK)
  } catch {
    throw new Error(`packaged core is not executable: ${core}`)
  }
}

function runExpectedFailure(step, environment) {
  const result = spawnSync(step.args[0], step.args.slice(1), {
    cwd: ROOT,
    env: environment,
    encoding: 'utf8',
    timeout: COMMAND_TIMEOUT_MS,
    windowsHide: true,
  })
  const output = `${result.stdout ?? ''}${result.stderr ?? ''}`
  const safeOutput = redactAuthorizationOutput(output)
  if (result.error) throw new Error(`${step.name} could not start: ${result.error.message}`)
  if (result.status === 0) throw new Error(`${step.name} unexpectedly succeeded`)
  if (!output.includes(step.expected_output)) {
    throw new Error(
      `${step.name} did not report its safe failure marker: ${step.expected_output}; output=${safeOutput}`
    )
  }
  return {
    name: step.name,
    provider: step.provider,
    status: 'passed',
    expected_failure: true,
    failure_marker: step.expected_output,
  }
}

export function summarizeAuthorizationEvidence({ target, version, steps, stateChanged }) {
  const failures = []
  const passedSteps = steps.filter((step) => step.status === 'passed')
  const seen = new Set(passedSteps.map((step) => step.name))
  for (const name of AUTHORIZATION_CASES) {
    const step = steps.find((candidate) => candidate.name === name)
    if (!step || step.status !== 'passed') {
      failures.push(`missing or failed authorization case: ${name}`)
    } else if (name !== 'authorization-no-sync-side-effect' && step.expected_failure !== true) {
      failures.push(`provider authorization case did not record an expected failure: ${name}`)
    }
  }
  if (stateChanged) {
    failures.push('authorization preflight changed isolated state or data')
  }

  return {
    schema_version: 1,
    status: failures.length === 0 ? 'passed' : 'failed',
    target: describeAuthorizationTarget(target),
    version,
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-package-source-authorization',
      prospective: 'prospective-source-source-authorization',
    }),
    cases: AUTHORIZATION_CASES.filter((name) => seen.has(name)),
    steps,
    state_changed: stateChanged,
    scope: {
      provider_network: 'not_requested',
      external_services: 'not_started',
      state: 'isolated-temporary-directory',
      source_data: 'not_read',
    },
    limitations: [
      'This lane exercises packaged authorization validation failures only; it never supplies credentials or completes provider consent.',
      'Provider-network use is not OS-level sandboxed by this runner; malformed input is rejected before the provider flow is opened.',
      'Successful Google, GitHub, Discord, and Slack authorization requires separate operator-controlled packaged evidence.',
    ],
    failures,
  }
}

export function runSourceAuthorizationAcceptance({ core, target, version, evidenceDirectory }) {
  requireCore(core)
  const descriptor = describeAuthorizationTarget(target)
  mkdirSync(evidenceDirectory, { recursive: true })
  const stateRoot = mkdtempSync(join(tmpdir(), 'cortana-source-authorization-'))
  let evidence
  try {
    const fixture = writeFixture(stateRoot)
    const environment = buildIsolatedEnvironment(stateRoot, fixture.config)
    const before = snapshotTree(stateRoot)
    const steps = []
    for (const step of buildAuthorizationPlan({
      core,
      config: fixture.config,
      sources: fixture.sources,
      missingSources: fixture.missingSources,
    })) {
      try {
        steps.push(runExpectedFailure(step, environment))
      } catch (error) {
        steps.push({
          name: step.name,
          provider: step.provider,
          status: 'failed',
          expected_failure: true,
          error: redactAuthorizationOutput(error instanceof Error ? error.message : error),
        })
      }
    }
    const after = snapshotTree(stateRoot)
    evidence = summarizeAuthorizationEvidence({
      target: descriptor.target,
      version,
      steps: [
        ...steps,
        {
          name: 'authorization-no-sync-side-effect',
          status: before === after ? 'passed' : 'failed',
        },
      ],
      stateChanged: before !== after,
    })
  } finally {
    rmSync(stateRoot, { recursive: true, force: true })
  }
  const output = resolve(evidenceDirectory, `desktop-source-authorization-${target}.json`)
  writeFileSync(output, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 })
  return { ...evidence, output }
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
        'usage: desktop-source-authorization-acceptance.mjs --target TARGET --version VERSION --core FILE --evidence-dir DIR'
      )
    }
    mkdirSync(evidenceDirectory, { recursive: true })
    validateEvidenceOutputPath(
      evidenceDirectory,
      resolve(evidenceDirectory, `desktop-source-authorization-${target}.json`)
    )
    const evidence = runSourceAuthorizationAcceptance({
      core: resolve(core),
      target,
      version,
      evidenceDirectory: resolve(evidenceDirectory),
    })
    console.log(`desktop source authorization acceptance ${evidence.status}: ${evidence.output}`)
    if (evidence.status !== 'passed') process.exitCode = 1
    return evidence
  } catch (error) {
    console.error(redactAuthorizationOutput(error instanceof Error ? error.message : error))
    process.exitCode = 1
    return null
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main()
