import { expect, test } from 'bun:test'

import {
  setViewportAndWaitForLayout,
  waitForInitialKeyboardTarget,
} from './knowledge-accessibility-browser.mjs'

test('waits for the rendered skip link before the initial keyboard check', async () => {
  const calls = []
  const skipLink = { waitFor: async (options) => calls.push(options) }
  const page = {
    locator(selector) {
      calls.push(selector)
      return skipLink
    },
  }

  expect(await waitForInitialKeyboardTarget(page)).toBe(skipLink)
  expect(calls).toEqual(['a[href="#main-content"]', { state: 'visible' }])
})

test('waits for the browser layout after changing viewport dimensions', async () => {
  const calls = []
  const page = {
    setViewportSize: async (viewport) => calls.push(['setViewportSize', viewport]),
    evaluate: async (callback) => {
      calls.push(['evaluate', typeof callback])
      return callback
    },
  }

  await setViewportAndWaitForLayout(page, { width: 768, height: 1024 })

  expect(calls).toEqual([
    ['setViewportSize', { width: 768, height: 1024 }],
    ['evaluate', 'function'],
  ])
})
