#!/usr/bin/env node

import { createHash } from 'node:crypto'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

import { resolveAcceptanceInstallationType } from './acceptance-provenance.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const MAX_OUTPUT_LENGTH = 1_000
const COMMAND_TIMEOUT_MS = 60_000
const TAURI_CONFIG_PATH = resolve(ROOT, 'apps/desktop/src-tauri/tauri.conf.json')
const UPDATER_ENDPOINT = 'https://github.com/adea-ai/cortana/releases/latest/download/latest.json'

const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({
    platform: 'macOS',
    architecture: 'arm64',
    artifacts: ['app', 'dmg', 'updater-signature', 'updater-manifest'],
  }),
  'x86_64-unknown-linux-gnu': Object.freeze({
    platform: 'Linux',
    architecture: 'x64',
    artifacts: ['appimage', 'deb', 'rpm', 'updater-signature', 'updater-manifest'],
  }),
  'x86_64-pc-windows-msvc': Object.freeze({
    platform: 'Windows',
    architecture: 'x64',
    artifacts: ['nsis', 'msi', 'updater-signature', 'updater-manifest'],
  }),
})

export function describeDesktopTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported desktop target: ${target}`)
  return { platform: descriptor.platform, architecture: descriptor.architecture, target }
}

export function requiredPackageArtifacts(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported desktop target: ${target}`)
  return [...descriptor.artifacts]
}

export function redactEvidence(value) {
  return String(value)
    .replace(
      /\b(password|passwd|token|secret|api[_-]?key|private[_-]?key)\s*=\s*[^\s,;]+/gi,
      (_, key) => `${key}=[REDACTED]`
    )
    .replace(
      /(["']?)(password|passwd|token|secret|api[_-]?key|private[_-]?key)\1\s*:\s*(["']?)([^,"'\s}]+)\3/gi,
      (_, quote, key, valueQuote) => `${quote}${key}${quote}:${valueQuote}[REDACTED]${valueQuote}`
    )
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .slice(0, MAX_OUTPUT_LENGTH)
}

export function validateEvidenceOutputPath(directory, outputPath) {
  const base = resolve(directory)
  const output = resolve(outputPath)
  const relativeOutput = relative(base, output)
  if (!relativeOutput || relativeOutput.startsWith('..') || relativeOutput.startsWith('/')) {
    throw new Error(`evidence output must stay inside ${base}`)
  }
  return output
}

function failureTargetMetadata(target) {
  try {
    return describeDesktopTarget(target)
  } catch {
    return { target }
  }
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'))
}

function projectVersions() {
  const rootVersion = readJson(resolve(ROOT, 'package.json')).version
  const connectorVersion = readFileSync(resolve(ROOT, 'pyproject.toml'), 'utf8').match(
    /^version\s*=\s*["']([^"']+)["']/m
  )?.[1]
  if (!connectorVersion) throw new Error('connector project version is missing from pyproject.toml')
  return {
    ...(rootVersion ? { root: rootVersion } : {}),
    web: readJson(resolve(ROOT, 'apps/web/package.json')).version,
    desktop: readJson(resolve(ROOT, 'apps/desktop/package.json')).version,
    tauri: readJson(resolve(ROOT, 'apps/desktop/src-tauri/tauri.conf.json')).version,
    connector: connectorVersion,
  }
}

function sourceVersionMatches(versions, version) {
  return Object.values(versions).every((candidate) => candidate === version)
}

function filesUnder(directory) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) files.push(...filesUnder(path))
    else if (entry.isFile()) files.push(entry.name)
  }
  return files
}

