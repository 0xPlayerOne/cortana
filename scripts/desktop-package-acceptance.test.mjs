import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

import { expect, test } from 'bun:test'

import {
  artifactChecksums,
  describeDesktopTarget,
  failureEvidence,
  redactEvidence,
  requiredPackageArtifacts,
  runAcceptance,
  validateEvidenceOutputPath,
  verifyUpdaterConfiguration,
  verifyUpdaterManifest,
} from './desktop-package-acceptance.mjs'

test('acceptance target metadata is limited to the supported release lanes', () => {
  expect(describeDesktopTarget('aarch64-apple-darwin')).toEqual({
    platform: 'macOS',
    architecture: 'arm64',
    target: 'aarch64-apple-darwin',
  })
  expect(describeDesktopTarget('x86_64-unknown-linux-gnu')).toEqual({
    platform: 'Linux',
    architecture: 'x64',
    target: 'x86_64-unknown-linux-gnu',
  })
  expect(describeDesktopTarget('x86_64-pc-windows-msvc')).toEqual({
    platform: 'Windows',
    architecture: 'x64',
    target: 'x86_64-pc-windows-msvc',
  })
  expect(() => describeDesktopTarget('x86_64-unknown-freebsd')).toThrow(
    'unsupported desktop target'
  )
})

test('required artifacts describe the target-specific package contract', () => {
  expect(requiredPackageArtifacts('aarch64-apple-darwin')).toEqual([
    'app',
    'dmg',
    'updater-signature',
    'updater-manifest',
  ])
  expect(requiredPackageArtifacts('x86_64-unknown-linux-gnu')).toEqual([
    'appimage',
    'deb',
    'rpm',
    'updater-signature',
    'updater-manifest',
  ])
  expect(requiredPackageArtifacts('x86_64-pc-windows-msvc')).toEqual([
    'nsis',
    'msi',
    'updater-signature',
    'updater-manifest',
  ])
})

test('evidence redaction removes secret-shaped values while retaining safe metadata', () => {
  expect(
    redactEvidence(
      'target=x86_64-unknown-linux-gnu password=hidden-token api_key=abc123 version=0.37.0'
    )
  ).toBe('target=x86_64-unknown-linux-gnu password=[REDACTED] api_key=[REDACTED] version=0.37.0')
})

test('evidence redaction also handles structured error output', () => {
  expect(redactEvidence('{"token":"hidden","safe":"visible"}')).toBe(
    '{"token":"[REDACTED]","safe":"visible"}'
  )
  expect(
    failureEvidence({ target: 'x86_64-pc-windows-msvc', version: '0.37.0', error: 'token=hidden' })
  ).toMatchObject({
    schema_version: 1,
    status: 'failed',
    target: {
      target: 'x86_64-pc-windows-msvc',
      platform: 'Windows',
      architecture: 'x64',
    },
    version: '0.37.0',
    error: 'token=[REDACTED]',
  })
})

test('evidence paths must stay inside the requested evidence directory', () => {
  expect(() => validateEvidenceOutputPath('/tmp/evidence', '/tmp/evidence/run.json')).not.toThrow()
  expect(() => validateEvidenceOutputPath('/tmp/evidence', '/tmp/evidence/../secret.json')).toThrow(
    'evidence output must stay inside'
  )
})

function writeMacUpdaterManifest(directory, version = '0.56.3') {
  writeFileSync(
    join(directory, 'latest.json'),
    JSON.stringify({
      version,
      platforms: {
        'darwin-aarch64-app': {
          url: `https://github.com/adea-ai/cortana/releases/download/v${version}/Cortana_${version}_aarch64.app.tar.gz`,
          signature: 'signed-update-fixture',
        },
      },
    })
  )
}

