#!/usr/bin/env node

import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  BROWSER_RESOURCE_THRESHOLDS,
  DOCUMENT_SCREENSHOTS,
  LARGE_CORPUS_SCREENSHOTS,
  RESPONSIVE_SCREENSHOTS,
} from './knowledge-accessibility-matrix.mjs'
import { AUTHORIZATION_CASES } from './desktop-source-authorization-acceptance.mjs'
import { SERVICE_STATUS_CASES } from './desktop-service-status-acceptance.mjs'

export const DESKTOP_TARGETS = Object.freeze([
  Object.freeze({ target: 'aarch64-apple-darwin', platform: 'macOS', architecture: 'arm64' }),
  Object.freeze({
    target: 'x86_64-unknown-linux-gnu',
    platform: 'Linux',
    architecture: 'x64',
  }),
  Object.freeze({
    target: 'x86_64-pc-windows-msvc',
    platform: 'Windows',
    architecture: 'x64',
  }),
])

const PACKAGE_INSTALLATION = 'published-release-assets'
const HOST_INSTALLATION = 'published-package-host-launch'
const CONTROL_PLANE_INSTALLATION = 'published-package-control-plane'
const RENDERER_INSTALLATION = 'published-package-renderer'
const INSTALLER_INSTALLATION = 'published-release-installer'
const NATIVE_LIFECYCLE_INSTALLATION = 'published-package-macos-native-lifecycle'
const SOURCE_AUTHORIZATION_INSTALLATION = 'published-package-source-authorization'
const SERVICE_STATUS_INSTALLATION = 'published-package-service-status'
const MATRIX_INSTALLATION_TYPES = new Set([
  PACKAGE_INSTALLATION,
  HOST_INSTALLATION,
  CONTROL_PLANE_INSTALLATION,
  RENDERER_INSTALLATION,
  INSTALLER_INSTALLATION,
  NATIVE_LIFECYCLE_INSTALLATION,
  SOURCE_AUTHORIZATION_INSTALLATION,
  SERVICE_STATUS_INSTALLATION,
])
const INSTALLER_CASES = Object.freeze([
  'clean-install-first-run',
  'installed-version',
  'installed-web-assets',
  'explicit-connector-install',
  'query-only-default',
  'no-implicit-source-or-schedule-side-effects',
  'installed-core-offline-evaluation',
])
const WINDOWS_INSTALLER_CASES = Object.freeze([
  'clean-install-msi',
  'installed-version',
  'installed-web-assets',
  'installed-core-sidecar',
  'installed-core-offline-evaluation',
  'clean-uninstall-msi',
])
const RENDERER_CASES = Object.freeze([
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
])
const CONTROL_PLANE_CASES = Object.freeze([
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
const NATIVE_LIFECYCLE_CASES = Object.freeze([
  'isolated-user-state',
  'packaged-process-startup',
  'native-tray-status',
  'close-to-tray',
  'tray-reopen',
])
const REQUIRED_AXE_SURFACES = Object.freeze(['knowledge', 'document', 'graph'])
const UPDATER_PLATFORM_FOR_TARGET = Object.freeze({
  'aarch64-apple-darwin': 'darwin-aarch64-app',
  'x86_64-unknown-linux-gnu': 'linux-x86_64-appimage',
  'x86_64-pc-windows-msvc': 'windows-x86_64-nsis',
})
const RESOURCE_LATENCY_METRICS = Object.freeze([
  'navigation_ms',
  'document_open_ms',
  'graph_open_ms',
  'graph_selection_ms',
])
const RESOURCE_PEAK_METRICS = Object.freeze([
  'request_count',
  'response_bytes',
  'dom_nodes',
  'visible_document_rows',
  'visible_graph_nodes',
  'js_heap_used_bytes',
])
export const REQUIRED_RENDERER_SCREENSHOTS = Object.freeze([
  ...DOCUMENT_SCREENSHOTS.map(({ width, height, file }) => ({
    surface: 'document',
    width,
    height,
    file,
  })),
  ...RESPONSIVE_SCREENSHOTS.map(({ width, height, file }) => ({
    surface: 'graph',
    width,
    height,
    file,
  })),
])
const LARGE_CORPUS_RENDERER_SCREENSHOTS = Object.freeze(
  LARGE_CORPUS_SCREENSHOTS.map(({ width, height, file }, index) => ({
    surface: index === 0 ? 'large-corpus-document' : 'large-corpus-graph',
    width,
    height,
    file,
  }))
)
const APPROVED_RENDERER_FIXTURES = new Set([
  'provider-free-release-demo',
  'provider-free-demo-and-large-corpus',
])

function jsonFiles(directory) {
  if (!existsSync(directory)) return []
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) files.push(...jsonFiles(path))
    else if (entry.isFile() && entry.name.endsWith('.json')) files.push(path)
  }
  return files
}

