import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { expect, test } from 'bun:test'

import {
  buildDesktopAcceptanceMatrix,
  REQUIRED_RENDERER_SCREENSHOTS,
} from './desktop-acceptance-matrix.mjs'
import { BROWSER_RESOURCE_THRESHOLDS } from './knowledge-accessibility-matrix.mjs'

const CONTROL_PLANE_CASES = [
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
]

const RENDERER_CASES = [
  'axe-wcag-2.2-aa',
  'keyboard-skip-link',
  'workspace-switching',
  'source-tree-scoping',
  'document-filtering',
  'keyboard-document-open',
  'document-canonical-content',
  'document-provenance',
  'document-source-link',
  'document-relations',
  'document-responsive-width-1440',
  'document-responsive-width-1024',
  'document-responsive-width-768',
  'document-zoom-200-percent-reflow-720',
  'document-responsive-width-390',
  'document-responsive-width-320',
  'keyboard-graph-open',
  'keyboard-node-selection',
  'graph-focus-history',
  'graph-document-navigation',
  'graph-collapse-restoration',
  'graph-filter-labels',
  'selection-live-region',
  'reduced-motion',
  'responsive-width-1440',
  'responsive-width-1024',
  'responsive-width-768',
  'zoom-200-percent-reflow-720',
  'responsive-width-390',
  'responsive-width-320',
  'browser-resource-budgets',
  'browser-console-clean',
]

const INSTALLER_CASES = [
  'clean-install-first-run',
  'installed-version',
  'installed-web-assets',
  'explicit-connector-install',
  'query-only-default',
  'no-implicit-source-or-schedule-side-effects',
  'installed-core-offline-evaluation',
]
const WINDOWS_INSTALLER_CASES = [
  'clean-install-msi',
  'installed-version',
  'installed-web-assets',
  'installed-core-sidecar',
  'installed-core-offline-evaluation',
  'clean-uninstall-msi',
]
const SOURCE_AUTHORIZATION_CASES = [
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
]
const SERVICE_STATUS_CASES = [
  'packaged-service-status',
  'complete-managed-service-set',
  'read-only-state-unchanged',
  'no-mutating-action-requested',
]

const TARGETS = [
  ['aarch64-apple-darwin', 'macOS', 'arm64'],
  ['x86_64-unknown-linux-gnu', 'Linux', 'x64'],
  ['x86_64-pc-windows-msvc', 'Windows', 'x64'],
]