test('updater manifest binding requires the expected target archive', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-updater-manifest-test-'))
  try {
    writeMacUpdaterManifest(directory)
    expect(verifyUpdaterManifest(directory, 'aarch64-apple-darwin', '0.56.3')).toEqual({
      status: 'passed',
      version: '0.56.3',
      platform: 'darwin-aarch64-app',
    })

    writeFileSync(
      join(directory, 'latest.json'),
      JSON.stringify({
        version: '0.56.3',
        platforms: {
          'darwin-aarch64-app': {
            url: 'https://github.com/adea-ai/cortana/releases/download/v0.56.3/Cortana_0.56.3_aarch64.dmg',
            signature: 'signed-update-fixture',
          },
        },
      })
    )
    expect(() => verifyUpdaterManifest(directory, 'aarch64-apple-darwin', '0.56.3')).toThrow(
      'official release URL'
    )

    writeMacUpdaterManifest(directory)
    const manifest = JSON.parse(readFileSync(join(directory, 'latest.json'), 'utf8'))
    manifest.platforms['darwin-aarch64-app'].url =
      'https://example.test/Cortana_0.56.3_aarch64.app.tar.gz'
    writeFileSync(join(directory, 'latest.json'), JSON.stringify(manifest))
    expect(() => verifyUpdaterManifest(directory, 'aarch64-apple-darwin', '0.56.3')).toThrow(
      'official release URL'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('updater configuration binds the signed feed and keeps automatic installation disabled', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-updater-config-test-'))
  const path = join(directory, 'tauri.conf.json')
  const valid = {
    bundle: { createUpdaterArtifacts: true },
    plugins: {
      updater: {
        endpoints: ['https://github.com/adea-ai/cortana/releases/latest/download/latest.json'],
        pubkey: 'test-public-key',
        dialog: false,
      },
    },
  }
  try {
    writeFileSync(path, JSON.stringify(valid))
    expect(verifyUpdaterConfiguration(path)).toEqual({
      status: 'passed',
      endpoint: 'https://github.com/adea-ai/cortana/releases/latest/download/latest.json',
      signed_updates_required: true,
      automatic_dialog: false,
    })

    valid.plugins.updater.endpoints = ['https://updates.example.test/latest.json']
    writeFileSync(path, JSON.stringify(valid))
    expect(() => verifyUpdaterConfiguration(path)).toThrow('official HTTPS feed')

    valid.plugins.updater.endpoints = [
      'https://github.com/adea-ai/cortana/releases/latest/download/latest.json',
    ]
    valid.plugins.updater.dialog = true
    writeFileSync(path, JSON.stringify(valid))
    expect(() => verifyUpdaterConfiguration(path)).toThrow('automatic dialog')

    valid.plugins.updater.dialog = false
    valid.plugins.updater.pubkey = ''
    writeFileSync(path, JSON.stringify(valid))
    expect(() => verifyUpdaterConfiguration(path)).toThrow('verification public key')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('acceptance verifies a complete release fixture and packaged core evaluation', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-package-acceptance-test-'))
  const packageDirectory = join(directory, 'release-assets')
  const core = join(directory, 'cortana')
  mkdirSync(packageDirectory)
  const version = JSON.parse(
    readFileSync(resolve(import.meta.dir, '../apps/desktop/src-tauri/tauri.conf.json'), 'utf8')
  ).version
  const artifacts = [
    `Cortana_${version}_aarch64.app.tar.gz`,
    `Cortana_${version}_aarch64.dmg`,
    `Cortana_${version}_aarch64.app.tar.gz.sig`,
    'latest.json',
  ]
  try {
    for (const artifact of artifacts) writeFileSync(join(packageDirectory, artifact), 'fixture')
    writeMacUpdaterManifest(packageDirectory, version)
    writeFileSync(
      core,
      `#!/usr/bin/env node\nif (process.argv.includes('--version')) console.log('cortana ${version}'); else console.log(JSON.stringify({ passed: true }))\n`
    )
    chmodSync(core, 0o755)

    expect(
      runAcceptance({
        target: 'aarch64-apple-darwin',
        version,
        packageDirectory,
        core,
      })
    ).toMatchObject({
      status: 'passed',
      version,
      artifacts,
      core: { reported_version: `cortana ${version}`, offline_evaluation: 'passed' },
    })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('historical release acceptance records verifier source-version drift', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-package-acceptance-drift-'))
  const packageDirectory = join(directory, 'release-assets')
  const core = join(directory, 'cortana')
  mkdirSync(packageDirectory)
  const version = '0.37.0'
  const artifacts = [
    `Cortana_${version}_aarch64.app.tar.gz`,
    `Cortana_${version}_aarch64.dmg`,
    `Cortana_${version}_aarch64.app.tar.gz.sig`,
    'latest.json',
  ]
  try {
    for (const artifact of artifacts) writeFileSync(join(packageDirectory, artifact), 'fixture')
    writeMacUpdaterManifest(packageDirectory, version)
    writeFileSync(
      core,
      `#!/usr/bin/env node\nif (process.argv.includes('--version')) console.log('cortana ${version}'); else console.log(JSON.stringify({ passed: true }))\n`
    )
    chmodSync(core, 0o755)

    const evidence = runAcceptance({
      target: 'aarch64-apple-darwin',
      version,
      packageDirectory,
      core,
      allowSourceVersionDrift: true,
    })
    expect(evidence).toMatchObject({
      status: 'passed',
      version,
      component_versions: {
        application: version,
        web: version,
        core: version,
        connector: version,
      },
      source_project_version_match: false,
    })
    expect(evidence.cases).toContain('source-project-version-drift-recorded')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('acceptance evidence records the packaged contract required by the audit', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-package-acceptance-contract-'))
  const packageDirectory = join(directory, 'release-assets')
  const core = join(directory, 'cortana')
  mkdirSync(packageDirectory)
  const version = JSON.parse(
    readFileSync(resolve(import.meta.dir, '../apps/desktop/src-tauri/tauri.conf.json'), 'utf8')
  ).version
  const artifacts = [
    `Cortana_${version}_aarch64.app.tar.gz`,
    `Cortana_${version}_aarch64.dmg`,
    `Cortana_${version}_aarch64.app.tar.gz.sig`,
    'latest.json',
  ]
  try {
    for (const artifact of artifacts) writeFileSync(join(packageDirectory, artifact), 'fixture')
    writeMacUpdaterManifest(packageDirectory, version)
    writeFileSync(
      core,
      `#!/usr/bin/env node\nif (process.argv.includes('--version')) console.log('cortana ${version}'); else console.log(JSON.stringify({ passed: true }))\n`
    )
    chmodSync(core, 0o755)

    const evidence = runAcceptance({
      target: 'aarch64-apple-darwin',
      version,
      packageDirectory,
      core,
    })

    expect(evidence).toMatchObject({
      installation_type: 'published-release-assets',
      updater_manifest: {
        status: 'passed',
        version,
        platform: 'darwin-aarch64-app',
      },
      updater_configuration: {
        status: 'passed',
        signed_updates_required: true,
        automatic_dialog: false,
      },
      component_versions: {
        application: version,
        core: version,
        connector: version,
      },
      cases: [
        'published-artifact-presence',
        'updater-configuration-binding',
        'updater-manifest-binding',
        'component-version-agreement',
        'packaged-core-version',
        'packaged-core-offline-evaluation',
      ],
      host_acceptance: {
        status: 'not_exercised',
      },
    })
    expect(Object.keys(evidence.package_checksums).sort()).toEqual([...artifacts].sort())
    expect(Object.values(evidence.package_checksums)).toEqual(
      artifacts.map(() => expect.stringMatching(/^[a-f0-9]{64}$/))
    )
    expect(artifactChecksums(packageDirectory, artifacts)).toEqual(evidence.package_checksums)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
