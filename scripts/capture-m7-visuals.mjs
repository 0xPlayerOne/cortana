#!/usr/bin/env node

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'

import AxeBuilder from '@axe-core/playwright'
import { chromium } from 'playwright'

const args = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  args.set(process.argv[index], process.argv[index + 1])
}

const renderer = args.get('--renderer') ?? 'legacy'
if (!['legacy', 'shadcn'].includes(renderer)) {
  throw new Error('--renderer must be legacy or shadcn')
}

const baseUrl = args.get('--base-url') ?? 'http://127.0.0.1:4173'
const output = resolve(args.get('--output') ?? `artifacts/m7-shadcn/${renderer}`)
const widths = [320, 768, 1024, 1440]
const themes = ['blue', 'accessible', 'forest', 'plum']
const consoleErrors = []
let screenshotCount = 0

await mkdir(output, { recursive: true })
const browser = await chromium.launch({ headless: true })

async function openPage(theme, width) {
  const context = await browser.newContext({ viewport: { width, height: 1000 } })
  await context.addInitScript((value) => localStorage.setItem('cortana.theme.v1', value), theme)
  const page = await context.newPage()
  page.setDefaultTimeout(60_000)
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(`${renderer}/${theme}/${width}: ${message.text()}`)
    }
  })
  const query = renderer === 'shadcn' ? '?demo=1&renderer=shadcn' : '?demo=1'
  await page.goto(`${baseUrl}/${query}`, { waitUntil: 'domcontentloaded' })
  if (renderer === 'shadcn') {
    await page.getByRole('heading', { name: 'Release evidence' }).waitFor()
    await page.locator('[data-m7-header-overlays-ready]').waitFor({ state: 'attached' })
    if (width > 768) {
      await page.locator('[data-m7-workspace-overlays-ready]').waitFor({ state: 'attached' })
    }
    if (!new URL(page.url()).searchParams.has('prototypeState')) {
      await page.locator('[data-m7-evidence-overlays-ready]').waitFor({ state: 'attached' })
    }
  } else {
    await page.getByRole('main').waitFor()
  }
  return { context, page }
}

async function screenshot(page, name) {
  await page.screenshot({
    path: resolve(output, `${name}.png`),
    animations: 'disabled',
  })
  screenshotCount += 1
}

async function auditAccessibility(page, label) {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze()
  if (accessibility.violations.length > 0) {
    const summary = accessibility.violations
      .map(
        (violation) =>
          `${violation.id}: ${violation.help}\n${violation.nodes
            .map((node) => `  ${node.target.join(' ')}: ${node.failureSummary}`)
            .join('\n')}`
      )
      .join('\n')
    throw new Error(`Accessibility violations in ${label}:\n${summary}`)
  }
}

