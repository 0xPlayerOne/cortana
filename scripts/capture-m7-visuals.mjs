#!/usr/bin/env node

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'

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
  page.setDefaultTimeout(10_000)
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(`${renderer}/${theme}/${width}: ${message.text()}`)
    }
  })
  const query = renderer === 'shadcn' ? '?demo=1&renderer=shadcn' : '?demo=1'
  await page.goto(`${baseUrl}/${query}`, { waitUntil: 'networkidle' })
  if (renderer === 'shadcn') {
    await page.getByRole('heading', { name: 'Release evidence' }).waitFor()
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

if (renderer === 'shadcn') {
  for (const theme of themes) {
    for (const width of widths) {
      const { context, page } = await openPage(theme, width)
      await screenshot(page, `shell-${theme}-${width}`)
      if (theme === 'blue' && width === 1440) {
        await page.getByRole('button', { name: 'Review context boundary' }).click()
        await page.getByRole('dialog').waitFor()
        await screenshot(page, 'dialog-blue-1440')
      }
      await context.close()
    }
  }

  const zoomContext = await browser.newContext({
    viewport: { width: 720, height: 500 },
    deviceScaleFactor: 2,
  })
  const zoomPage = await zoomContext.newPage()
  await zoomPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, { waitUntil: 'networkidle' })
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
  await motionPage.goto(`${baseUrl}/?demo=1&renderer=shadcn`, { waitUntil: 'networkidle' })
  const reviewButton = motionPage.getByRole('button', { name: 'Review context boundary' })
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
  if (!(await reviewButton.evaluate((element) => element === document.activeElement))) {
    throw new Error('Dialog focus did not return to its trigger')
  }
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
          await page.goto(`${baseUrl}/?demo=1`, { waitUntil: 'networkidle' })
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
