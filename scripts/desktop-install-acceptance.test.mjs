import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { expect, test } from 'bun:test'

import {
  buildInstallEnvironment,
  buildWindowsMsiInstallArguments,
  buildWindowsMsiUninstallArguments,
  describeInstallTarget,
  installFailureEvidence,
  runInstallAcceptance,
  validateArchiveRoot,
} from './desktop-install-acceptance.mjs'

test('installer target metadata is limited to supported archive lanes', () => {
  expect(describeInstallTarget('aarch64-apple-darwin')).toEqual({
    platform: 'macOS',
    architecture: 'arm64',
    target: 'aarch64-apple-darwin',
  })
  expect(describeInstallTarget('x86_64-unknown-linux-gnu')).toEqual({
    platform: 'Linux',
    architecture: 'x64',
    target: 'x86_64-unknown-linux-gnu',
  })
  expect(describeInstallTarget('x86_64-pc-windows-msvc')).toEqual({
    platform: 'Windows',
    architecture: 'x64',
    target: 'x86_64-pc-windows-msvc',
  })
})

test('installer failure evidence retains the supported target descriptor', () => {
  expect(
    installFailureEvidence({
      target: 'x86_64-pc-windows-msvc',
      version: '0.56.3',
      error: 'installer failed',
    })
  ).toMatchObject({
    target: {
      target: 'x86_64-pc-windows-msvc',
      platform: 'Windows',
      architecture: 'x64',
    },
    installation_type: 'published-release-installer',
  })
})

test('Windows MSI acceptance uses a silent, non-restarting install and uninstall plan', () => {
  expect(
    buildWindowsMsiInstallArguments({
      msiPath: '/runner/temp/Cortana_0.56.3_x64_en-US.msi',
      installDirectory: '/runner/temp/isolated-install',
      logPath: '/runner/temp/msiexec-install.log',
    })
  ).toEqual([
    '/i',
    '/runner/temp/Cortana_0.56.3_x64_en-US.msi',
    '/quiet',
    '/norestart',
    'INSTALLDIR=/runner/temp/isolated-install',
    '/l*v',
    '/runner/temp/msiexec-install.log',
  ])
  expect(
    buildWindowsMsiUninstallArguments({
      msiPath: '/runner/temp/Cortana_0.56.3_x64_en-US.msi',
      logPath: '/runner/temp/msiexec-uninstall.log',
    })
  ).toEqual([
    '/x',
    '/runner/temp/Cortana_0.56.3_x64_en-US.msi',
    '/quiet',
    '/norestart',
    '/l*v',
    '/runner/temp/msiexec-uninstall.log',
  ])
})

test('Windows MSI acceptance refuses to claim a Windows install on another host', () => {
  if (process.platform !== 'win32') {
    expect(() =>
      runInstallAcceptance({
        target: 'x86_64-pc-windows-msvc',
        version: '0.56.3',
        msiPath: '/runner/temp/Cortana_0.56.3_x64_en-US.msi',
      })
    ).toThrow('Windows MSI acceptance must run on Windows')
  }
})

