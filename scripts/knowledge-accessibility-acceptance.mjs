#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import AxeBuilder from '@axe-core/playwright'
import { chromium } from 'playwright'

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const PORT = 4183
const BASE_URL = `http://127.0.0.1:${PORT}/?demo=1`
const EVIDENCE_DIRECTORY = resolve(ROOT, 'artifacts/knowledge-accessibility')

function ensure(condition, message) {
  if (!condition) throw new Error(message)
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
  ensure(
    await control.evaluate((element) => {
      const box = element.getBoundingClientRect()
      const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)
      return hit === element || element.contains(hit)
    }),
    `${label} is obscured by another graph control`
  )
}

async function run() {
  mkdirSync(EVIDENCE_DIRECTORY, { recursive: true })
  const server = spawn(
    'bun',
    [
      'run',
      '--cwd',
      'apps/web',
      'dev',
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
  try {
    await waitForServer()
    browser = await chromium.launch({ headless: true })
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      reducedMotion: 'reduce',
    })
    const page = await context.newPage()
    const consoleErrors = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text().slice(0, 500))
    })
    await page.goto(BASE_URL, { waitUntil: 'networkidle' })

    await page.keyboard.press('Tab')
    ensure(
      (await page.locator(':focus').getAttribute('href')) === '#main-content',
      'first keyboard stop is not the main-content skip link'
    )
    await page.keyboard.press('Enter')
    ensure(new URL(page.url()).hash === '#main-content', 'skip link did not reach main content')
    const knowledgeAxe = await assertAxe(page, 'knowledge')

    const graphButton = page.getByRole('button', { name: 'Graph', exact: true })
    await graphButton.focus()
    await page.keyboard.press('Enter')
    await page
      .getByRole('button', { name: /^Open document:/ })
      .first()
      .waitFor()
    const graphNode = page.getByRole('button', { name: /^Open document:/ }).first()
    await graphNode.focus()
    await page.keyboard.press('Enter')
    await page.getByRole('complementary', { name: 'Selected graph node' }).waitFor()
    ensure(await page.getByLabel('Filter graph nodes').isVisible(), 'graph node filter is missing')
    ensure(
      await page.getByLabel('Filter graph relationships').isVisible(),
      'graph relationship filter is missing'
    )
    const graphAxe = await assertAxe(page, 'graph')
    const transitionDuration = await graphNode.evaluate(
      (element) => getComputedStyle(element).transitionDuration
    )
    ensure(
      transitionDuration.split(',').every((duration) => Number.parseFloat(duration) <= 0.001),
      `graph motion ignored reduced-motion: ${transitionDuration}`
    )
    await page.screenshot({
      path: resolve(EVIDENCE_DIRECTORY, 'graph-desktop-reduced-motion.png'),
      fullPage: true,
    })

    await page.setViewportSize({ width: 720, height: 900 })
    ensure(await graphNode.isVisible(), 'selected graph node disappeared at 200% zoom')
    ensure(
      await page.getByRole('complementary', { name: 'Selected graph node' }).isVisible(),
      'graph provenance panel disappeared at 200% zoom'
    )
    for (const label of [
      'Filter graph nodes',
      'Filter graph relationships',
      'Filter graph relationship origin',
      'Filter graph minimum confidence',
    ]) {
      await assertControlExposed(page, label, 720)
    }
    ensure(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= document.documentElement.clientWidth
      ),
      'knowledge graph requires horizontal page scrolling at 200% equivalent reflow'
    )
    await page.screenshot({
      path: resolve(EVIDENCE_DIRECTORY, 'graph-desktop-200-percent.png'),
      fullPage: true,
    })

    await page.setViewportSize({ width: 390, height: 844 })
    ensure(await graphNode.isVisible(), 'graph is not responsive at mobile width')
    for (const label of [
      'Filter graph nodes',
      'Filter graph relationships',
      'Filter graph relationship origin',
      'Filter graph minimum confidence',
    ]) {
      await assertControlExposed(page, label, 390)
    }
    await page.screenshot({
      path: resolve(EVIDENCE_DIRECTORY, 'graph-mobile.png'),
      fullPage: true,
    })
    ensure(consoleErrors.length === 0, `browser console errors: ${consoleErrors.join('; ')}`)

    const evidence = {
      schema_version: 1,
      status: 'passed',
      fixture: 'provider-free-demo',
      browser: 'chromium-headless',
      platform: process.platform,
      cases: [
        'axe-wcag-2.2-aa',
        'keyboard-skip-link',
        'keyboard-graph-open',
        'keyboard-node-selection',
        'graph-filter-labels',
        'selection-live-region',
        'reduced-motion',
        'zoom-200-percent-reflow',
        'responsive-mobile',
        'browser-console-clean',
      ],
      axe: [knowledgeAxe, graphAxe],
      screenshots: [
        'graph-desktop-reduced-motion.png',
        'graph-desktop-200-percent.png',
        'graph-mobile.png',
      ],
      limitation:
        'This renderer gate complements, but does not replace, signed package launch and assistive-technology host review.',
      generated_at: new Date().toISOString(),
    }
    writeFileSync(
      resolve(EVIDENCE_DIRECTORY, 'report.json'),
      `${JSON.stringify(evidence, null, 2)}\n`
    )
    console.log(`knowledge accessibility acceptance passed: ${EVIDENCE_DIRECTORY}`)
  } finally {
    await browser?.close()
    await stopServer(server)
  }
}

await run()
