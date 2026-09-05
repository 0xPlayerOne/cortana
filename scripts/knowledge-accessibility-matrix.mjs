export const RESPONSIVE_SCREENSHOTS = Object.freeze([
  Object.freeze({ width: 1440, height: 900, file: 'graph-desktop-reduced-motion.png' }),
  Object.freeze({ width: 1024, height: 900, file: 'graph-desktop-1024.png' }),
  Object.freeze({ width: 768, height: 900, file: 'graph-tablet-768.png' }),
  Object.freeze({ width: 720, height: 900, file: 'graph-desktop-200-percent.png' }),
  Object.freeze({ width: 390, height: 844, file: 'graph-mobile.png' }),
  Object.freeze({ width: 320, height: 844, file: 'graph-mobile-320.png' }),
])

export const DOCUMENT_SCREENSHOTS = Object.freeze([
  Object.freeze({ width: 1440, height: 900, file: 'document-desktop.png' }),
  Object.freeze({ width: 1024, height: 900, file: 'document-desktop-1024.png' }),
  Object.freeze({ width: 768, height: 900, file: 'document-tablet-768.png' }),
  Object.freeze({ width: 720, height: 900, file: 'document-desktop-200-percent.png' }),
  Object.freeze({ width: 390, height: 844, file: 'document-mobile.png' }),
  Object.freeze({ width: 320, height: 844, file: 'document-mobile-320.png' }),
])

export const LARGE_CORPUS_SCREENSHOTS = Object.freeze([
  Object.freeze({ width: 1440, height: 900, file: 'large-corpus-documents.png' }),
  Object.freeze({ width: 1440, height: 900, file: 'large-corpus-graph.png' }),
])

export const BROWSER_RESOURCE_THRESHOLDS = Object.freeze({
  max_navigation_p95_ms: 5_000,
  max_document_open_p95_ms: 2_000,
  max_graph_open_p95_ms: 3_000,
  max_graph_selection_p95_ms: 1_000,
  max_request_count: 200,
  max_response_bytes: 16 * 1024 * 1024,
  max_dom_nodes: 20_000,
  max_visible_document_rows: 100,
  max_visible_graph_nodes: 200,
  max_js_heap_used_bytes: 512 * 1024 * 1024,
})

const TARGETS = Object.freeze({
  'aarch64-apple-darwin': Object.freeze({ platform: 'macOS', architecture: 'arm64' }),
  'x86_64-unknown-linux-gnu': Object.freeze({ platform: 'Linux', architecture: 'x64' }),
  'x86_64-pc-windows-msvc': Object.freeze({ platform: 'Windows', architecture: 'x64' }),
})

export const KNOWLEDGE_RENDERER_INSTALLATION_TYPES = Object.freeze([
  'published-package-renderer',
  'prospective-source-renderer',
])

export function describeKnowledgeAccessibilityTarget(target) {
  const descriptor = TARGETS[target]
  if (!descriptor) throw new Error(`unsupported knowledge accessibility target: ${target}`)
  return { ...descriptor, target }
}

function optionalEnvironmentValue(value) {
  return typeof value === 'string' && value.trim() ? value.trim() : null
}

export function resolveKnowledgeAcceptanceConfig(env = process.env) {
  const packagedWebDirectory = optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_WEB_DIR)
  const baseUrl = optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_BASE_URL)
  if (packagedWebDirectory && baseUrl) {
    throw new Error(
      'CORTANA_KNOWLEDGE_WEB_DIR and CORTANA_KNOWLEDGE_BASE_URL cannot be used together'
    )
  }

  const largeCorpusSetting = optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_RUN_LARGE)
  if (largeCorpusSetting && !['true', 'false'].includes(largeCorpusSetting)) {
    throw new Error('CORTANA_KNOWLEDGE_RUN_LARGE must be true or false')
  }

  const installationType =
    optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_INSTALLATION_TYPE) ||
    KNOWLEDGE_RENDERER_INSTALLATION_TYPES[0]
  if (!KNOWLEDGE_RENDERER_INSTALLATION_TYPES.includes(installationType)) {
    throw new Error(
      `CORTANA_KNOWLEDGE_INSTALLATION_TYPE must be one of ${KNOWLEDGE_RENDERER_INSTALLATION_TYPES.join(', ')}`
    )
  }

  return {
    serverMode:
      packagedWebDirectory || baseUrl
        ? 'external'
        : resolveKnowledgeServerMode(env.CORTANA_KNOWLEDGE_SERVER),
    packagedWebDirectory,
    baseUrl,
    evidenceDirectory: optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_EVIDENCE_DIRECTORY),
    target: optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_TARGET),
    version: optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_VERSION),
    revision: optionalEnvironmentValue(env.CORTANA_KNOWLEDGE_REVISION),
    installationType,
    runLargeCorpus: largeCorpusSetting !== 'false',
  }
}

