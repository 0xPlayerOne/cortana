#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import AxeBuilder from '@axe-core/playwright'
import { chromium } from 'playwright'

import {
  DOCUMENT_SCREENSHOTS,
  LARGE_CORPUS_SCREENSHOTS,
  RESPONSIVE_SCREENSHOTS,
  buildKnowledgeAcceptanceFailureEvidence,
  describeKnowledgeAccessibilityTarget,
  resolveKnowledgeAcceptanceConfig,
  summarizeBrowserResourceSamples,
} from './knowledge-accessibility-matrix.mjs'
import {
  setViewportAndWaitForLayout,
  waitForInitialKeyboardTarget,
} from './knowledge-accessibility-browser.mjs'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const PROJECT_VERSION = JSON.parse(
  readFileSync(resolve(ROOT, 'apps/web/package.json'), 'utf8')
).version
const PORT = 4183
const ACCEPTANCE_CONFIG = resolveKnowledgeAcceptanceConfig()
const EVIDENCE_DIRECTORY = resolve(
  ROOT,
  ACCEPTANCE_CONFIG.evidenceDirectory || 'artifacts/knowledge-accessibility'
)
const EXPECTED_VERSION = ACCEPTANCE_CONFIG.version || PROJECT_VERSION
const ACCEPTANCE_REVISION = ACCEPTANCE_CONFIG.revision || process.env.GITHUB_SHA || 'working-tree'
const SERVER_MODE = ACCEPTANCE_CONFIG.serverMode
const BASE_URL = new URL(
  '?demo=1',
  ACCEPTANCE_CONFIG.baseUrl || `http://127.0.0.1:${PORT}/`
).toString()
const LARGE_BASE_URL = new URL(
  '?demo=large',
  ACCEPTANCE_CONFIG.baseUrl || `http://127.0.0.1:${PORT}/`
).toString()
const RUN_LARGE_CORPUS = ACCEPTANCE_CONFIG.runLargeCorpus

function ensure(condition, message) {
  if (!condition) throw new Error(message)
}

async function waitForPaint(page) {
  await page.evaluate(
    () =>
      new Promise((resolvePaint) =>
        requestAnimationFrame(() => requestAnimationFrame(resolvePaint))
      )
  )
}

async function readBrowserResourceSnapshot(page) {
  return page.evaluate(() => {
    const resources = performance.getEntriesByType('resource')
    const memory = performance.memory
    const byteSize = (entry) =>
      entry.transferSize || entry.encodedBodySize || entry.decodedBodySize || 0
    return {
      request_count: resources.length,
      response_bytes: resources.reduce((total, entry) => total + byteSize(entry), 0),
      dom_nodes: document.getElementsByTagName('*').length,
      visible_document_rows: document.querySelectorAll('[data-m7-document-row]').length,
      visible_graph_nodes: document.querySelectorAll('.graph-node').length,
      js_heap_used_bytes:
        typeof memory?.usedJSHeapSize === 'number' ? Math.round(memory.usedJSHeapSize) : null,
    }
  })
}

function mergeResourceSnapshots(...snapshots) {
  const max = (metric) => Math.max(...snapshots.map((snapshot) => snapshot[metric]))
  const heapValues = snapshots
    .map((snapshot) => snapshot.js_heap_used_bytes)
    .filter((value) => typeof value === 'number')
  return {
    request_count: max('request_count'),
    response_bytes: max('response_bytes'),
    dom_nodes: max('dom_nodes'),
    visible_document_rows: max('visible_document_rows'),
    visible_graph_nodes: max('visible_graph_nodes'),
    js_heap_used_bytes: heapValues.length ? Math.max(...heapValues) : null,
  }
}

async function measureInteraction(page, action) {
  const startedAt = Date.now()
  await action()
  await waitForPaint(page)
  return Date.now() - startedAt
}

async function waitForServer(timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(BASE_URL)
      if (response.ok) return
    } catch {
      // The bounded poll continues until Vite is accepting connections.
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 100))
  }
  throw new Error('knowledge accessibility server did not become ready')
}

