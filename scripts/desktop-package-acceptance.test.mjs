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
  ])
  expect(requiredPackageArtifacts('x86_64-unknown-linux-gnu')).toEqual([
    'appimage',
    'deb',
    'rpm',
    'updater-signature',
  ])
  expect(requiredPackageArtifacts('x86_64-pc-windows-msvc')).toEqual([
    'nsis',
    'msi',
    'updater-signature',
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
    target: { target: 'x86_64-pc-windows-msvc' },
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
  ]
  try {
    for (const artifact of artifacts) writeFileSync(join(packageDirectory, artifact), 'fixture')
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
  ]
  try {
    for (const artifact of artifacts) writeFileSync(join(packageDirectory, artifact), 'fixture')
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
      component_versions: {
        application: version,
        core: version,
        connector: version,
      },
      cases: [
        'published-artifact-presence',
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