function readReports(directory) {
  const reports = []
  let invalidCount = 0
  for (const path of jsonFiles(directory)) {
    try {
      const value = JSON.parse(readFileSync(path, 'utf8'))
      if (value && typeof value === 'object' && !Array.isArray(value)) {
        reports.push({ path, report: value })
      } else invalidCount += 1
    } catch {
      invalidCount += 1
    }
  }
  return { reports, invalidCount }
}

function reportsFor(reports, descriptor, installationType) {
  return reports.filter(
    ({ report }) =>
      report.target?.target === descriptor.target &&
      report.target?.platform === descriptor.platform &&
      report.target?.architecture === descriptor.architecture &&
      report.installation_type === installationType
  )
}

function packageSummary({ report }, version, target) {
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    report.core?.offline_evaluation !== 'passed' ||
    report.host_acceptance?.status !== 'not_exercised' ||
    report.updater_configuration?.status !== 'passed' ||
    report.updater_manifest?.status !== 'passed' ||
    report.updater_manifest?.version !== version ||
    report.updater_manifest?.platform !== UPDATER_PLATFORM_FOR_TARGET[target]
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    offline_core: 'passed',
    host_interaction: 'not_exercised',
    updater_configuration: 'passed',
  }
}

function hostSummary({ report }, version) {
  const cases = new Set(report.cases)
  const startupMs = report.host?.startup_ms
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    report.host?.status !== 'passed' ||
    report.host?.isolated_state !== true ||
    report.first_run?.no_implicit_connector_install !== true ||
    report.first_run?.query_only_default !== true ||
    report.first_run?.no_implicit_side_effects !== true ||
    !Number.isSafeInteger(startupMs) ||
    startupMs < 0 ||
    !cases.has('isolated-user-state') ||
    !cases.has('packaged-process-startup') ||
    !cases.has('no-implicit-connector-install') ||
    !cases.has('query-only-first-run') ||
    !cases.has('no-implicit-side-effects')
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    startup_ms: startupMs,
    isolated_state: true,
  }
}

function controlPlaneSummary({ report }, version) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const steps = Array.isArray(report.steps) ? report.steps : []
  const completeSteps =
    steps.length === CONTROL_PLANE_CASES.length &&
    new Set(steps.map((step) => step?.name)).size === CONTROL_PLANE_CASES.length &&
    CONTROL_PLANE_CASES.every((name) =>
      steps.some(
        (step) =>
          step?.name === name &&
          step.status === 'passed' &&
          Number.isSafeInteger(step.duration_ms) &&
          step.duration_ms >= 0
      )
    )
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    report.preflight !== 'passed' ||
    !CONTROL_PLANE_CASES.every((name) => cases.has(name)) ||
    !completeSteps ||
    report.recovery?.invalid_restore_preserved_index !== true ||
    report.scope?.network !== 'not-requested' ||
    report.scope?.network_enforcement !== 'not-asserted' ||
    report.scope?.external_services !== 'not_started' ||
    report.scope?.state !== 'isolated-temporary-directory'
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    cases: CONTROL_PLANE_CASES,
    recovery: { invalid_restore_preserved_index: true },
  }
}