function writeReleaseReports(directory, version = '0.56.3') {
  for (const [target, platform, architecture] of TARGETS) {
    const targetDirectory = join(directory, target)
    mkdirSync(targetDirectory, { recursive: true })
    const descriptor = { platform, architecture, target }
    const updaterPlatform = {
      'aarch64-apple-darwin': 'darwin-aarch64-app',
      'x86_64-unknown-linux-gnu': 'linux-x86_64-appimage',
      'x86_64-pc-windows-msvc': 'windows-x86_64-nsis',
    }[target]
    writeFileSync(
      join(targetDirectory, `${target}.json`),
      JSON.stringify({
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-release-assets',
        core: { offline_evaluation: 'passed' },
        host_acceptance: { status: 'not_exercised' },
        updater_configuration: { status: 'passed' },
        updater_manifest: { status: 'passed', version, platform: updaterPlatform },
      })
    )
    writeFileSync(
      join(targetDirectory, `${target}-host.json`),
      JSON.stringify({
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-package-host-launch',
        cases: [
          'isolated-user-state',
          'packaged-process-startup',
          'no-implicit-connector-install',
          'query-only-first-run',
          'no-implicit-side-effects',
        ],
        host: {
          status: 'passed',
          startup_ms: 2_000,
          isolated_state: true,
        },
        first_run: {
          no_implicit_connector_install: true,
          query_only_default: true,
          no_implicit_side_effects: true,
        },
      })
    )
    writeFileSync(
      join(targetDirectory, `${target}-control-plane.json`),
      JSON.stringify({
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-package-control-plane',
        preflight: 'passed',
        cases: CONTROL_PLANE_CASES,
        steps: CONTROL_PLANE_CASES.map((name) => ({ name, status: 'passed', duration_ms: 1 })),
        recovery: { invalid_restore_preserved_index: true },
        scope: {
          network: 'not-requested',
          network_enforcement: 'not-asserted',
          external_services: 'not_started',
          state: 'isolated-temporary-directory',
        },
      })
    )
    writeFileSync(
      join(targetDirectory, `${target}-service-status.json`),
      JSON.stringify({
        schema_version: 1,
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-package-service-status',
        cases: SERVICE_STATUS_CASES,
        steps: [{ name: 'packaged-service-status', status: 'passed', duration_ms: 1 }],
        service_manager: { supported: false, operation: 'status-only' },
        state_unchanged: true,
        scope: {
          provider_network: 'not-requested',
          external_services: 'not_started',
          state: 'isolated-temporary-directory',
          service_mutation: 'not-requested',
        },
      })
    )
    writeFileSync(
      join(targetDirectory, `${target}-renderer.json`),
      JSON.stringify({
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-package-renderer',
        revision: `v${version}`,
        fixture: 'provider-free-release-demo',
        server_mode: 'external',
        cases: RENDERER_CASES,
        axe: [
          { surface: 'knowledge', violations: 0, passes: 20 },
          { surface: 'document', violations: 0, passes: 20 },
          { surface: 'graph', violations: 0, passes: 20 },
        ],
        screenshots: REQUIRED_RENDERER_SCREENSHOTS.map(({ file }) => file),
        screenshot_matrix: REQUIRED_RENDERER_SCREENSHOTS,
        resource_metrics: {
          status: 'passed',
          sample_count: 3,
          latency_p50_ms: {
            navigation_ms: 100,
            document_open_ms: 100,
            graph_open_ms: 100,
            graph_selection_ms: 100,
          },
          latency_p95_ms: {
            navigation_ms: 100,
            document_open_ms: 100,
            graph_open_ms: 100,
            graph_selection_ms: 100,
          },
          peak: {
            request_count: 10,
            response_bytes: 1_000,
            dom_nodes: 100,
            visible_document_rows: 4,
            visible_graph_nodes: 12,
            js_heap_used_bytes: 1_000_000,
          },
          thresholds: BROWSER_RESOURCE_THRESHOLDS,
        },
      })
    )
    for (const screenshot of REQUIRED_RENDERER_SCREENSHOTS) {
      writeFileSync(join(targetDirectory, screenshot.file), 'fixture image')
    }
    writeFileSync(
      join(targetDirectory, `${target}-installer.json`),
      JSON.stringify({
        status: 'passed',
        version,
        target: descriptor,
        installation_type: 'published-release-installer',
        cases: target === 'x86_64-pc-windows-msvc' ? WINDOWS_INSTALLER_CASES : INSTALLER_CASES,
        install: { status: 'passed', exit_code: 0, duration_ms: 10 },
        cleanup: { status: 'passed', state_root_removed: true },
        ...(target === 'x86_64-pc-windows-msvc'
          ? {
              uninstall: { status: 'passed', exit_code: 0, duration_ms: 10 },
              first_run: { status: 'not_exercised' },
            }
          : {
              first_run: {
                query_only_default: true,
                no_source_or_schedule_side_effects: true,
                connector_installed_by_explicit_installer: true,
              },
            }),
        core: {
          reported_version: `cortana ${version}`,
          offline_evaluation: 'passed',
        },
        services: { status: 'not_exercised' },
        scope: {
          provider_network: 'not_requested',
          installer_dependency_network: 'may_be_used',
          external_services: 'not_started',
          state: 'isolated-temporary-directory',
        },
      })
    )
  }
}

