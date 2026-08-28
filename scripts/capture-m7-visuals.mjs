#!/usr/bin/env node

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'

import AxeBuilder from '@axe-core/playwright'
import { chromium } from 'playwright'

const args = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  args.set(process.argv[index], process.argv[index + 1])
}

const baseUrl = args.get('--base-url') ?? 'http://127.0.0.1:4173'
const output = resolve(args.get('--output') ?? 'artifacts/m7-shadcn/final')
const widths = [320, 768, 1024, 1440, 1920]
const themes = ['blue', 'accessible', 'forest', 'plum']
const consoleErrors = []
let screenshotCount = 0

await mkdir(output, { recursive: true })
const browser = await chromium.launch({ headless: true })

async function openPage(theme, width, state = 'configured') {
  const context = await browser.newContext({ viewport: { width, height: 1000 } })
  await context.addInitScript((value) => localStorage.setItem('cortana.theme.v1', value), theme)
  const page = await context.newPage()
  page.setDefaultTimeout(60_000)
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(`${theme}/${width}: ${message.text()}`)
    }
  })
  await page.goto(`${baseUrl}/?demo=1&demo-state=${state}`, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-m7-production-shell-ready]').waitFor({ state: 'attached' })
  await page.getByRole('textbox', { name: 'Search your knowledge' }).waitFor()
  return { context, page }
}

async function openSettings(page, width) {
  if (width <= 768) {
    await page.getByRole('button', { name: 'Toggle navigation' }).click()
    await page.locator('[data-mobile="true"]').waitFor()
  }
  await page.getByRole('button', { name: 'Settings', exact: true }).click()
  if (width <= 768) await page.locator('[data-mobile="true"]').waitFor({ state: 'detached' })
  await page.locator('.settings-view').waitFor()
  await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor()
}