function artifactPatterns(target, version) {
  const patterns = {
    'aarch64-apple-darwin': [
      [`Cortana_${version}_aarch64.app.tar.gz`, 'app'],
      [`Cortana_${version}_aarch64.dmg`, 'dmg'],
      [`Cortana_${version}_aarch64.app.tar.gz.sig`, 'updater-signature'],
      ['latest.json', 'updater-manifest'],
    ],
    'x86_64-unknown-linux-gnu': [
      [`Cortana_${version}_amd64.AppImage`, 'appimage'],
      [`Cortana_${version}_amd64.deb`, 'deb'],
      [`Cortana-${version}-1.x86_64.rpm`, 'rpm'],
      [`Cortana_${version}_amd64.AppImage.sig`, 'updater-signature'],
      [`Cortana_${version}_amd64.deb.sig`, 'updater-signature'],
      [`Cortana-${version}-1.x86_64.rpm.sig`, 'updater-signature'],
      ['latest.json', 'updater-manifest'],
    ],
    'x86_64-pc-windows-msvc': [
      [`Cortana_${version}_x64-setup.exe`, 'nsis'],
      [`Cortana_${version}_x64_en-US.msi`, 'msi'],
      [`Cortana_${version}_x64-setup.exe.sig`, 'updater-signature'],
      [`Cortana_${version}_x64_en-US.msi.sig`, 'updater-signature'],
      ['latest.json', 'updater-manifest'],
    ],
  }
  if (!patterns[target]) throw new Error(`unsupported desktop target: ${target}`)
  return patterns[target]
}

function verifyArtifacts(packageDirectory, target, version) {
  if (!existsSync(packageDirectory))
    throw new Error(`package directory does not exist: ${packageDirectory}`)
  const available = new Set(filesUnder(packageDirectory))
  const found = artifactPatterns(target, version).map(([name, kind]) => {
    if (!available.has(name)) throw new Error(`missing ${kind} artifact: ${name}`)
    return name
  })
  return found
}

const UPDATER_PLATFORM_FOR_TARGET = Object.freeze({
  'aarch64-apple-darwin': 'darwin-aarch64-app',
  'x86_64-unknown-linux-gnu': 'linux-x86_64-appimage',
  'x86_64-pc-windows-msvc': 'windows-x86_64-nsis',
})

export function verifyUpdaterManifest(packageDirectory, target, version) {
  const manifestPath = resolve(packageDirectory, 'latest.json')
  let manifest
  try {
    manifest = readJson(manifestPath)
  } catch {
    throw new Error('updater manifest is not valid JSON')
  }
  if (manifest?.version !== version) {
    throw new Error(`updater manifest version mismatch: expected ${version}`)
  }
  const platform = UPDATER_PLATFORM_FOR_TARGET[target]
  const entry = manifest?.platforms?.[platform]
  if (!platform || !entry || typeof entry !== 'object') {
    throw new Error(`updater manifest is missing the ${target} platform entry`)
  }
  if (typeof entry.signature !== 'string' || !entry.signature.trim()) {
    throw new Error(`updater manifest ${platform} signature is empty`)
  }
  const expectedArchive = artifactPatterns(target, version).find(
    ([, kind]) => kind === 'app' || kind === 'appimage' || kind === 'nsis'
  )?.[0]
  if (!expectedArchive) {
    throw new Error(`updater manifest ${platform} URL does not bind to the package archive`)
  }
  let updateUrl
  try {
    updateUrl = new URL(entry.url)
  } catch {
    throw new Error(`updater manifest ${platform} URL is not a valid HTTPS release URL`)
  }
  const expectedPath = `/adea-ai/cortana/releases/download/v${version}/${expectedArchive}`
  if (
    updateUrl.origin !== 'https://github.com' ||
    updateUrl.pathname !== expectedPath ||
    updateUrl.search ||
    updateUrl.hash
  ) {
    throw new Error(`updater manifest ${platform} URL is not the official release URL`)
  }
  return { status: 'passed', version, platform }
}