function rendererSummary({ path, report }, version) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const isLargeCorpusFixture = report.fixture === 'provider-free-demo-and-large-corpus'
  const requiredRendererCases = isLargeCorpusFixture
    ? [...RENDERER_CASES, 'large-corpus-bounded-rendering']
    : RENDERER_CASES
  const requiredRendererScreenshots = isLargeCorpusFixture
    ? [...REQUIRED_RENDERER_SCREENSHOTS, ...LARGE_CORPUS_RENDERER_SCREENSHOTS]
    : REQUIRED_RENDERER_SCREENSHOTS
  const axe = Array.isArray(report.axe) ? report.axe : []
  const screenshots = Array.isArray(report.screenshot_matrix) ? report.screenshot_matrix : []
  const sampleCount = report.resource_metrics?.sample_count
  const axeSurfaces = new Set(axe.map((surface) => surface?.surface))
  const completeAxeSurfaceSet =
    axe.length === REQUIRED_AXE_SURFACES.length &&
    axeSurfaces.size === REQUIRED_AXE_SURFACES.length &&
    REQUIRED_AXE_SURFACES.every((surface) => axeSurfaces.has(surface))
  const reportDirectory = resolve(path, '..')
  const screenshotFiles = screenshots.map((screenshot) => screenshot?.file)
  const screenshotManifest = Array.isArray(report.screenshots) ? report.screenshots : []
  const screenshotKey = (screenshot) =>
    JSON.stringify([screenshot?.surface, screenshot?.width, screenshot?.height, screenshot?.file])
  const screenshotMatrixKeys = new Set(screenshots.map(screenshotKey))
  const screenshotManifestKeys = new Set(screenshotManifest)
  const completeScreenshotMatrix = requiredRendererScreenshots.every((screenshot) =>
    screenshotMatrixKeys.has(screenshotKey(screenshot))
  )
  const screenshotManifestMatches =
    screenshotManifest.length === screenshotFiles.length &&
    screenshotManifestKeys.size === screenshotFiles.length &&
    screenshotFiles.every((file) => screenshotManifestKeys.has(file))
  const resourceMetrics = report.resource_metrics
  const resourceThresholds = resourceMetrics?.thresholds
  const resourceLatencyP50 = resourceMetrics?.latency_p50_ms
  const resourceLatency = resourceMetrics?.latency_p95_ms
  const resourcePeak = resourceMetrics?.peak
  const completeResourceMeasurements =
    resourceMetrics?.status === 'passed' &&
    Number.isSafeInteger(resourceMetrics.sample_count) &&
    resourceMetrics.sample_count >= 1 &&
    resourceLatency &&
    resourceLatencyP50 &&
    Object.keys(resourceLatencyP50).length === RESOURCE_LATENCY_METRICS.length &&
    RESOURCE_LATENCY_METRICS.every(
      (metric) =>
        Number.isSafeInteger(resourceLatencyP50[metric]) && resourceLatencyP50[metric] >= 0
    ) &&
    Object.keys(resourceLatency).length === RESOURCE_LATENCY_METRICS.length &&
    RESOURCE_LATENCY_METRICS.every(
      (metric) => Number.isSafeInteger(resourceLatency[metric]) && resourceLatency[metric] >= 0
    ) &&
    RESOURCE_LATENCY_METRICS.every(
      (metric) => resourceLatencyP50[metric] <= resourceLatency[metric]
    ) &&
    resourcePeak &&
    Object.keys(resourcePeak).length === RESOURCE_PEAK_METRICS.length &&
    RESOURCE_PEAK_METRICS.every(
      (metric) =>
        (metric === 'js_heap_used_bytes' && resourcePeak[metric] === null) ||
        (Number.isSafeInteger(resourcePeak[metric]) && resourcePeak[metric] >= 0)
    ) &&
    resourceThresholds &&
    JSON.stringify(resourceThresholds) === JSON.stringify(BROWSER_RESOURCE_THRESHOLDS)
  const screenshotsPresent =
    screenshots.length >= requiredRendererScreenshots.length &&
    completeScreenshotMatrix &&
    screenshotManifestMatches &&
    new Set(screenshotFiles).size === screenshots.length &&
    screenshotFiles.every((file) => {
      if (typeof file !== 'string' || !file || file.includes('\0')) return false
      const screenshotPath = resolve(reportDirectory, file)
      const relativeScreenshotPath = relative(reportDirectory, screenshotPath)
      if (
        !relativeScreenshotPath ||
        relativeScreenshotPath.startsWith('..') ||
        relativeScreenshotPath.startsWith('/')
      ) {
        return false
      }
      try {
        const metadata = statSync(screenshotPath)
        return metadata.isFile() && metadata.size > 0
      } catch {
        return false
      }
    })
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    report.revision !== `v${version}` ||
    report.server_mode !== 'external' ||
    !APPROVED_RENDERER_FIXTURES.has(report.fixture) ||
    !requiredRendererCases.every((name) => cases.has(name)) ||
    !completeAxeSurfaceSet ||
    axe.some((surface) => surface?.violations !== 0) ||
    !screenshotsPresent ||
    !completeResourceMeasurements
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    fixture: report.fixture,
    axe_surfaces: axe.length,
    screenshots: screenshots.length,
    screenshot_files_present: true,
    screenshot_manifest_matches: true,
    resource_samples: sampleCount,
    resource_metrics: {
      latency_p95_ms: resourceLatency,
      latency_p50_ms: resourceLatencyP50,
      peak: resourcePeak,
    },
  }
}