async function stopServer(server) {
  if (!server?.pid || server.exitCode !== null) return
  try {
    process.kill(-server.pid, 'SIGTERM')
  } catch (error) {
    if (error?.code !== 'ESRCH') server.kill('SIGTERM')
  }
  await Promise.race([
    new Promise((resolveWait) => server.once('exit', resolveWait)),
    new Promise((resolveWait) => setTimeout(resolveWait, 2_000)),
  ])
  if (server.exitCode === null) {
    try {
      process.kill(-server.pid, 'SIGKILL')
    } catch (error) {
      if (error?.code !== 'ESRCH') server.kill('SIGKILL')
    }
  }
}

async function assertAxe(page, surface) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag22aa'])
    .analyze()
  const violations = results.violations.filter((violation) =>
    ['critical', 'serious'].includes(violation.impact ?? '')
  )
  ensure(
    violations.length === 0,
    `${surface} has serious axe violations: ${violations
      .map((violation) => `${violation.id} (${violation.nodes.length})`)
      .join(', ')}`
  )
  return { surface, violations: results.violations.length, passes: results.passes.length }
}

async function assertControlExposed(page, label, viewportWidth) {
  const control = page.getByLabel(label)
  const box = await control.boundingBox()
  ensure(
    box && box.x >= 0 && box.x + box.width <= viewportWidth,
    `${label} does not reflow inside the ${viewportWidth}px viewport`
  )
  const exposed = await control.evaluate((element) => {
    const box = element.getBoundingClientRect()
    const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)
    const rectangle = (selector) => {
      const candidate = document.querySelector(selector)
      if (!candidate) return null
      const candidateBox = candidate.getBoundingClientRect()
      return {
        x: candidateBox.x,
        y: candidateBox.y,
        width: candidateBox.width,
        height: candidateBox.height,
      }
    }
    return {
      visible: hit === element || element.contains(hit),
      box: {
        x: box.x,
        y: box.y,
        width: box.width,
        height: box.height,
      },
      toolbar: rectangle('.graph-toolbar'),
      kindFilter: rectangle('.graph-kind-filter'),
      summary: rectangle('.graph-summary'),
      selection: rectangle('.graph-selection'),
      hit: hit
        ? {
            tag: hit.tagName,
            role: hit.getAttribute('role'),
            label: hit.getAttribute('aria-label'),
            class_name: hit.getAttribute('class'),
          }
        : null,
    }
  })
  ensure(
    exposed.visible,
    `${label} is obscured by another graph control at ${viewportWidth}px: ${JSON.stringify(exposed)}`
  )
}