function writePassingNativeLifecycleReport(directory, startupMs = 2_305) {
  const target = 'aarch64-apple-darwin'
  writeFileSync(
    join(directory, target, `${target}-native-lifecycle.json`),
    JSON.stringify({
      schema_version: 1,
      status: 'passed',
      target: { target, platform: 'macOS', architecture: 'arm64' },
      version: '0.56.3',
      installation_type: 'published-package-macos-native-lifecycle',
      host: { status: 'passed', startup_ms: startupMs, isolated_state: true },
      first_run: {
        no_implicit_connector_install: true,
        query_only_default: true,
        no_implicit_side_effects: true,
      },
      cases: [
        'isolated-user-state',
        'packaged-process-startup',
        'native-tray-status',
        'close-to-tray',
        'tray-reopen',
      ],
      tray: {
        runtimeStatus: true,
        corpusStatus: true,
        ingestionStatus: true,
        sourceJobsStatus: true,
        show: true,
        quit: true,
      },
      window: { initial: 'present', after_close: 'hidden', after_show: 'present' },
      scope: {
        state: 'isolated-temporary-directory',
        provider_network: 'not_requested',
        external_services: 'not_started',
      },
    })
  )
}

function writePassingSourceAuthorizationReports(directory) {
  for (const [target, platform, architecture] of TARGETS) {
    writeFileSync(
      join(directory, target, `${target}-source-authorization.json`),
      JSON.stringify({
        schema_version: 1,
        status: 'passed',
        target: { target, platform, architecture },
        version: '0.56.3',
        installation_type: 'published-package-source-authorization',
        cases: SOURCE_AUTHORIZATION_CASES,
        steps: SOURCE_AUTHORIZATION_CASES.map((name) => ({
          name,
          status: 'passed',
          ...(name === 'authorization-no-sync-side-effect' ? {} : { expected_failure: true }),
        })),
        state_changed: false,
        scope: {
          provider_network: 'not_requested',
          external_services: 'not_started',
          state: 'isolated-temporary-directory',
          source_data: 'not_read',
        },
      })
    )
  }
}