function installerSummary({ report }, version, target) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const isWindows = target === 'x86_64-pc-windows-msvc'
  const requiredCases = isWindows ? WINDOWS_INSTALLER_CASES : INSTALLER_CASES
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    !requiredCases.every((name) => cases.has(name)) ||
    report.install?.status !== 'passed' ||
    report.cleanup?.status !== 'passed' ||
    report.cleanup?.state_root_removed !== true ||
    report.core?.reported_version !== `cortana ${version}` ||
    report.core?.offline_evaluation !== 'passed' ||
    report.services?.status !== 'not_exercised' ||
    report.scope?.provider_network !== 'not_requested' ||
    report.scope?.external_services !== 'not_started' ||
    report.scope?.state !== 'isolated-temporary-directory'
  ) {
    return null
  }
  if (isWindows) {
    if (report.uninstall?.status !== 'passed' || report.first_run?.status !== 'not_exercised') {
      return null
    }
  } else if (
    report.first_run?.query_only_default !== true ||
    report.first_run?.no_source_or_schedule_side_effects !== true ||
    report.first_run?.connector_installed_by_explicit_installer !== true
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    cases: requiredCases,
  }
}

function nativeLifecycleSummary({ report }, version) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const tray = report.tray
  const startupMs = report.host?.startup_ms
  const trayReady =
    tray &&
    typeof tray === 'object' &&
    Object.keys(tray).length === 6 &&
    Object.values(tray).every((value) => value === true)
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    report.target?.target !== 'aarch64-apple-darwin' ||
    report.target?.platform !== 'macOS' ||
    report.target?.architecture !== 'arm64' ||
    report.host?.status !== 'passed' ||
    report.host?.isolated_state !== true ||
    !Number.isSafeInteger(startupMs) ||
    startupMs < 0 ||
    !NATIVE_LIFECYCLE_CASES.every((name) => cases.has(name)) ||
    report.first_run?.no_implicit_connector_install !== true ||
    report.first_run?.query_only_default !== true ||
    report.first_run?.no_implicit_side_effects !== true ||
    !trayReady ||
    report.window?.initial !== 'present' ||
    report.window?.after_close !== 'hidden' ||
    report.window?.after_show !== 'present' ||
    report.scope?.state !== 'isolated-temporary-directory' ||
    report.scope?.provider_network !== 'not_requested' ||
    report.scope?.external_services !== 'not_started'
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    startup_ms: startupMs,
    tray_status: 'passed',
    close_to_tray: 'passed',
  }
}

function sourceAuthorizationSummary({ report }, version) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const steps = Array.isArray(report.steps) ? report.steps : []
  const completeSteps =
    steps.length === AUTHORIZATION_CASES.length &&
    new Set(steps.map((step) => step?.name)).size === AUTHORIZATION_CASES.length &&
    AUTHORIZATION_CASES.every((name) =>
      steps.some(
        (step) =>
          step?.name === name &&
          step.status === 'passed' &&
          (name === 'authorization-no-sync-side-effect' || step.expected_failure === true)
      )
    )
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    !AUTHORIZATION_CASES.every((name) => cases.has(name)) ||
    !completeSteps ||
    report.state_changed !== false ||
    report.scope?.provider_network !== 'not_requested' ||
    report.scope?.external_services !== 'not_started' ||
    report.scope?.state !== 'isolated-temporary-directory' ||
    report.scope?.source_data !== 'not_read'
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    cases: AUTHORIZATION_CASES,
    state_changed: false,
  }
}