export function verifyUpdaterConfiguration(configPath = TAURI_CONFIG_PATH) {
  let config
  try {
    config = readJson(resolve(configPath))
  } catch {
    throw new Error('Tauri updater configuration is not valid JSON')
  }
  const updater = config?.plugins?.updater
  if (
    !Array.isArray(updater?.endpoints) ||
    updater.endpoints.length !== 1 ||
    updater.endpoints[0] !== UPDATER_ENDPOINT
  ) {
    throw new Error('Tauri updater configuration does not use the official HTTPS feed')
  }
  if (typeof updater.pubkey !== 'string' || !updater.pubkey.trim()) {
    throw new Error('Tauri updater configuration is missing the verification public key')
  }
  if (updater.dialog !== false) {
    throw new Error('Tauri updater automatic dialog must remain disabled')
  }
  if (config?.bundle?.createUpdaterArtifacts !== true) {
    throw new Error('Tauri updater artifacts are not enabled')
  }
  return {
    status: 'passed',
    endpoint: UPDATER_ENDPOINT,
    signed_updates_required: true,
    automatic_dialog: false,
  }
}

export function artifactChecksums(directory, artifacts) {
  const packageDirectory = resolve(directory)
  return Object.fromEntries(
    artifacts.map((artifact) => [
      artifact,
      createHash('sha256')
        .update(readFileSync(resolve(packageDirectory, artifact)))
        .digest('hex'),
    ])
  )
}

function runCore(core, version) {
  const temporary = mkdtempSync(resolve(tmpdir(), 'cortana-package-acceptance-'))
  try {
    const versionResult = spawnSync(core, ['--version'], {
      encoding: 'utf8',
      timeout: COMMAND_TIMEOUT_MS,
      windowsHide: true,
    })
    if (versionResult.error || versionResult.status !== 0) {
      throw new Error(
        `packaged core --version failed: ${redactEvidence(versionResult.stderr || versionResult.error)}`
      )
    }
    const reportedVersion = redactEvidence(versionResult.stdout).trim()
    if (reportedVersion !== `cortana ${version}`) {
      throw new Error(
        `packaged core version mismatch: expected cortana ${version}, got ${reportedVersion}`
      )
    }

    const config = resolve(temporary, 'config.toml')
    writeFileSync(config, '[query]\n')
    const evaluation = spawnSync(core, ['--config', config, '--offline', 'eval'], {
      encoding: 'utf8',
      timeout: COMMAND_TIMEOUT_MS,
      windowsHide: true,
    })
    if (evaluation.error || evaluation.status !== 0) {
      throw new Error(
        `packaged core offline evaluation failed: ${redactEvidence(evaluation.stderr || evaluation.error)}`
      )
    }
    let report
    try {
      report = JSON.parse(evaluation.stdout)
    } catch (error) {
      throw new Error(`packaged core evaluation was not JSON: ${redactEvidence(error)}`)
    }
    if (report?.passed !== true) throw new Error('packaged core offline evaluation did not pass')
    return { reported_version: reportedVersion, offline_evaluation: 'passed' }
  } finally {
    rmSync(temporary, { recursive: true, force: true })
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

export function failureEvidence({ target, version, error }) {
  return {
    schema_version: 1,
    status: 'failed',
    ...(target ? { target: failureTargetMetadata(target) } : {}),
    ...(version ? { version } : {}),
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-release-assets',
      prospective: 'prospective-source-release-assets',
    }),
    error: redactEvidence(error instanceof Error ? error.message : error),
    generated_at: new Date().toISOString(),
  }
}