async function run() {
  mkdirSync(EVIDENCE_DIRECTORY, { recursive: true })
  const server = ACCEPTANCE_CONFIG.packagedWebDirectory
    ? spawn(
        process.execPath,
        [
          resolve(ROOT, 'scripts/static-web-server.mjs'),
          '--directory',
          ACCEPTANCE_CONFIG.packagedWebDirectory,
          '--address',
          '127.0.0.1',
          '--port',
          String(PORT),
        ],
        { cwd: ROOT, detached: true, stdio: 'ignore' }
      )
    : ACCEPTANCE_CONFIG.baseUrl
      ? null
      : spawn(
          'bun',
          [
            'run',
            '--cwd',
            'apps/web',
            SERVER_MODE,
            '--',
            '--host',
            '127.0.0.1',
            '--port',
            String(PORT),
            '--strictPort',
          ],
          { cwd: ROOT, detached: true, stdio: 'ignore' }
        )
  let browser
  let largePage
  const progress = {
    completed_cases: [],
    axe: [],
    screenshots: [],
    resource_metrics: null,
  }
  try {
    await waitForServer()
    browser = await chromium.launch({
      headless: true,
      args: ['--enable-precise-memory-info'],
    })
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      reducedMotion: 'reduce',
    })
    const page = await context.newPage()
    const consoleErrors = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text().slice(0, 500))
    })
    const navigationStartedAt = Date.now()
    await page.goto(BASE_URL, { waitUntil: 'networkidle' })
    const navigationMs = Date.now() - navigationStartedAt

    await waitForInitialKeyboardTarget(page)
    await page.keyboard.press('Tab')
    ensure(
      (await page.locator(':focus').getAttribute('href')) === '#main-content',
      'first keyboard stop is not the main-content skip link'
    )
    await page.keyboard.press('Enter')
    ensure(new URL(page.url()).hash === '#main-content', 'skip link did not reach main content')
    const knowledgeAxe = await assertAxe(page, 'knowledge')
    progress.axe.push(knowledgeAxe)
    progress.completed_cases.push('axe-knowledge')

    const workspaceSwitcher = page.getByRole('button', { name: 'Switch workspace' })
    await workspaceSwitcher.click()
    await page.getByRole('menuitemradio', { name: 'Work', exact: true }).click()
    await page.waitForFunction(() =>
      document.querySelector('.document-explorer-heading strong')?.textContent?.includes('Work')
    )
    const workSource = page.getByRole('button', { name: /^work-code / })
    await workSource.waitFor()
    ensure(
      (await page.getByRole('button', { name: /^personal-notes / }).count()) === 0,
      'workspace switch left a source from the previous workspace in the tree'
    )
    progress.completed_cases.push('workspace-switching')

    await workSource.click()
    await page.waitForFunction(() =>
      document
        .querySelector('.document-explorer-heading strong')
        ?.textContent?.includes('Files & code')
    )
    await page.getByRole('option', { name: /Deployment playbook/ }).waitFor()
    ensure(
      (await page.getByRole('option', { name: /Deployment playbook/ }).count()) === 1,
      'source selection did not scope the document list to the selected source'
    )
    progress.completed_cases.push('source-tree-scoping')

    await workSource.click()
    const documentFilter = page.getByRole('textbox', { name: 'Filter documents' })
    await documentFilter.fill('Slack')
    await page.waitForFunction(
      () => document.querySelectorAll('[data-m7-document-row]').length === 1
    )
    ensure(
      await page.getByRole('option', { name: /Slack: #releases/ }).isVisible(),
      'document filter did not return the matching canonical document'
    )
    await documentFilter.fill('')
    await page.getByRole('option', { name: /Deployment playbook/ }).waitFor()
    progress.completed_cases.push('document-filtering')

    const documentExplorer = page.getByRole('region', { name: 'Document explorer' })
    const documentList = page.getByRole('listbox', { name: 'Documents' })
    const canonicalDocument = page.locator('article.canonical-document')
    await documentExplorer.waitFor()
    await documentList.waitFor()
    const documentRow = documentList.locator('[data-m7-document-row]').first()
    await documentRow.waitFor()
    await documentList.focus()
    const activeDocumentId = await documentList.getAttribute('aria-activedescendant')
    const firstDocumentId = await documentRow.getAttribute('id')
    ensure(
      Boolean(activeDocumentId && firstDocumentId && activeDocumentId === firstDocumentId),
      'document list does not expose a non-empty active keyboard option'
    )
    const documentSamples = []
    const firstDocumentOpenMs = await measureInteraction(page, async () => {
      await page.keyboard.press('Enter')
      await canonicalDocument.waitFor()
    })
    documentSamples.push({
      document_open_ms: firstDocumentOpenMs,
      resource: await readBrowserResourceSnapshot(page),
    })
    const sourceLink = page.getByRole('link', { name: 'Open original source' })
    ensure(await sourceLink.isVisible(), 'canonical document source link is missing')
    ensure(
      (await sourceLink.getAttribute('href'))?.startsWith('https://example.test/'),
      'canonical document source link is not provenance-safe'
    )
    const documentRowCount = await documentList.locator('[data-m7-document-row]').count()
    for (let index = 1; index < Math.min(3, documentRowCount); index += 1) {
      const documentOpenMs = await measureInteraction(page, async () => {
        await documentList.locator('[data-m7-document-row]').nth(index).click()
        await canonicalDocument.waitFor()
      })
      documentSamples.push({
        document_open_ms: documentOpenMs,
        resource: await readBrowserResourceSnapshot(page),
      })
    }
    await canonicalDocument.waitFor()
    ensure(await canonicalDocument.locator('h1').isVisible(), 'canonical document title is missing')
    ensure(
      await page.getByLabel('Document security and provenance').isVisible(),
      'canonical document provenance labels are missing'
    )
    ensure(
      await page.getByRole('group', { name: 'Document copy actions' }).isVisible(),
      'canonical document actions are missing'
    )
    await page.getByRole('option', { name: /Deployment playbook/ }).click()
    await page
      .getByRole('heading', { level: 1, name: 'Deployment playbook', exact: true })
      .waitFor()
    ensure(
      await page.getByRole('heading', { name: 'Backlinks', exact: true }).isVisible(),
      'canonical document backlinks are missing'
    )
    ensure(
      await page.getByRole('heading', { name: 'Surrounding documents', exact: true }).isVisible(),
      'canonical document surrounding context is missing'
    )
    const relatedDocument = page.getByRole('button', { name: /^How do releases work\?/ })
    await relatedDocument.click()
    await page
      .getByRole('heading', { level: 1, name: 'How do releases work?', exact: true })
      .waitFor()
    progress.completed_cases.push('document-relations')
    const documentAxe = await assertAxe(page, 'document')
    progress.axe.push(documentAxe)
    progress.completed_cases.push('axe-document')
    for (const screenshot of DOCUMENT_SCREENSHOTS) {
      await setViewportAndWaitForLayout(page, {
        width: screenshot.width,
        height: screenshot.height,
      })
      ensure(
        await canonicalDocument.locator('h1').isVisible(),
        `canonical document title disappeared at ${screenshot.width}px`
      )
      ensure(
        await page.getByLabel('Document security and provenance').isVisible(),
        `document provenance disappeared at ${screenshot.width}px`
      )
      ensure(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= document.documentElement.clientWidth
        ),
        `document browser requires horizontal page scrolling at ${screenshot.width}px`
      )
      await page.screenshot({
        path: resolve(EVIDENCE_DIRECTORY, screenshot.file),
        fullPage: true,
      })
      progress.screenshots.push({ surface: 'document', ...screenshot })
    }
    await setViewportAndWaitForLayout(page, { width: 1440, height: 900 })

    const graphButton = page.getByRole('button', { name: 'Graph', exact: true })
    const graphNode = page.getByRole('button', { name: /^Open document:/ }).first()
    const selectedGraph = page.getByRole('complementary', { name: 'Selected graph node' })
    const graphSamples = []
    const graphSampleCount = Math.max(1, Math.min(3, documentSamples.length))
    for (let index = 0; index < graphSampleCount; index += 1) {
      if (index > 0) {
        await page.getByRole('button', { name: 'Knowledge', exact: true }).click()
        await documentExplorer.waitFor()
      }
      const graphOpenMs = await measureInteraction(page, async () => {
        await graphButton.focus()
        await page.keyboard.press('Enter')
        await graphNode.waitFor()
      })
      const graphNodes = page.getByRole('button', { name: /^Open document:/ })
      const graphNodeCount = await graphNodes.count()
      ensure(graphNodeCount > 0, 'graph contains no document nodes to measure')
      const graphSelectionMs = await measureInteraction(page, async () => {
        await graphNodes.nth(index % graphNodeCount).focus()
        await page.keyboard.press('Enter')
        await selectedGraph.waitFor()
      })
      graphSamples.push({
        graph_open_ms: graphOpenMs,
        graph_selection_ms: graphSelectionMs,
        resource: await readBrowserResourceSnapshot(page),
      })
    }
    const browserResourceSamples = graphSamples.map((graphSample, index) => ({
      navigation_ms: navigationMs,
      document_open_ms: documentSamples[index % documentSamples.length].document_open_ms,
      graph_open_ms: graphSample.graph_open_ms,
      graph_selection_ms: graphSample.graph_selection_ms,
      ...mergeResourceSnapshots(
        documentSamples[index % documentSamples.length].resource,
        graphSample.resource
      ),
    }))
    let browserResources = summarizeBrowserResourceSamples(browserResourceSamples)
    progress.resource_metrics = browserResources
    ensure(
      browserResources.status === 'passed',
      `browser resource budgets failed: ${browserResources.failures.join('; ')}`
    )
    ensure(await page.getByLabel('Filter graph nodes').isVisible(), 'graph node filter is missing')
    ensure(
      await page.getByLabel('Filter graph relationships').isVisible(),
      'graph relationship filter is missing'
    )
    const graphAxe = await assertAxe(page, 'graph')
    progress.axe.push(graphAxe)
    progress.completed_cases.push('axe-graph')
    const transitionDuration = await graphNode.evaluate(
      (element) => getComputedStyle(element).transitionDuration
    )
    ensure(
      transitionDuration.split(',').every((duration) => Number.parseFloat(duration) <= 0.001),
      `graph motion ignored reduced-motion: ${transitionDuration}`
    )
    const [desktopScreenshot, ...responsiveScreenshots] = RESPONSIVE_SCREENSHOTS
    ensure(desktopScreenshot.width === 1440, 'desktop screenshot matrix must begin at 1440px')
    await page.screenshot({
      path: resolve(EVIDENCE_DIRECTORY, desktopScreenshot.file),
      fullPage: true,
    })
    progress.screenshots.push({ surface: 'graph', ...desktopScreenshot })

    const graphFilterLabels = [
      'Filter graph nodes',
      'Filter graph relationships',
      'Filter graph relationship origin',
      'Filter graph minimum confidence',
    ]
    for (const screenshot of responsiveScreenshots) {
      await setViewportAndWaitForLayout(page, {
        width: screenshot.width,
        height: screenshot.height,
      })
      ensure(
        await graphNode.isVisible(),
        `selected graph node disappeared at ${screenshot.width}px`
      )
      ensure(
        await page.getByRole('complementary', { name: 'Selected graph node' }).isVisible(),
        `graph provenance panel disappeared at ${screenshot.width}px`
      )
      for (const label of graphFilterLabels) {
        await assertControlExposed(page, label, screenshot.width)
      }
      if (screenshot.width <= 720) {
        ensure(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= document.documentElement.clientWidth
          ),
          `knowledge graph requires horizontal page scrolling at ${screenshot.width}px`
        )
      }
      await page.screenshot({
        path: resolve(EVIDENCE_DIRECTORY, screenshot.file),
        fullPage: true,
      })
      progress.screenshots.push({ surface: 'graph', ...screenshot })
    }

    await setViewportAndWaitForLayout(page, { width: 1440, height: 900 })
    const selectedDocumentNode = page.getByRole('button', { name: /^Open document:/ }).first()
    await selectedDocumentNode.focus()
    await page.keyboard.press('Enter')
    await selectedGraph.waitFor()
    const expandRelationships = selectedGraph.getByRole('button', {
      name: 'Expand one-hop relationships',
      exact: true,
    })
    await expandRelationships.click()
    await page.getByRole('button', { name: 'Return to graph overview', exact: true }).waitFor()
    const graphBackButton = page.getByRole('button', { name: 'Back', exact: true })
    ensure(await graphBackButton.isEnabled(), 'graph focus history did not expose a back action')
    await graphBackButton.click()
    await graphNode.waitFor()
    ensure(
      (await page
        .getByRole('button', { name: 'Return to graph overview', exact: true })
        .count()) === 0,
      'graph overview did not restore after navigating back'
    )

    await page
      .getByRole('button', { name: /^Open document:/ })
      .first()
      .focus()
    await page.keyboard.press('Enter')
    await selectedGraph.waitFor()
    await selectedGraph.getByRole('button', { name: 'Open document', exact: true }).click()
    await canonicalDocument.waitFor()
    ensure(
      await canonicalDocument.locator('h1').isVisible(),
      'graph document navigation lost the document'
    )
    await graphButton.focus()
    await page.keyboard.press('Enter')
    await graphNode.waitFor()
    await page.getByRole('button', { name: 'Knowledge', exact: true }).click()
    await documentExplorer.waitFor()
    ensure(
      await documentExplorer.isVisible(),
      'knowledge workspace did not restore after collapsing the graph view'
    )
    await graphButton.focus()
    await page.keyboard.press('Enter')
    await graphNode.waitFor()

    if (RUN_LARGE_CORPUS) {
      largePage = await context.newPage()
      const largeConsoleErrors = []
      largePage.on('console', (message) => {
        if (message.type() === 'error') largeConsoleErrors.push(message.text().slice(0, 500))
      })
      const largeNavigationStartedAt = Date.now()
      await largePage.goto(LARGE_BASE_URL, { waitUntil: 'networkidle' })
      const largeNavigationMs = Date.now() - largeNavigationStartedAt
      const largeDocumentList = largePage.getByRole('listbox', { name: 'Documents' })
      await largeDocumentList.waitFor()
      const largeDocumentRows = largeDocumentList.locator('[data-m7-document-row]')
      await largeDocumentRows.first().waitFor()
      const initialLargeRows = await largeDocumentRows.count()
      ensure(initialLargeRows > 0 && initialLargeRows <= 100, 'large document list is not bounded')
      const largeExplorerHeading = largePage.locator('.document-explorer-heading')
      ensure(
        /50 loaded/.test(await largeExplorerHeading.textContent()),
        'large document fixture did not begin with a bounded page'
      )
      await largeDocumentList.evaluate((element) => {
        element.scrollTop = element.scrollHeight
        element.dispatchEvent(new Event('scroll', { bubbles: true }))
      })
      await largePage.waitForFunction(() =>
        document.querySelector('.document-explorer-heading')?.textContent?.includes('100 loaded')
      )
      const loadedLargeRows = await largeDocumentRows.count()
      ensure(
        loadedLargeRows > 0 && loadedLargeRows <= 100,
        'large document pagination rendered too many rows'
      )
      await largePage.screenshot({
        path: resolve(EVIDENCE_DIRECTORY, LARGE_CORPUS_SCREENSHOTS[0].file),
        fullPage: true,
      })
      progress.screenshots.push({
        surface: 'large-corpus-document',
        ...LARGE_CORPUS_SCREENSHOTS[0],
      })
      const largeDocumentOpenMs = await measureInteraction(largePage, async () => {
        await largeDocumentRows.first().click()
        await largePage.locator('article.canonical-document').waitFor()
      })
      ensure(
        await largePage.locator('article.canonical-document h1').isVisible(),
        'large document fixture did not open a canonical document'
      )
      const largeGraphButton = largePage.getByRole('button', { name: 'Graph', exact: true })
      const largeGraphNode = largePage.getByRole('button', { name: /^Open document:/ }).first()
      const largeSelectedGraph = largePage.getByRole('complementary', {
        name: 'Selected graph node',
      })
      const largeGraphOpenMs = await measureInteraction(largePage, async () => {
        await largeGraphButton.focus()
        await largePage.keyboard.press('Enter')
        await largeGraphNode.waitFor()
      })
      const initialLargeGraphNodes = await largePage.locator('.graph-node').count()
      ensure(
        initialLargeGraphNodes > 0 && initialLargeGraphNodes <= 200,
        'large graph initial DOM is not bounded'
      )
      const loadMoreNodes = largePage.getByRole('button', { name: 'Load more nodes', exact: true })
      ensure(await loadMoreNodes.isVisible(), 'large graph did not expose bounded pagination')
      const initialLargeGraphSummary = await largePage.locator('.graph-summary').textContent()
      await measureInteraction(largePage, async () => {
        await loadMoreNodes.click()
        await largePage.waitForFunction(
          (previous) => document.querySelector('.graph-summary')?.textContent !== previous,
          initialLargeGraphSummary
        )
      })
      const loadedLargeGraphNodes = await largePage.locator('.graph-node').count()
      ensure(
        loadedLargeGraphNodes > 0 && loadedLargeGraphNodes <= 200,
        'large graph pagination rendered too many nodes'
      )
      await largePage.screenshot({
        path: resolve(EVIDENCE_DIRECTORY, LARGE_CORPUS_SCREENSHOTS[1].file),
        fullPage: true,
      })
      progress.screenshots.push({
        surface: 'large-corpus-graph',
        ...LARGE_CORPUS_SCREENSHOTS[1],
      })
      const largeGraphSelectionMs = await measureInteraction(largePage, async () => {
        await largeGraphNode.focus()
        await largePage.keyboard.press('Enter')
        await largeSelectedGraph.waitFor()
      })
      browserResourceSamples.push({
        navigation_ms: largeNavigationMs,
        document_open_ms: largeDocumentOpenMs,
        graph_open_ms: largeGraphOpenMs,
        graph_selection_ms: largeGraphSelectionMs,
        ...(await readBrowserResourceSnapshot(largePage)),
      })
      ensure(
        largeConsoleErrors.length === 0,
        `large browser console errors: ${largeConsoleErrors.join('; ')}`
      )
    }
    browserResources = summarizeBrowserResourceSamples(browserResourceSamples)
    progress.resource_metrics = browserResources
    ensure(
      browserResources.status === 'passed',
      `browser resource budgets failed: ${browserResources.failures.join('; ')}`
    )
    ensure(consoleErrors.length === 0, `browser console errors: ${consoleErrors.join('; ')}`)

    const evidence = {
      schema_version: 1,
      status: 'passed',
      ...(ACCEPTANCE_CONFIG.target
        ? {
            target: describeKnowledgeAccessibilityTarget(ACCEPTANCE_CONFIG.target),
            installation_type: ACCEPTANCE_CONFIG.installationType,
          }
        : {}),
      version: EXPECTED_VERSION,
      revision: ACCEPTANCE_REVISION,
      fixture: RUN_LARGE_CORPUS
        ? 'provider-free-demo-and-large-corpus'
        : 'provider-free-release-demo',
      browser: 'chromium-headless',
      server_mode: SERVER_MODE,
      platform: process.platform,
      cases: [
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
        ...(RUN_LARGE_CORPUS ? ['large-corpus-bounded-rendering'] : []),
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
      ],
      axe: [knowledgeAxe, documentAxe, graphAxe],
      screenshots: [
        ...DOCUMENT_SCREENSHOTS.map(({ file }) => file),
        ...RESPONSIVE_SCREENSHOTS.map(({ file }) => file),
        ...(RUN_LARGE_CORPUS ? LARGE_CORPUS_SCREENSHOTS.map(({ file }) => file) : []),
      ],
      screenshot_matrix: [
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
        ...(RUN_LARGE_CORPUS
          ? LARGE_CORPUS_SCREENSHOTS.map(({ width, height, file }, index) => ({
              surface: index === 0 ? 'large-corpus-document' : 'large-corpus-graph',
              width,
              height,
              file,
            }))
          : []),
      ],
      resource_metrics: browserResources,
      limitation: [
        'Resource metrics are provider-free demo-fixture observations from Chromium and complement, but do not replace, approved large-corpus, signed package launch, and assistive-technology host review.',
        ...(RUN_LARGE_CORPUS
          ? []
          : [
              'This packaged release lane intentionally skips the large-corpus fixture because the immutable release bundle predates that fixture.',
            ]),
      ].join(' '),
      generated_at: new Date().toISOString(),
    }
    writeFileSync(
      resolve(EVIDENCE_DIRECTORY, 'report.json'),
      `${JSON.stringify(evidence, null, 2)}\n`
    )
    console.log(`knowledge accessibility acceptance passed: ${EVIDENCE_DIRECTORY}`)
  } catch (error) {
    const failure = buildKnowledgeAcceptanceFailureEvidence({
      target: ACCEPTANCE_CONFIG.target,
      version: EXPECTED_VERSION,
      revision: ACCEPTANCE_REVISION,
      serverMode: SERVER_MODE,
      fixture: RUN_LARGE_CORPUS
        ? 'provider-free-demo-and-large-corpus'
        : 'provider-free-release-demo',
      installationType: ACCEPTANCE_CONFIG.installationType,
      error,
      progress,
    })
    writeFileSync(
      resolve(EVIDENCE_DIRECTORY, 'report.json'),
      `${JSON.stringify(failure, null, 2)}\n`
    )
    throw error
  } finally {
    await largePage?.close()
    await browser?.close()
    await stopServer(server)
  }
}

await run()