test('desktop acceptance matrix requires passing package and host reports for every target', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-test-'))
  try {
    writeReleaseReports(directory)
    writePassingNativeLifecycleReport(directory)
    writePassingSourceAuthorizationReports(directory)
    expect(buildDesktopAcceptanceMatrix(directory, '0.56.3')).toMatchObject({
      schema_version: 1,
      status: 'passed',
      release: '0.56.3',
      aggregate: {
        package_targets_passed: 3,
        host_targets_passed: 3,
        control_plane_targets_passed: 3,
        renderer_targets_passed: 3,
        installer_targets_passed: 3,
        source_authorization_targets_passed: 3,
        service_status_targets_passed: 3,
        host_startup_ms_max: 2_000,
      },
    })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects evidence for unsupported targets', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-unsupported-target-test-'))
  try {
    writeReleaseReports(directory)
    const unsupportedDirectory = join(directory, 'x86_64-apple-darwin')
    mkdirSync(unsupportedDirectory, { recursive: true })
    writeFileSync(
      join(unsupportedDirectory, 'x86_64-apple-darwin.json'),
      JSON.stringify({
        status: 'passed',
        version: '0.56.3',
        target: { target: 'x86_64-apple-darwin', platform: 'macOS', architecture: 'x64' },
        installation_type: 'published-release-assets',
      })
    )

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.failures).toContain(
      'unsupported target evidence: x86_64-apple-darwin (published-release-assets)'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects an unrecognized renderer fixture', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-renderer-fixture-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.fixture = 'unapproved-fixture'
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires the large-corpus renderer contract when declared', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-large-fixture-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.fixture = 'provider-free-demo-and-large-corpus'
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires updater manifest binding evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-updater-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    delete report.updater_manifest
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.package_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-release-assets report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires target-specific updater platform binding', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-updater-target-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.updater_manifest.platform = 'linux-x86_64-appimage'
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.package_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-release-assets report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires macOS native lifecycle evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-native-required-test-'))
  try {
    writeReleaseReports(directory)
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.native_lifecycle_targets_passed).toBe(0)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-macos-native-lifecycle report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires packaged source-authorization safety evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-source-auth-required-test-'))
  try {
    writeReleaseReports(directory)
    writePassingNativeLifecycleReport(directory)
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.source_authorization_targets_passed).toBe(0)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-source-authorization report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix records supplemental macOS native lifecycle evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-native-lifecycle-test-'))
  try {
    writeReleaseReports(directory)
    writePassingSourceAuthorizationReports(directory)
    const target = 'aarch64-apple-darwin'
    writeFileSync(
      join(directory, target, `${target}-native-lifecycle.json`),
      JSON.stringify({
        schema_version: 1,
        status: 'passed',
        target: { target, platform: 'macOS', architecture: 'arm64' },
        version: '0.56.3',
        installation_type: 'published-package-macos-native-lifecycle',
        host: { status: 'passed', startup_ms: 2_305, isolated_state: true },
        first_run: {
          no_implicit_connector_install: true,
          query_only_default: true,
          no_implicit_side_effects: true,
        },
        cases: [
          'isolated-user-state',
          'packaged-process-startup',
          'native-tray-status',
          'close-to-tray',
          'tray-reopen',
        ],
        tray: {
          runtimeStatus: true,
          corpusStatus: true,
          ingestionStatus: true,
          sourceJobsStatus: true,
          show: true,
          quit: true,
        },
        window: { initial: 'present', after_close: 'hidden', after_show: 'present' },
        scope: {
          state: 'isolated-temporary-directory',
          provider_network: 'not_requested',
          external_services: 'not_started',
        },
      })
    )

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('passed')
    expect(result.aggregate.native_lifecycle_targets_passed).toBe(1)
    expect(result.targets[0].native_lifecycle).toMatchObject({
      status: 'passed',
      version: '0.56.3',
      startup_ms: 2_305,
    })
    expect(result.targets[1].native_lifecycle).toEqual({ status: 'not_applicable' })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix gates a failed supplemental lifecycle lane', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-native-lifecycle-failure-'))
  try {
    writeReleaseReports(directory)
    const target = 'aarch64-apple-darwin'
    writeFileSync(
      join(directory, target, `${target}-native-lifecycle.json`),
      JSON.stringify({
        schema_version: 1,
        status: 'failed',
        target: { target, platform: 'macOS', architecture: 'arm64' },
        version: '0.56.3',
        installation_type: 'published-package-macos-native-lifecycle',
        error: 'Accessibility permission unavailable',
      })
    )

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.native_lifecycle_targets_passed).toBe(0)
    expect(result.targets[0].native_lifecycle).toEqual({ status: 'failed' })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires every release installer lane', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-installer-test-'))
  try {
    writeReleaseReports(directory)
    rmSync(join(directory, 'x86_64-unknown-linux-gnu', 'x86_64-unknown-linux-gnu-installer.json'))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.installer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'x86_64-unknown-linux-gnu: expected one passing published-release-installer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires installer cleanup evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-cleanup-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'aarch64-apple-darwin',
      'aarch64-apple-darwin-installer.json'
    )
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    delete report.cleanup
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.installer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-release-installer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects incomplete Windows MSI evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-windows-installer-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'x86_64-pc-windows-msvc',
      'x86_64-pc-windows-msvc-installer.json'
    )
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    delete report.uninstall
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.installer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'x86_64-pc-windows-msvc: expected one passing published-release-installer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix fails closed on missing or mismatched evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-failure-test-'))
  try {
    writeReleaseReports(directory)
    rmSync(join(directory, 'x86_64-pc-windows-msvc', 'x86_64-pc-windows-msvc-host.json'))
    const report = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(report.status).toBe('failed')
    expect(report.failures).toContain(
      'x86_64-pc-windows-msvc: expected one passing published-package-host-launch report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix treats packaged control-plane evidence as a required target lane', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-control-plane-test-'))
  try {
    writeReleaseReports(directory)
    rmSync(join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-control-plane.json'))
    const report = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(report.status).toBe('failed')
    expect(report.aggregate.control_plane_targets_passed).toBe(2)
    expect(report.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-control-plane report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix treats packaged service-status evidence as a required target lane', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-service-status-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'aarch64-apple-darwin',
      'aarch64-apple-darwin-service-status.json'
    )
    rmSync(reportPath)
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.service_status_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-service-status report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects service-status evidence that requests mutation', () => {
  const directory = mkdtempSync(
    join(tmpdir(), 'cortana-desktop-matrix-service-status-contract-test-')
  )
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'aarch64-apple-darwin',
      'aarch64-apple-darwin-service-status.json'
    )
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.scope.service_mutation = 'requested'
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.service_status_targets_passed).toBe(2)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects control-plane reports with incomplete step evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-control-plane-steps-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'aarch64-apple-darwin',
      'aarch64-apple-darwin-control-plane.json'
    )
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.steps[report.steps.length - 1].status = 'failed'
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.control_plane_targets_passed).toBe(2)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires invalid-restore preservation evidence', () => {
  const directory = mkdtempSync(
    join(tmpdir(), 'cortana-desktop-matrix-control-plane-recovery-test-')
  )
  try {
    writeReleaseReports(directory)
    const reportPath = join(
      directory,
      'aarch64-apple-darwin',
      'aarch64-apple-darwin-control-plane.json'
    )
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    delete report.recovery
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.control_plane_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-control-plane report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires clean first-run side-effect evidence', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-first-run-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-host.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.cases = report.cases.filter((name) => name !== 'no-implicit-connector-install')
    delete report.host.first_run
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-host-launch report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects a first run that enables synthesis', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-query-only-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-host.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.first_run.query_only_default = false
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-host-launch report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects a failed packaged renderer report', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-renderer-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.status = 'failed'
    report.error = 'responsive control is obscured'
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix requires knowledge, document, and graph axe surfaces', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-axe-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.axe[2].surface = 'knowledge'
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects renderer evidence without measurements', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-resources-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    delete report.resource_metrics.peak
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects impossible p50 values above p95', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-p50-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.resource_metrics.latency_p50_ms.navigation_ms = 101
    report.resource_metrics.latency_p95_ms.navigation_ms = 100
    writeFileSync(reportPath, JSON.stringify(report))

    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')

    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects a screenshot manifest that disagrees with the matrix', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-screenshot-manifest-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.screenshots = ['document-desktop.png']
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects renderer evidence with missing screenshot files', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-screenshot-test-'))
  try {
    writeReleaseReports(directory)
    rmSync(join(directory, 'aarch64-apple-darwin', 'document-desktop.png'))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects an incomplete renderer viewport matrix', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-viewport-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.screenshot_matrix[0].width = 1_440
    report.screenshot_matrix[0].surface = 'graph'
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects renderer evidence from a working tree revision', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-revision-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin-renderer.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.revision = 'working-tree-production-package'
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.renderer_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-package-renderer report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('desktop acceptance matrix rejects evidence with mismatched platform metadata', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-desktop-matrix-target-metadata-test-'))
  try {
    writeReleaseReports(directory)
    const reportPath = join(directory, 'aarch64-apple-darwin', 'aarch64-apple-darwin.json')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    report.target.platform = 'Windows'
    report.target.architecture = 'x64'
    writeFileSync(reportPath, JSON.stringify(report))
    const result = buildDesktopAcceptanceMatrix(directory, '0.56.3')
    expect(result.status).toBe('failed')
    expect(result.aggregate.package_targets_passed).toBe(2)
    expect(result.failures).toContain(
      'aarch64-apple-darwin: expected one passing published-release-assets report'
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