if (renderer === 'shadcn') {
  for (const theme of themes) {
    for (const width of widths) {
      const { context, page } = await openPage(theme, width)
      await screenshot(page, `shell-${theme}-${width}`)
      await auditAccessibility(page, `${theme}/${width}`)
      if (theme === 'blue' && width === 320) {
        const navigationTrigger = page.getByRole('button', { name: 'Toggle navigation' })
        const navigationElement = await navigationTrigger.elementHandle()
        if (!navigationElement) throw new Error('Mobile navigation trigger was not rendered')
        await navigationTrigger.focus()
        const focusStyle = await navigationTrigger.evaluate((element) => {
          const style = getComputedStyle(element)
          return { boxShadow: style.boxShadow, outlineStyle: style.outlineStyle }
        })
        if (focusStyle.boxShadow === 'none' && focusStyle.outlineStyle === 'none') {
          throw new Error('Mobile navigation trigger has no visible focus indicator')
        }
        await navigationTrigger.press('Enter')
        await page.getByText('Settings', { exact: true }).last().waitFor()
        await page.locator('[data-m7-workspace-overlays-ready]').waitFor({ state: 'attached' })
        await page.waitForTimeout(250)
        await auditAccessibility(page, 'mobile navigation overlay')
        await screenshot(page, 'mobile-navigation-blue-320')
        await page.keyboard.press('Escape')
        await page.locator('[data-slot="sheet-content"]').waitFor({ state: 'detached' })
        await page.waitForFunction(() =>
          document.activeElement?.matches('[data-sidebar="trigger"]')
        )
      }
      if (theme === 'blue' && width === 1440) {
        await page.getByRole('tab', { name: 'Answer' }).click()
        await page
          .getByText('Synthesized answers will compose the same evidence cards and citations.')
          .waitFor()
        await page.getByRole('button', { name: 'Review context boundary' }).click()
        await page.getByRole('dialog').waitFor()
        await page.waitForTimeout(150)
        await auditAccessibility(page, 'context boundary dialog')
        await screenshot(page, 'dialog-blue-1440')
        await page.keyboard.press('Escape')
        await page.getByRole('dialog').waitFor({ state: 'detached' })

        await page.getByRole('button', { name: 'Open command palette' }).click()
        await page.getByRole('dialog', { name: 'Command palette' }).waitFor()
        await page.waitForTimeout(150)
        await auditAccessibility(page, 'command palette')
        await screenshot(page, 'command-blue-1440')
        await page.keyboard.press('Escape')
        await page.getByRole('dialog', { name: 'Command palette' }).waitFor({ state: 'detached' })

        await page.getByRole('button', { name: 'Filter evidence' }).click()
        await page
          .getByText('Narrow the visible evidence without expanding retrieval scope.')
          .waitFor()
        await page.waitForTimeout(150)
        await auditAccessibility(page, 'filter popover')
        await screenshot(page, 'filter-popover-blue-1440')
        await page.keyboard.press('Escape')

        await page
          .getByRole('button', { name: 'Switch workspace. Current workspace: Personal' })
          .click()
        await page.getByRole('menuitem', { name: 'Product' }).waitFor()
        await page.waitForTimeout(150)
        await auditAccessibility(page, 'workspace menu')
        await screenshot(page, 'workspace-menu-blue-1440')
        await page.keyboard.press('Escape')

        await page.getByRole('tab', { name: 'Document' }).click()
        await page.getByText('How do releases work?').click({ button: 'right' })
        await page.getByRole('menuitem', { name: 'Copy citation' }).waitFor()
        await page.waitForTimeout(150)
        await auditAccessibility(page, 'evidence context menu')
        await screenshot(page, 'context-menu-blue-1440')
      }
      await context.close()
    }
  }

  for (const state of ['loading', 'empty', 'error']) {
    const { context, page } = await openPage('blue', 1024)
    await page.goto(`${baseUrl}/?demo=1&renderer=shadcn&prototypeState=${state}`, {
      waitUntil: 'domcontentloaded',
    })
    await page.getByRole('heading', { name: 'Release evidence' }).waitFor()
    await Promise.all([
      page.locator('[data-m7-workspace-overlays-ready]').waitFor({ state: 'attached' }),
      page.locator('[data-m7-header-overlays-ready]').waitFor({ state: 'attached' }),
    ])
    await auditAccessibility(page, `${state} retrieval state`)
    await screenshot(page, `retrieval-${state}-blue-1024`)
    await context.close()
  }

  // A 1440-physical-pixel window at 200% browser zoom exposes a 720 CSS-pixel
  // layout viewport. Model that reflow and density together.
  const zoomContext = await browser.newContext({
    viewport: { width: 720, height: 500 },
    deviceScaleFactor: 2,
  })
  const zoomPage = await zoomContext.newPage()
  await zoomPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, { waitUntil: 'domcontentloaded' })
  await zoomPage.getByRole('heading', { name: 'Release evidence' }).waitFor()
  const horizontalOverflow = await zoomPage.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth
  )
  if (horizontalOverflow) throw new Error('The shadcn prototype overflows at the 200% zoom proxy')
  await zoomContext.close()

  const motionContext = await browser.newContext({
    viewport: { width: 1024, height: 800 },
    reducedMotion: 'reduce',
  })
  const motionPage = await motionContext.newPage()
  await motionPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, {
    waitUntil: 'domcontentloaded',
  })
  await motionPage.getByRole('heading', { name: 'Release evidence' }).waitFor()
  await Promise.all([
    motionPage.locator('[data-m7-workspace-overlays-ready]').waitFor({ state: 'attached' }),
    motionPage.locator('[data-m7-header-overlays-ready]').waitFor({ state: 'attached' }),
    motionPage.locator('[data-m7-evidence-overlays-ready]').waitFor({ state: 'attached' }),
  ])
  const reviewButton = motionPage.getByRole('button', { name: 'Review context boundary' })
  const reviewElement = await reviewButton.elementHandle()
  if (!reviewElement) throw new Error('Context boundary trigger was not rendered')
  await reviewButton.focus()
  await reviewButton.press('Enter')
  const contextDialog = motionPage.getByRole('dialog')
  await contextDialog.waitFor()
  const animationDuration = await contextDialog.evaluate(
    (element) => getComputedStyle(element).animationDuration
  )
  if (Number.parseFloat(animationDuration) > 0.00002) {
    throw new Error(`Reduced-motion dialog animation remained ${animationDuration}`)
  }
  await motionPage.keyboard.press('Escape')
  await contextDialog.waitFor({ state: 'detached' })
  await motionPage.waitForFunction((element) => element === document.activeElement, reviewElement)
  await motionContext.close()
} else {
  const labels = {
    inbox: 'Inbox',
    conversations: 'Conversations',
    'agent-tools': 'Agent tools',
    index: 'Index',
    settings: 'Settings',
    help: 'Help',
  }
  for (const [surface, label] of Object.entries(labels)) {
    const { context, page } = await openPage('blue', 1440)
    await page.getByRole('button', { name: label, exact: true }).click()
    await screenshot(page, `${surface}-blue-1440`)
    await context.close()
  }

  for (const theme of themes) {
    for (const width of widths) {
      // Secondary themes are required at compact and desktop widths only.
      if (['forest', 'plum'].includes(theme) && ![320, 1024, 1440].includes(width)) continue
      const { context, page } = await openPage(theme, width)
      await screenshot(page, `knowledge-${theme}-${width}`)

      await page.keyboard.press('Control+p')
      await page.getByRole('dialog', { name: 'Command palette' }).waitFor()
      await screenshot(page, `command-${theme}-${width}`)
      await page.keyboard.press('Escape')

      if (width <= 768) {
        await page.getByRole('button', { name: 'Open sources', exact: true }).click()
        await screenshot(page, `source-sheet-${theme}-${width}`)
      } else {
        for (const surface of ['settings', 'graph']) {
          await page.goto(`${baseUrl}/?demo=1`, { waitUntil: 'domcontentloaded' })
          await page
            .getByRole('button', {
              name: surface[0].toUpperCase() + surface.slice(1),
              exact: true,
            })
            .click()
          await screenshot(page, `${surface}-${theme}-${width}`)
        }
      }
      await context.close()
    }
  }
}

await browser.close()

if (consoleErrors.length > 0) {
  throw new Error(`Browser console errors:\n${consoleErrors.join('\n')}`)
}

console.log(`Captured ${screenshotCount} ${renderer} screenshots in ${output}`)
