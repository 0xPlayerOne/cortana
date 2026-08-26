import { expect, test } from 'bun:test'

import { assertWithinBudget } from './check-web-bundle-budget.mjs'

test('accepts an asset at the reviewed byte budget', () => {
  expect(() => assertWithinBudget('renderer entry', 475_000, 475_000)).not.toThrow()
})

test('rejects an asset that exceeds the reviewed byte budget', () => {
  expect(() => assertWithinBudget('renderer entry', 475_001, 475_000)).toThrow(
    'renderer entry is 475001 bytes; budget is 475000 bytes'
  )
})
