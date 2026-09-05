import { expect, test } from 'bun:test'

import {
  BROWSER_RESOURCE_THRESHOLDS,
  buildKnowledgeAcceptanceFailureEvidence,
  describeKnowledgeAccessibilityTarget,
  DOCUMENT_SCREENSHOTS,
  LARGE_CORPUS_SCREENSHOTS,
  RESPONSIVE_SCREENSHOTS,
  resolveKnowledgeAcceptanceConfig,
  resolveKnowledgeServerMode,
  summarizeBrowserResourceSamples,
} from './knowledge-accessibility-matrix.mjs'

test('knowledge accessibility server mode defaults to dev and supports production preview', () => {
  expect(resolveKnowledgeServerMode()).toBe('dev')
  expect(resolveKnowledgeServerMode('preview')).toBe('preview')
  expect(() => resolveKnowledgeServerMode('unknown')).toThrow(
    'unsupported knowledge accessibility server mode'
  )
})

test('knowledge accessibility config supports an isolated packaged web bundle', () => {
  expect(
    resolveKnowledgeAcceptanceConfig({
      CORTANA_KNOWLEDGE_WEB_DIR: '/runner/temp/cortana-web',
      CORTANA_KNOWLEDGE_EVIDENCE_DIRECTORY: '/runner/temp/evidence',
      CORTANA_KNOWLEDGE_TARGET: 'aarch64-apple-darwin',
      CORTANA_KNOWLEDGE_VERSION: '0.56.3',
      CORTANA_KNOWLEDGE_REVISION: 'v0.56.3',
      CORTANA_KNOWLEDGE_INSTALLATION_TYPE: 'published-package-renderer',
      CORTANA_KNOWLEDGE_RUN_LARGE: 'false',
    })
  ).toEqual({
    serverMode: 'external',
    packagedWebDirectory: '/runner/temp/cortana-web',
    baseUrl: null,
    evidenceDirectory: '/runner/temp/evidence',
    target: 'aarch64-apple-darwin',
    version: '0.56.3',
    revision: 'v0.56.3',
    installationType: 'published-package-renderer',
    runLargeCorpus: false,
  })
})

test('knowledge accessibility config rejects ambiguous or invalid external settings', () => {
  expect(() =>
    resolveKnowledgeAcceptanceConfig({
      CORTANA_KNOWLEDGE_WEB_DIR: '/runner/temp/cortana-web',
      CORTANA_KNOWLEDGE_BASE_URL: 'http://127.0.0.1:4183',
    })
  ).toThrow('cannot be used together')
  expect(() =>
    resolveKnowledgeAcceptanceConfig({ CORTANA_KNOWLEDGE_RUN_LARGE: 'sometimes' })
  ).toThrow('must be true or false')
  expect(() =>
    resolveKnowledgeAcceptanceConfig({ CORTANA_KNOWLEDGE_INSTALLATION_TYPE: 'untrusted' })
  ).toThrow('must be one of')
})

test('knowledge accessibility config records prospective source renderer provenance', () => {
  expect(
    resolveKnowledgeAcceptanceConfig({
      CORTANA_KNOWLEDGE_INSTALLATION_TYPE: 'prospective-source-renderer',
    }).installationType
  ).toBe('prospective-source-renderer')
})

test('knowledge accessibility target metadata is limited to supported release lanes', () => {
  expect(describeKnowledgeAccessibilityTarget('aarch64-apple-darwin')).toEqual({
    target: 'aarch64-apple-darwin',
    platform: 'macOS',
    architecture: 'arm64',
  })
  expect(() => describeKnowledgeAccessibilityTarget('unknown-target')).toThrow(
    'unsupported knowledge accessibility target'
  )
})

test('knowledge accessibility failure evidence is bounded and preserves the release boundary', () => {
  expect(
    buildKnowledgeAcceptanceFailureEvidence({
      target: 'aarch64-apple-darwin',
      version: '0.56.3',
      revision: 'published-release',
      serverMode: 'external',
      fixture: 'provider-free-release-demo',
      installationType: 'prospective-source-renderer',
      error: `${'x'.repeat(2_000)}\nsecret output`,
    })
  ).toMatchObject({
    schema_version: 1,
    status: 'failed',
    version: '0.56.3',
    revision: 'published-release',
    server_mode: 'external',
    installation_type: 'prospective-source-renderer',
  })
})