test('installer environment isolates user state and disables service side effects', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-install-environment-test-'))
  try {
    const environment = buildInstallEnvironment({ root, baseEnvironment: { PATH: '/safe/bin' } })
    expect(environment).toMatchObject({
      HOME: root,
      USERPROFILE: root,
      PATH: '/safe/bin',
      TMPDIR: join(root, 'tmp'),
      TEMP: join(root, 'tmp'),
      TMP: join(root, 'tmp'),
      APPDATA: join(root, 'appdata'),
      LOCALAPPDATA: join(root, 'localappdata'),
      CORTANA_INSTALL_PREFIX: join(root, 'prefix'),
      CORTANA_INSTALL_SERVICE: '0',
      CORTANA_ENABLE_SYNC_SERVICE: '0',
      CORTANA_INSTALL_AGENT_INTEGRATIONS: '0',
    })
    expect(environment.CORTANA_CONFIG).toBe(join(root, 'config', 'cortana', 'config.toml'))
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('archive validation requires the release installer payload', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-install-archive-test-'))
  try {
    mkdirSync(join(root, 'bin'))
    mkdirSync(join(root, 'dist'))
    mkdirSync(join(root, 'share', 'cortana', 'web'), { recursive: true })
    writeFileSync(join(root, 'bin', 'cortana'), '')
    writeFileSync(join(root, 'dist', 'cortana_brain-0.56.3-py3-none-any.whl'), '')
    writeFileSync(join(root, 'share', 'cortana', 'web', 'index.html'), '')
    expect(() => validateArchiveRoot(root)).toThrow('release installer is missing')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('archive validation rejects a connector wheel version drift', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-install-version-test-'))
  try {
    mkdirSync(join(root, 'bin'))
    mkdirSync(join(root, 'dist'))
    mkdirSync(join(root, 'share', 'cortana', 'web'), { recursive: true })
    writeFileSync(join(root, 'install.sh'), '#!/bin/sh\n')
    writeFileSync(join(root, 'bin', 'cortana'), '')
    writeFileSync(join(root, 'dist', 'cortana_brain-0.55.0-py3-none-any.whl'), '')
    writeFileSync(join(root, 'share', 'cortana', 'web', 'index.html'), '')
    chmodSync(join(root, 'install.sh'), 0o755)
    chmodSync(join(root, 'bin', 'cortana'), 0o755)
    expect(() => validateArchiveRoot(root, '0.56.3')).toThrow(
      'release connector wheel version mismatch'
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('installer acceptance records a passing isolated release install', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-install-acceptance-test-'))
  const archive = join(root, 'archive')
  const installer = join(archive, 'install.sh')
  try {
    mkdirSync(join(archive, 'bin'), { recursive: true })
    mkdirSync(join(archive, 'dist'), { recursive: true })
    mkdirSync(join(archive, 'share', 'cortana', 'web'), { recursive: true })
    writeFileSync(join(archive, 'bin', 'cortana'), '')
    chmodSync(join(archive, 'bin', 'cortana'), 0o755)
    writeFileSync(join(archive, 'dist', 'cortana_brain-0.56.3-py3-none-any.whl'), '')
    writeFileSync(join(archive, 'share', 'cortana', 'web', 'index.html'), '<html></html>')
    const fakeCore = join(archive, 'fake-core')
    writeFileSync(
      fakeCore,
      [
        '#!/bin/sh',
        'if [ "${1:-}" = "--version" ]; then printf "%s\\n" "cortana 0.56.3"; else printf "%s\\n" "offline-evaluation" >&2; printf "%s\\n" "{\\"passed\\":true}"; fi',
        '',
      ].join('\n')
    )
    chmodSync(fakeCore, 0o755)
    const installerScript = [
      '#!/bin/sh',
      'set -eu',
      'archive_root="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"',
      'mkdir -p "$CORTANA_INSTALL_PREFIX/bin" "$CORTANA_INSTALL_PREFIX/share/cortana/web" "$CORTANA_INSTALL_PREFIX/share/cortana/venv/bin" "$(dirname "$CORTANA_CONFIG")"',
      'cp "$archive_root/fake-core" "$CORTANA_INSTALL_PREFIX/bin/cortana"',
      'chmod 755 "$CORTANA_INSTALL_PREFIX/bin/cortana"',
      ': > "$CORTANA_INSTALL_PREFIX/share/cortana/web/index.html"',
      ': > "$CORTANA_INSTALL_PREFIX/share/cortana/venv/bin/cortana-connectors"',
      'chmod 755 "$CORTANA_INSTALL_PREFIX/share/cortana/venv/bin/cortana-connectors"',
      'printf "%s\\n" "[query]" "synthesis_enabled = false" "" "[connectors]" "command = [\\"cortana-connectors\\"]" > "$CORTANA_CONFIG"',
    ].join('\n')
    writeFileSync(installer, `${installerScript}\n`)
    chmodSync(installer, 0o755)

    const report = runInstallAcceptance({
      target: 'aarch64-apple-darwin',
      version: '0.56.3',
      archiveRoot: archive,
    })

    expect(report).toMatchObject({
      status: 'passed',
      version: '0.56.3',
      installation_type: 'published-release-installer',
      install: { status: 'passed' },
      cleanup: { status: 'passed', state_root_removed: true },
      first_run: {
        query_only_default: true,
        no_source_or_schedule_side_effects: true,
      },
      core: { reported_version: 'cortana 0.56.3', offline_evaluation: 'passed' },
    })
    expect(report.cases).toContain('clean-install-first-run')
    expect(report.paths.config).toBe('config/cortana/config.toml')
    expect(JSON.stringify(report)).not.toContain(root)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
