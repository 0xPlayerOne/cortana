import { expect, test } from 'bun:test'

import { assertWithinBudget, staticImportKeys } from './check-web-bundle-budget.mjs'

test('accepts an asset at the reviewed byte budget', () => {
  expect(() => assertWithinBudget('renderer entry', 475_000, 475_000)).not.toThrow()
})

test('rejects an asset that exceeds the reviewed byte budget', () => {
  expect(() => assertWithinBudget('renderer entry', 475_001, 475_000)).toThrow(
    'renderer entry is 475001 bytes; budget is 475000 bytes'
  )
})

test('walks the complete static import graph without double counting cycles', () => {
  const manifest = {
    entry: { imports: ['shared', 'feature'] },
    shared: { imports: ['base'] },
    feature: { imports: ['shared'] },
    base: { imports: ['entry'] },
  }

  expect([...staticImportKeys(manifest, ['entry'])].sort()).toEqual([
    'base',
    'entry',
    'feature',
    'shared',
  ])
})