test('knowledge accessibility failure evidence preserves sanitized partial progress', () => {
  const evidence = buildKnowledgeAcceptanceFailureEvidence({
    target: 'aarch64-apple-darwin',
    version: '0.56.3',
    revision: 'v0.56.3',
    serverMode: 'external',
    fixture: 'provider-free-release-demo',
    installationType: 'prospective-source-renderer',
    error: 'graph filter failed',
    progress: {
      completed_cases: ['axe-wcag-2.2-aa'],
      axe: [{ surface: 'knowledge', violations: 0, passes: 29 }],
      screenshots: [
        { surface: 'document', width: 1440, height: 900, file: '/private/path/document.png' },
      ],
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
          response_bytes: 1000,
          dom_nodes: 100,
          visible_document_rows: 4,
          visible_graph_nodes: 12,
          js_heap_used_bytes: null,
        },
      },
    },
  })

  expect(evidence.progress).toMatchObject({
    completed_cases: ['axe-wcag-2.2-aa'],
    axe: [{ surface: 'knowledge', violations: 0, passes: 29 }],
    screenshots: [{ surface: 'document', width: 1440, height: 900, file: 'document.png' }],
    resource_metrics: { status: 'passed', sample_count: 3 },
  })
  expect(JSON.stringify(evidence)).not.toContain('/private/path')
  expect(evidence.installation_type).toBe('prospective-source-renderer')
})

test('knowledge accessibility screenshots cover the required responsive widths', () => {
  expect(RESPONSIVE_SCREENSHOTS.map(({ width }) => width)).toEqual([1440, 1024, 768, 720, 390, 320])
  expect(RESPONSIVE_SCREENSHOTS.map(({ file }) => file)).toEqual([
    'graph-desktop-reduced-motion.png',
    'graph-desktop-1024.png',
    'graph-tablet-768.png',
    'graph-desktop-200-percent.png',
    'graph-mobile.png',
    'graph-mobile-320.png',
  ])
  expect(DOCUMENT_SCREENSHOTS.map(({ width }) => width)).toEqual([1440, 1024, 768, 720, 390, 320])
  expect(DOCUMENT_SCREENSHOTS.map(({ file }) => file)).toEqual([
    'document-desktop.png',
    'document-desktop-1024.png',
    'document-tablet-768.png',
    'document-desktop-200-percent.png',
    'document-mobile.png',
    'document-mobile-320.png',
  ])
  expect(LARGE_CORPUS_SCREENSHOTS.map(({ width }) => width)).toEqual([1440, 1440])
  expect(LARGE_CORPUS_SCREENSHOTS.map(({ file }) => file)).toEqual([
    'large-corpus-documents.png',
    'large-corpus-graph.png',
  ])
})

test('browser resource summaries expose p50 and p95 and fail closed on budget regressions', () => {
  const samples = [
    {
      navigation_ms: 30,
      document_open_ms: 40,
      graph_open_ms: 80,
      graph_selection_ms: 20,
      request_count: 12,
      response_bytes: 1200,
      dom_nodes: 300,
      visible_document_rows: 18,
      visible_graph_nodes: 24,
      js_heap_used_bytes: 2_000,
    },
    {
      navigation_ms: 50,
      document_open_ms: 60,
      graph_open_ms: 100,
      graph_selection_ms: 30,
      request_count: 14,
      response_bytes: 1500,
      dom_nodes: 320,
      visible_document_rows: 20,
      visible_graph_nodes: 26,
      js_heap_used_bytes: 2_500,
    },
  ]
  expect(summarizeBrowserResourceSamples(samples)).toMatchObject({
    status: 'passed',
    sample_count: 2,
    latency_p50_ms: {
      navigation_ms: 30,
      document_open_ms: 40,
      graph_open_ms: 80,
      graph_selection_ms: 20,
    },
    latency_p95_ms: {
      navigation_ms: 50,
      document_open_ms: 60,
      graph_open_ms: 100,
      graph_selection_ms: 30,
    },
    peak: {
      request_count: 14,
      response_bytes: 1500,
      dom_nodes: 320,
      visible_document_rows: 20,
      visible_graph_nodes: 26,
      js_heap_used_bytes: 2500,
    },
    thresholds: BROWSER_RESOURCE_THRESHOLDS,
  })
  expect(
    summarizeBrowserResourceSamples([
      { ...samples[0], dom_nodes: BROWSER_RESOURCE_THRESHOLDS.max_dom_nodes + 1 },
    ]).status
  ).toBe('failed')
})