async function openDestination(page, width, destination) {
  if (width <= 768) {
    await page.getByRole('button', { name: 'Toggle navigation' }).click()
    await page.locator('[data-mobile="true"]').waitFor()
  }
  await page.getByRole('button', { name: destination, exact: true }).click()
  if (width <= 768) await page.locator('[data-mobile="true"]').waitFor({ state: 'detached' })
  await page.getByRole('heading', { name: destination, level: 1 }).waitFor()
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

{
  for (const theme of themes) {
    for (const width of widths) {
      const { context, page } = await openPage(theme, width)
      await screenshot(page, `shell-${theme}-${width}`)
      await auditAccessibility(page, `${theme}/${width} production shell`)

      if (theme === 'blue' && width === 320) {
        const mobileSearch = page.getByRole('textbox', { name: 'Search your knowledge' })
        await mobileSearch.fill('How do releases work?')
        await mobileSearch.press('Enter')
        await page.getByRole('heading', { name: 'How do releases work?', level: 1 }).waitFor()
        await auditAccessibility(page, 'mobile populated knowledge answer')
        await screenshot(page, 'knowledge-answer-blue-320')

        const navigationTrigger = page.getByRole('button', { name: 'Toggle navigation' })
        await navigationTrigger.click()
        await page.locator('[data-mobile="true"]').waitFor()
        await page.getByRole('button', { name: 'Conversations', exact: true }).click()
        await page.locator('[data-mobile="true"]').waitFor({ state: 'detached' })
        await page.getByRole('heading', { name: 'Conversations', level: 1 }).waitFor()
        await navigationTrigger.click()
        await page.locator('[data-mobile="true"]').waitFor()
        await page.getByRole('button', { name: 'Knowledge', exact: true }).click()
        await page.getByRole('heading', { name: 'How do releases work?', level: 1 }).waitFor()

        await page.evaluate(() => {
          if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
        })
        for (let index = 0; index < 80; index += 1) {
          await page.keyboard.press('Tab')
          if (await navigationTrigger.evaluate((element) => element === document.activeElement)) {
            break
          }
        }
        if (!(await navigationTrigger.evaluate((element) => element === document.activeElement))) {
          throw new Error('Mobile navigation trigger was not reachable by keyboard')
        }
        const focusStyle = await navigationTrigger.evaluate((element) => {
          const style = getComputedStyle(element)
          return { boxShadow: style.boxShadow, outlineStyle: style.outlineStyle }
        })
        if (focusStyle.boxShadow === 'none' && focusStyle.outlineStyle === 'none') {
          throw new Error('Mobile navigation trigger has no visible focus indicator')
        }
        await navigationTrigger.press('Enter')
        await page.locator('[data-mobile="true"]').waitFor()
        await page.getByRole('button', { name: 'Settings', exact: true }).waitFor()
        await page.waitForTimeout(300)
        await auditAccessibility(page, 'mobile production navigation')
        await screenshot(page, 'mobile-navigation-blue-320')
        await page.getByRole('button', { name: 'Knowledge', exact: true }).click()
        await page.locator('[data-mobile="true"]').waitFor({ state: 'detached' })
      }

      if (theme === 'blue' && width === 768) {
        await page.getByRole('button', { name: 'Actions' }).click()
        await page.getByRole('menuitem', { name: 'Open sources' }).click()
        await page.getByRole('dialog', { name: 'Sources and documents' }).waitFor()
        await page.locator('aside.source-panel.mobile-open').waitFor()
        await screenshot(page, 'source-panel-blue-768')
        await page.keyboard.press('Escape')
        await page.getByRole('dialog', { name: 'Sources and documents' }).waitFor({
          state: 'detached',
        })
        await page.waitForFunction(
          () => document.activeElement?.getAttribute('aria-label') === 'Actions'
        )
      }

      if (theme === 'blue' && width === 1024) {
        await page.getByRole('button', { name: 'Actions' }).click()
        await page.getByRole('menuitem', { name: 'Open agent context' }).click()
        await page.getByRole('dialog', { name: 'Agent context' }).waitFor()
        await page.waitForTimeout(300)
        await auditAccessibility(page, 'tablet agent context boundary')
        await screenshot(page, 'context-panel-blue-1024')
        await page.keyboard.press('Escape')
        await page.getByRole('dialog', { name: 'Agent context' }).waitFor({ state: 'detached' })
        await page.waitForFunction(
          () => document.activeElement?.getAttribute('aria-label') === 'Actions'
        )
      }

      if (theme === 'blue' && width === 1440) {
        const collapsedMetrics = await page.evaluate(() => {
          const sidebar = document.querySelector('[data-slot="sidebar-container"]')
          const destination = Array.from(
            document.querySelectorAll(
              '[data-slot="sidebar"][data-state="collapsed"] [data-sidebar="menu-button"]'
            )
          ).find((element) => element.textContent?.trim() === 'Knowledge')
          const icon = destination?.querySelector('svg')
          const logo = document.querySelector('[aria-label="Switch workspace"] .workspace-logo')
          const labels = document.querySelector('[data-workspace-labels]')
          return {
            sidebarWidth: sidebar?.getBoundingClientRect().width ?? 0,
            targetWidth: destination?.getBoundingClientRect().width ?? 0,
            iconWidth: icon?.getBoundingClientRect().width ?? 0,
            logoWidth: logo?.getBoundingClientRect().width ?? 0,
            labelsDisplay: labels ? getComputedStyle(labels).display : 'missing',
          }
        })
        if (
          Math.abs(collapsedMetrics.sidebarWidth - 72) > 1 ||
          Math.abs(collapsedMetrics.targetWidth - 48) > 1 ||
          Math.abs(collapsedMetrics.iconWidth - 24) > 1 ||
          Math.abs(collapsedMetrics.logoWidth - 72) > 1 ||
          collapsedMetrics.labelsDisplay !== 'none'
        ) {
          throw new Error(
            `Collapsed sidebar proportions regressed: ${JSON.stringify(collapsedMetrics)}`
          )
        }

        await page.getByRole('button', { name: 'Toggle navigation' }).click()
        await page.locator('[data-slot="sidebar"][data-state="expanded"]').waitFor()
        await page.waitForTimeout(300)
        const expandedLogoWidth = await page
          .locator('[aria-label="Switch workspace"] .workspace-logo')
          .evaluate((element) => element.getBoundingClientRect().width)
        if (Math.abs(expandedLogoWidth - 72) > 1) {
          throw new Error(`Expanded workspace logo regressed to ${expandedLogoWidth}px`)
        }
        await auditAccessibility(page, 'expanded desktop navigation')
        await screenshot(page, 'sidebar-expanded-blue-1440')
        await page.getByRole('button', { name: 'Toggle navigation' }).click()
        await page.locator('[data-slot="sidebar"][data-state="collapsed"]').waitFor()

        const knowledgeSearch = page.getByRole('textbox', { name: 'Search your knowledge' })
        await knowledgeSearch.fill('How do releases work?')
        await knowledgeSearch.press('Enter')
        await page.getByRole('heading', { name: 'How do releases work?', level: 1 }).waitFor()
        await auditAccessibility(page, 'populated knowledge answer')
        await screenshot(page, 'knowledge-answer-blue-1440')

        await page.getByRole('button', { name: 'Conversations', exact: true }).click()
        await page.getByRole('heading', { name: 'Conversations', level: 1 }).waitFor()
        await auditAccessibility(page, 'populated conversations')
        await screenshot(page, 'conversations-blue-1440')
        await page.getByRole('button', { name: 'Knowledge', exact: true }).click()

        await page.getByRole('tab', { name: /Evidence/ }).click()
        await page.getByRole('heading', { name: 'How do releases work?', level: 1 }).waitFor()
        await screenshot(page, 'knowledge-evidence-blue-1440')

        await page.getByRole('tab', { name: 'Timeline' }).click()
        await page.locator('.timeline-view').waitFor()
        await screenshot(page, 'knowledge-timeline-blue-1440')

        await page.getByRole('option').first().click()
        await page.locator('.canonical-document').waitFor()
        await screenshot(page, 'knowledge-document-blue-1440')

        await page.getByRole('button', { name: 'Graph', exact: true }).click()
        await page.locator('.graph-view').waitFor()
        await auditAccessibility(page, 'bounded knowledge graph')
        await screenshot(page, 'knowledge-graph-blue-1440')

        await page.getByRole('button', { name: 'Knowledge', exact: true }).click()
        await page.evaluate(() => {
          if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
        })
        await page.keyboard.press('Control+p')
        await page.getByRole('dialog', { name: 'Cortana command palette' }).waitFor()
        await page.waitForTimeout(300)
        await auditAccessibility(page, 'production command palette')
        await screenshot(page, 'command-blue-1440')
        await page.keyboard.press('Escape')
        await page.waitForFunction(
          () => document.activeElement?.getAttribute('aria-label') === 'Search your knowledge'
        )

        await page.getByRole('button', { name: 'Actions' }).click()
        await page.getByRole('menuitem', { name: 'Command palette' }).click()
        await page.getByRole('dialog', { name: 'Cortana command palette' }).waitFor()
        await page.keyboard.press('Escape')
        await page.waitForFunction(
          () => document.activeElement?.getAttribute('aria-label') === 'Actions'
        )

        await page.getByRole('button', { name: 'Switch workspace' }).click()
        await page.getByRole('menuitemradio', { name: 'Work' }).waitFor()
        await page.waitForTimeout(200)
        await auditAccessibility(page, 'production workspace switcher')
        await screenshot(page, 'workspace-menu-blue-1440')
        await page.keyboard.press('Escape')

        await page.getByRole('button', { name: 'Settings', exact: true }).click()
        await page.locator('.settings-view').waitFor()
        await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor()
        await auditAccessibility(page, 'settings readiness')
        await screenshot(page, 'settings-readiness-blue-1440')

        await page.getByRole('button', { name: 'Services', exact: true }).click()
        await page.getByRole('heading', { name: 'Services', exact: true }).waitFor()
        await auditAccessibility(page, 'settings services and recovery')
        await screenshot(page, 'settings-services-recovery-blue-1440')

        await page.getByRole('button', { name: 'Sources', exact: true }).click()
        await page.getByRole('heading', { name: 'Ingestion sources' }).waitFor()
        const addSource = page.getByRole('button', { name: 'Add source', exact: true })
        await addSource.click()
        const sourceTypeDialog = page.getByRole('dialog', { name: 'Choose a source type' })
        await sourceTypeDialog.waitFor()
        await page.waitForTimeout(300)
        await auditAccessibility(page, 'settings source-type selection')
        await screenshot(page, 'settings-source-type-blue-1440')
        await page.keyboard.press('Escape')
        await sourceTypeDialog.waitFor({ state: 'detached' })
        await page.waitForFunction(
          () => document.activeElement?.textContent?.trim() === 'Add source'
        )
        await page.getByRole('button', { name: 'Advanced source settings' }).click()
        await auditAccessibility(page, 'settings configured source')
        await screenshot(page, 'settings-source-configured-blue-1440')
        const removeSource = page.getByRole('button', { name: 'Remove work-code' })
        await removeSource.click()
        await page.getByRole('alertdialog').waitFor()
        await page.waitForTimeout(300)
        await auditAccessibility(page, 'settings destructive confirmation')
        await screenshot(page, 'settings-source-confirmation-blue-1440')
        await page.keyboard.press('Escape')
        await page.waitForFunction(
          () => document.activeElement?.getAttribute('aria-label') === 'Remove work-code'
        )
        await removeSource.click()
        await page.getByRole('alertdialog').waitFor()
        await page.getByRole('button', { name: 'Continue' }).click()
        await removeSource.waitFor({ state: 'detached' })
        await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'Sources')

        await page.getByRole('button', { name: 'Access', exact: true }).click()
        await page.getByRole('heading', { name: 'Agent access' }).waitFor()
        await auditAccessibility(page, 'settings write-only access')
        await screenshot(page, 'settings-access-blue-1440')

        await page.getByRole('button', { name: 'Updates', exact: true }).click()
        await page.getByRole('heading', { name: 'Updates', exact: true }).waitFor()
        await screenshot(page, 'settings-updater-blue-1440')

        await page.getByRole('button', { name: 'Query', exact: true }).click()
        await page.getByRole('heading', { name: 'Query and answer model' }).waitFor()
        await auditAccessibility(page, 'settings query model selector')
        await screenshot(page, 'settings-query-blue-1440')

        await page.getByRole('button', { name: 'Memory', exact: true }).click()
        await page.getByRole('heading', { name: 'Native agentic memory' }).waitFor()
        await auditAccessibility(page, 'settings memory control center')
        await screenshot(page, 'settings-memory-blue-1440')

        await page.getByRole('button', { name: 'Advanced', exact: true }).click()
        await page.getByRole('heading', { name: 'Local runtime' }).waitFor()
        await auditAccessibility(page, 'settings backup and recovery')
        await screenshot(page, 'settings-backup-recovery-blue-1440')

        const collapsedSidebar = page.locator('[data-slot="sidebar"][data-state="collapsed"]')
        await collapsedSidebar.waitFor()
        await page.waitForFunction(() => {
          const sidebar = document
            .querySelector('[data-slot="sidebar"][data-state="collapsed"]')
            ?.querySelector('[data-slot="sidebar-container"]')
          const header = document.querySelector('.m7-production-shell > header')
          if (!sidebar || !header) return false
          const sidebarBox = sidebar.getBoundingClientRect()
          const headerBox = header.getBoundingClientRect()
          return Math.abs(sidebarBox.right - headerBox.left) <= 1
        })
        const [sidebarBox, headerBox] = await Promise.all([
          collapsedSidebar.locator('[data-slot="sidebar-container"]').boundingBox(),
          page.locator('.m7-production-shell > header').boundingBox(),
        ])
        if (
          !sidebarBox ||
          !headerBox ||
          Math.abs(sidebarBox.x + sidebarBox.width - headerBox.x) > 1
        ) {
          throw new Error('Collapsed desktop sidebar leaves an empty layout gutter')
        }
      }

      await context.close()
    }
  }

  for (const width of widths) {
    const { context, page } = await openPage('blue', width)
    const search = page.getByRole('textbox', { name: 'Search your knowledge' })
    await search.fill('How do releases work?')
    await search.press('Enter')
    await page.getByRole('heading', { name: 'How do releases work?', level: 1 }).waitFor()

    for (const destination of ['Inbox', 'Conversations', 'Agent tools', 'Index', 'Help']) {
      await openDestination(page, width, destination)
      await auditAccessibility(page, `${destination} at ${width}px`)
      const horizontalOverflow = await page.evaluate(
        () => document.documentElement.scrollWidth > document.documentElement.clientWidth
      )
      if (horizontalOverflow) throw new Error(`${destination} overflows horizontally at ${width}px`)
      await screenshot(page, `${destination.toLowerCase().replaceAll(' ', '-')}-blue-${width}`)
    }
    await context.close()
  }

  for (const theme of themes) {
    for (const width of widths) {
      if (theme === 'blue' && width === 1440) continue
      const { context, page } = await openPage(theme, width)
      await openSettings(page, width)
      await auditAccessibility(page, `configured settings ${theme}/${width}`)
      await screenshot(page, `settings-configured-${theme}-${width}`)
      await context.close()
    }
  }

  for (const state of [
    'setup',
    'busy',
    'success',
    'warning',
    'failure',
    'cancelled',
    'retry',
    'recovery',
  ]) {
    const { context, page } = await openPage('blue', 1440, state)
    await openSettings(page, 1440)
    if (state === 'success') {
      await page.getByRole('button', { name: 'Services', exact: true }).click()
      await page.getByRole('heading', { name: 'Services', exact: true }).waitFor()
    }
    await auditAccessibility(page, `${state} settings state`)
    await screenshot(page, `settings-state-${state}-blue-1440`)
    await context.close()
  }

  const zoomContext = await browser.newContext({
    viewport: { width: 720, height: 500 },
    deviceScaleFactor: 2,
  })
  const zoomPage = await zoomContext.newPage()
  await zoomPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, { waitUntil: 'domcontentloaded' })
  await zoomPage.locator('[data-m7-production-shell-ready]').waitFor({ state: 'attached' })
  await openSettings(zoomPage, 720)
  await auditAccessibility(zoomPage, 'settings at 200% zoom')
  const horizontalOverflow = await zoomPage.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth
  )
  if (horizontalOverflow) throw new Error('The production shadcn shell overflows at 200% zoom')
  await zoomContext.close()

  const compactContext = await browser.newContext({ viewport: { width: 790, height: 800 } })
  const compactPage = await compactContext.newPage()
  await compactPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, {
    waitUntil: 'domcontentloaded',
  })
  await compactPage.locator('[data-m7-production-shell-ready]').waitFor({ state: 'attached' })
  const compactLayout = await compactPage.evaluate(() => {
    const source = document.querySelector('.source-panel')
    const workspace = document.querySelector('#main-content')
    if (!workspace) return null
    return {
      sourcePosition: source ? getComputedStyle(source).position : 'unmounted',
      sourceRight: source ? source.getBoundingClientRect().right : -1,
      workspaceWidth: workspace.getBoundingClientRect().width,
    }
  })
  if (
    !compactLayout ||
    !['fixed', 'unmounted'].includes(compactLayout.sourcePosition) ||
    compactLayout.sourceRight > 1 ||
    compactLayout.workspaceWidth < 700
  ) {
    throw new Error('The 781–799px compact shell leaves the source pane in the workspace flow')
  }
  await compactContext.close()

  const motionContext = await browser.newContext({
    viewport: { width: 768, height: 800 },
    reducedMotion: 'reduce',
  })
  const motionPage = await motionContext.newPage()
  await motionPage.goto(`${baseUrl}/?demo=1`, {
    waitUntil: 'domcontentloaded',
  })
  await motionPage.locator('[data-m7-production-shell-ready]').waitFor({ state: 'attached' })
  const navigationTrigger = motionPage.getByRole('button', { name: 'Toggle navigation' })
  await navigationTrigger.focus()
  await navigationTrigger.press('Enter')
  const mobileNavigation = motionPage.locator('[data-mobile="true"]')
  await mobileNavigation.waitFor()
  const animationDuration = await mobileNavigation.evaluate(
    (element) => getComputedStyle(element).animationDuration
  )
  if (Number.parseFloat(animationDuration) > 0.00002) {
    throw new Error(`Reduced-motion navigation animation remained ${animationDuration}`)
  }
  await motionPage.keyboard.press('Escape')
  await mobileNavigation.waitFor({ state: 'detached' })
  await motionPage.waitForFunction(() =>
    document.activeElement?.matches('[data-sidebar="trigger"]')
  )
  await motionContext.close()
}

await browser.close()

if (consoleErrors.length > 0) {
  throw new Error(`Browser console errors:\n${consoleErrors.join('\n')}`)
}

console.log(`Captured ${screenshotCount} final-renderer screenshots in ${output}`)