const PROGRESS_LATENCY_METRICS = Object.freeze([
  'navigation_ms',
  'document_open_ms',
  'graph_open_ms',
  'graph_selection_ms',
])
const PROGRESS_PEAK_METRICS = Object.freeze([
  'request_count',
  'response_bytes',
  'dom_nodes',
  'visible_document_rows',
  'visible_graph_nodes',
  'js_heap_used_bytes',
])

function boundedProgressLabel(value) {
  return typeof value === 'string'
    ? value.replace(/[\u0000-\u001f\u007f]/g, ' ').slice(0, 128)
    : null
}

function boundedProgressNumber(value) {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

function sanitizeAcceptanceResourceMetrics(value) {
  if (!value || typeof value !== 'object') return null
  const status = value.status === 'passed' || value.status === 'failed' ? value.status : null
  const sampleCount = Number.isSafeInteger(value.sample_count) ? value.sample_count : null
  if (!status || sampleCount === null || sampleCount < 1) return null

  const latencyP50 = {}
  const latencyP95 = {}
  for (const metric of PROGRESS_LATENCY_METRICS) {
    const p50 = boundedProgressNumber(value.latency_p50_ms?.[metric])
    const p95 = boundedProgressNumber(value.latency_p95_ms?.[metric])
    if (p50 === null || p95 === null) return null
    latencyP50[metric] = p50
    latencyP95[metric] = p95
  }

  const peak = {}
  for (const metric of PROGRESS_PEAK_METRICS) {
    const metricValue = value.peak?.[metric]
    if (metric === 'js_heap_used_bytes' && metricValue === null) {
      peak[metric] = null
      continue
    }
    const number = boundedProgressNumber(metricValue)
    if (number === null) return null
    peak[metric] = number
  }

  return {
    status,
    sample_count: sampleCount,
    latency_p50_ms: latencyP50,
    latency_p95_ms: latencyP95,
    peak,
  }
}

function sanitizeAcceptanceProgress(value) {
  if (!value || typeof value !== 'object') return null
  const completedCases = Array.isArray(value.completed_cases)
    ? value.completed_cases
        .map(boundedProgressLabel)
        .filter((label) => label !== null)
        .slice(0, 64)
    : []
  const axe = Array.isArray(value.axe)
    ? value.axe
        .flatMap((surface) => {
          const label = boundedProgressLabel(surface?.surface)
          if (
            label === null ||
            !Number.isSafeInteger(surface?.violations) ||
            surface.violations < 0 ||
            !Number.isSafeInteger(surface?.passes) ||
            surface.passes < 0
          ) {
            return []
          }
          return [{ surface: label, violations: surface.violations, passes: surface.passes }]
        })
        .slice(0, 8)
    : []
  const screenshots = Array.isArray(value.screenshots)
    ? value.screenshots
        .flatMap((screenshot) => {
          const surface = boundedProgressLabel(screenshot?.surface)
          const file =
            typeof screenshot?.file === 'string'
              ? screenshot.file.split(/[\\/]/).filter(Boolean).at(-1)
              : null
          if (
            surface === null ||
            !Number.isSafeInteger(screenshot?.width) ||
            screenshot.width < 1 ||
            !Number.isSafeInteger(screenshot?.height) ||
            screenshot.height < 1 ||
            !file
          ) {
            return []
          }
          return [
            {
              surface,
              width: screenshot.width,
              height: screenshot.height,
              file: file.replace(/[\u0000-\u001f\u007f]/g, ' ').slice(0, 256),
            },
          ]
        })
        .slice(0, 32)
    : []
  const resourceMetrics = sanitizeAcceptanceResourceMetrics(value.resource_metrics)
  return {
    completed_cases: [...new Set(completedCases)],
    axe,
    screenshots,
    resource_metrics: resourceMetrics,
  }
}

export function buildKnowledgeAcceptanceFailureEvidence({
  target,
  version,
  revision,
  serverMode,
  fixture,
  installationType = KNOWLEDGE_RENDERER_INSTALLATION_TYPES[0],
  error,
  progress,
}) {
  const message = String(error instanceof Error ? error.message : error)
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .slice(0, 1_000)
  return {
    schema_version: 1,
    status: 'failed',
    ...(target
      ? {
          target: describeKnowledgeAccessibilityTarget(target),
          installation_type: installationType,
        }
      : {}),
    version,
    revision,
    fixture,
    browser: 'chromium-headless',
    server_mode: serverMode,
    error: message,
    ...(progress !== undefined ? { progress: sanitizeAcceptanceProgress(progress) } : {}),
    generated_at: new Date().toISOString(),
  }
}

export function resolveKnowledgeServerMode(value = 'dev') {
  const mode = value || 'dev'
  if (!['dev', 'preview'].includes(mode)) {
    throw new Error(`unsupported knowledge accessibility server mode: ${mode}`)
  }
  return mode
}

const LATENCY_METRICS = Object.freeze([
  ['navigation_ms', 'max_navigation_p95_ms'],
  ['document_open_ms', 'max_document_open_p95_ms'],
  ['graph_open_ms', 'max_graph_open_p95_ms'],
  ['graph_selection_ms', 'max_graph_selection_p95_ms'],
])
const PEAK_METRICS = Object.freeze([
  ['request_count', 'max_request_count'],
  ['response_bytes', 'max_response_bytes'],
  ['dom_nodes', 'max_dom_nodes'],
  ['visible_document_rows', 'max_visible_document_rows'],
  ['visible_graph_nodes', 'max_visible_graph_nodes'],
  ['js_heap_used_bytes', 'max_js_heap_used_bytes'],
])

function percentile(values, fraction) {
  const ordered = [...values].sort((left, right) => left - right)
  return ordered[Math.max(0, Math.ceil(ordered.length * fraction) - 1)]
}

function isNonNegativeNumber(value) {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
}

export function summarizeBrowserResourceSamples(samples) {
  const failures = []
  if (!Array.isArray(samples) || samples.length === 0) {
    return {
      status: 'failed',
      sample_count: 0,
      latency_p50_ms: null,
      latency_p95_ms: null,
      peak: null,
      thresholds: BROWSER_RESOURCE_THRESHOLDS,
      failures: ['at least one browser resource sample is required'],
    }
  }

  const latencyP50 = {}
  const latencyP95 = {}
  for (const [metric, threshold] of LATENCY_METRICS) {
    const values = samples.map((sample) => sample?.[metric])
    if (!values.every(isNonNegativeNumber)) {
      failures.push(`${metric} is missing or invalid`)
      continue
    }
    latencyP50[metric] = Math.round(percentile(values, 0.5))
    latencyP95[metric] = Math.round(percentile(values, 0.95))
    if (latencyP95[metric] > BROWSER_RESOURCE_THRESHOLDS[threshold]) {
      failures.push(`${metric} p95 exceeds its threshold`)
    }
  }

  const peak = {}
  for (const [metric, threshold] of PEAK_METRICS) {
    const values = samples.map((sample) => sample?.[metric])
    const nullable = metric === 'js_heap_used_bytes'
    if (nullable && values.every((value) => value === null || value === undefined)) {
      peak[metric] = null
      continue
    }
    if (!values.every(isNonNegativeNumber)) {
      failures.push(`${metric} is missing or invalid`)
      continue
    }
    peak[metric] = Math.max(...values)
    if (peak[metric] > BROWSER_RESOURCE_THRESHOLDS[threshold]) {
      failures.push(`${metric} exceeds its threshold`)
    }
  }

  return {
    status: failures.length === 0 ? 'passed' : 'failed',
    sample_count: samples.length,
    latency_p50_ms: latencyP50,
    latency_p95_ms: latencyP95,
    peak,
    thresholds: BROWSER_RESOURCE_THRESHOLDS,
    failures,
  }
}