function serviceStatusSummary({ report }, version) {
  const cases = Array.isArray(report.cases) ? new Set(report.cases) : new Set()
  const steps = Array.isArray(report.steps) ? report.steps : []
  const completeSteps =
    steps.length === 1 &&
    steps[0]?.name === 'packaged-service-status' &&
    steps[0].status === 'passed' &&
    Number.isSafeInteger(steps[0].duration_ms) &&
    steps[0].duration_ms >= 0
  if (
    report.status !== 'passed' ||
    report.version !== version ||
    !SERVICE_STATUS_CASES.every((name) => cases.has(name)) ||
    !completeSteps ||
    typeof report.service_manager?.supported !== 'boolean' ||
    report.service_manager?.operation !== 'status-only' ||
    report.state_unchanged !== true ||
    report.scope?.provider_network !== 'not-requested' ||
    report.scope?.external_services !== 'not_started' ||
    report.scope?.state !== 'isolated-temporary-directory' ||
    report.scope?.service_mutation !== 'not-requested'
  ) {
    return null
  }
  return {
    status: 'passed',
    version,
    cases: SERVICE_STATUS_CASES,
    service_manager_supported: report.service_manager.supported,
    state_unchanged: true,
  }
}

export function buildDesktopAcceptanceMatrix(directory, version) {
  const { reports, invalidCount } = readReports(resolve(directory))
  const failures = []
  if (invalidCount > 0) failures.push(`ignored evidence JSON files were invalid: ${invalidCount}`)
  const supportedTargets = new Set(DESKTOP_TARGETS.map((descriptor) => descriptor.target))
  for (const { report } of reports) {
    const installationType = report.installation_type
    const target = report.target?.target
    if (
      MATRIX_INSTALLATION_TYPES.has(installationType) &&
      typeof target === 'string' &&
      !supportedTargets.has(target)
    ) {
      failures.push(`unsupported target evidence: ${target} (${installationType})`)
    }
  }
  const targets = DESKTOP_TARGETS.map((descriptor) => {
    const packageReports = reportsFor(reports, descriptor, PACKAGE_INSTALLATION)
    const hostReports = reportsFor(reports, descriptor, HOST_INSTALLATION)
    const controlPlaneReports = reportsFor(reports, descriptor, CONTROL_PLANE_INSTALLATION)
    const rendererReports = reportsFor(reports, descriptor, RENDERER_INSTALLATION)
    const installerReports = reportsFor(reports, descriptor, INSTALLER_INSTALLATION)
    const nativeLifecycleReports = reportsFor(reports, descriptor, NATIVE_LIFECYCLE_INSTALLATION)
    const sourceAuthorizationReports = reportsFor(
      reports,
      descriptor,
      SOURCE_AUTHORIZATION_INSTALLATION
    )
    const serviceStatusReports = reportsFor(reports, descriptor, SERVICE_STATUS_INSTALLATION)
    const packageEvidence =
      packageReports.length === 1
        ? packageSummary(packageReports[0], version, descriptor.target)
        : null
    const hostEvidence = hostReports.length === 1 ? hostSummary(hostReports[0], version) : null
    const controlPlaneEvidence =
      controlPlaneReports.length === 1 ? controlPlaneSummary(controlPlaneReports[0], version) : null
    const rendererEvidence =
      rendererReports.length === 1 ? rendererSummary(rendererReports[0], version) : null
    const installerEvidence =
      installerReports.length === 1
        ? installerSummary(installerReports[0], version, descriptor.target)
        : null
    const nativeLifecycleEvidence =
      descriptor.target !== 'aarch64-apple-darwin'
        ? { status: 'not_applicable' }
        : nativeLifecycleReports.length === 0
          ? { status: 'not_collected' }
          : nativeLifecycleReports.length > 1
            ? { status: 'ambiguous' }
            : (nativeLifecycleSummary(nativeLifecycleReports[0], version) ?? { status: 'failed' })
    const sourceAuthorizationEvidence =
      sourceAuthorizationReports.length === 1
        ? (sourceAuthorizationSummary(sourceAuthorizationReports[0], version) ?? {
            status: 'failed',
          })
        : null
    const serviceStatusEvidence =
      serviceStatusReports.length === 1
        ? (serviceStatusSummary(serviceStatusReports[0], version) ?? { status: 'failed' })
        : null
    if (!packageEvidence || packageReports.length !== 1) {
      failures.push(`${descriptor.target}: expected one passing ${PACKAGE_INSTALLATION} report`)
    }
    if (!hostEvidence || hostReports.length !== 1) {
      failures.push(`${descriptor.target}: expected one passing ${HOST_INSTALLATION} report`)
    }
    if (!controlPlaneEvidence || controlPlaneReports.length !== 1) {
      failures.push(
        `${descriptor.target}: expected one passing ${CONTROL_PLANE_INSTALLATION} report`
      )
    }
    if (!rendererEvidence || rendererReports.length !== 1) {
      failures.push(`${descriptor.target}: expected one passing ${RENDERER_INSTALLATION} report`)
    }
    if (!installerEvidence || installerReports.length !== 1) {
      failures.push(`${descriptor.target}: expected one passing ${INSTALLER_INSTALLATION} report`)
    }
    if (
      descriptor.target === 'aarch64-apple-darwin' &&
      nativeLifecycleEvidence.status !== 'passed'
    ) {
      failures.push(
        `${descriptor.target}: expected one passing ${NATIVE_LIFECYCLE_INSTALLATION} report`
      )
    }
    if (!sourceAuthorizationEvidence || sourceAuthorizationReports.length !== 1) {
      failures.push(
        `${descriptor.target}: expected one passing ${SOURCE_AUTHORIZATION_INSTALLATION} report`
      )
    }
    if (!serviceStatusEvidence || serviceStatusReports.length !== 1) {
      failures.push(
        `${descriptor.target}: expected one passing ${SERVICE_STATUS_INSTALLATION} report`
      )
    }
    return {
      ...descriptor,
      package: packageEvidence,
      host: hostEvidence,
      control_plane: controlPlaneEvidence,
      renderer: rendererEvidence,
      installer: installerEvidence,
      native_lifecycle: nativeLifecycleEvidence,
      source_authorization: sourceAuthorizationEvidence,
      service_status: serviceStatusEvidence,
    }
  })
  const startupValues = targets
    .map((target) => target.host?.startup_ms)
    .filter((value) => Number.isSafeInteger(value))
  return {
    schema_version: 1,
    status: failures.length === 0 ? 'passed' : 'failed',
    release: version,
    targets,
    aggregate: {
      package_targets_passed: targets.filter((target) => target.package?.status === 'passed')
        .length,
      host_targets_passed: targets.filter((target) => target.host?.status === 'passed').length,
      control_plane_targets_passed: targets.filter(
        (target) => target.control_plane?.status === 'passed'
      ).length,
      renderer_targets_passed: targets.filter((target) => target.renderer?.status === 'passed')
        .length,
      installer_targets_passed: targets.filter((target) => target.installer?.status === 'passed')
        .length,
      source_authorization_targets_passed: targets.filter(
        (target) => target.source_authorization?.status === 'passed'
      ).length,
      service_status_targets_passed: targets.filter(
        (target) => target.service_status?.status === 'passed'
      ).length,
      native_lifecycle_targets_passed: targets.filter(
        (target) => target.native_lifecycle?.status === 'passed'
      ).length,
      host_startup_ms_max: Math.max(0, ...startupValues),
    },
    limitations: [
      'This matrix verifies published package/core evidence, Unix release-archive or Windows MSI installation and cleanup, disposable packaged control-plane behavior, isolated process startup, and packaged renderer evidence.',
      'The control-plane lane uses provider-free offline commands but does not provide an OS-level network sandbox.',
      'The required macOS native lifecycle lane covers the status item, close-to-tray, and tray reopen only; services, autostart, and embedded renderer controls remain separate acceptance lanes.',
      'Interactive renderer controls, native dialogs, services, OAuth, updater lifecycle, manual accessibility, recovery, uninstall, and OS trust require separate acceptance lanes.',
    ],
    failures,
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

function outputInside(directory, output) {
  const base = resolve(directory)
  const path = resolve(output)
  const relativeOutput = relative(base, path)
  if (!relativeOutput || relativeOutput.startsWith('..') || relativeOutput.startsWith('/')) {
    throw new Error(`evidence output must stay inside ${base}`)
  }
  return path
}

export function main(args = process.argv.slice(2)) {
  const values = parseArguments(args)
  const version = values.version
  const directory = values['evidence-dir']
  if (!version || !directory) {
    throw new Error(
      'usage: desktop-acceptance-matrix.mjs --version VERSION --evidence-dir DIR [--output FILE]'
    )
  }
  const output = values.output || resolve(directory, `desktop-acceptance-matrix-${version}.json`)
  mkdirSync(directory, { recursive: true })
  const outputPath = outputInside(directory, output)
  const evidence = buildDesktopAcceptanceMatrix(directory, version)
  writeFileSync(outputPath, `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(`desktop acceptance matrix ${evidence.status}: ${outputPath}`)
  if (evidence.status !== 'passed') process.exitCode = 1
  return evidence
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}