export function runAcceptance(options) {
  const target = options.target
  const version = options.version
  const descriptor = describeDesktopTarget(target)
  const versions = projectVersions()
  const sourceVersionMatch = sourceVersionMatches(versions, version)
  const allowSourceVersionDrift =
    options.allowSourceVersionDrift === true ||
    options.allowSourceVersionDrift === 'true' ||
    process.env.CORTANA_ALLOW_SOURCE_VERSION_DRIFT === 'true'
  if (!sourceVersionMatch && !allowSourceVersionDrift) {
    throw new Error(`project version mismatch: ${redactEvidence(JSON.stringify(versions))}`)
  }
  const artifacts = verifyArtifacts(resolve(options.packageDirectory), target, version)
  const packageDirectory = resolve(options.packageDirectory)
  const updaterConfiguration = verifyUpdaterConfiguration()
  const updaterManifest = verifyUpdaterManifest(packageDirectory, target, version)
  const core = resolve(options.core)
  if (!existsSync(core)) throw new Error(`packaged core does not exist: ${core}`)
  const coreEvidence = runCore(core, version)
  const generatedAt = new Date().toISOString()
  const cases = [
    'published-artifact-presence',
    'updater-configuration-binding',
    'updater-manifest-binding',
    'component-version-agreement',
    'packaged-core-version',
    'packaged-core-offline-evaluation',
  ]
  if (!sourceVersionMatch) cases.push('source-project-version-drift-recorded')
  return {
    schema_version: 1,
    status: 'passed',
    target: descriptor,
    version,
    installation_type: resolveAcceptanceInstallationType({
      published: 'published-release-assets',
      prospective: 'prospective-source-release-assets',
    }),
    artifacts,
    package_checksums: artifactChecksums(packageDirectory, artifacts),
    updater_manifest: updaterManifest,
    updater_configuration: updaterConfiguration,
    component_versions: {
      application: version,
      web: version,
      core: coreEvidence.reported_version.replace(/^cortana\s+/, ''),
      connector: version,
    },
    verifier_project_versions: versions,
    source_project_version_match: sourceVersionMatch,
    cases,
    core: coreEvidence,
    gui: 'not exercised by this headless lane; requires host acceptance',
    host_acceptance: {
      status: 'not_exercised',
      limitation:
        'GUI, native dialogs, services, updater lifecycle, and OS trust require host acceptance',
    },
    reviewer: 'automated CI',
    generated_at: generatedAt,
    reviewed_at: generatedAt,
  }
}

export function main(args = process.argv.slice(2)) {
  const values = parseArguments(args)
  const target = values.target || process.env.CORTANA_DESKTOP_TARGET
  const version = values.version || process.env.CORTANA_RELEASE_VERSION
  const packageDirectory = values['package-dir'] || process.env.CORTANA_PACKAGE_DIRECTORY
  const core = values.core || process.env.CORTANA_PACKAGED_CORE
  const allowSourceVersionDrift = values['allow-source-version-drift']
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output = values.output || resolve(evidenceDirectory, `${target || 'unknown'}.json`)
  if (!target || !version || !packageDirectory || !core) {
    throw new Error(
      'usage: desktop-package-acceptance.mjs --target TARGET --version VERSION --package-dir DIR --core PATH [--evidence-dir DIR] [--output FILE]'
    )
  }
  mkdirSync(evidenceDirectory, { recursive: true })
  const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
  const evidence = runAcceptance({
    target,
    version,
    packageDirectory,
    core,
    allowSourceVersionDrift,
  })
  writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(`desktop package acceptance passed: ${evidencePath}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const values = parseArguments(process.argv.slice(2))
  const target = values.target || process.env.CORTANA_DESKTOP_TARGET
  const version = values.version || process.env.CORTANA_RELEASE_VERSION
  const evidenceDirectory =
    values['evidence-dir'] ||
    process.env.CORTANA_EVIDENCE_DIRECTORY ||
    resolve(ROOT, 'artifacts/desktop-acceptance')
  const output = values.output || resolve(evidenceDirectory, `${target || 'unknown'}.json`)
  try {
    main(process.argv.slice(2))
  } catch (error) {
    const message = redactEvidence(error instanceof Error ? error.message : error)
    try {
      mkdirSync(evidenceDirectory, { recursive: true })
      const evidencePath = validateEvidenceOutputPath(evidenceDirectory, output)
      writeFileSync(
        evidencePath,
        `${JSON.stringify(failureEvidence({ target, version, error: message }), null, 2)}\n`
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
