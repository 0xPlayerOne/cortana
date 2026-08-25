import { expect, test } from 'bun:test'

import { buildTestGroups, resolveMaxParallel, scheduleGroups } from './js-test-groups.mjs'

test('buildTestGroups keeps the shared group first and isolates API-mock suites', () => {
  const groups = buildTestGroups(
    ['apps/web/src/App.test.tsx', 'apps/web/src/api.test.ts', 'scripts/helper.test.mjs'],
    new Set(['App.test.tsx'])
  )

  expect(groups).toEqual([
    ['apps/web/src/api.test.ts', 'scripts/helper.test.mjs'],
    ['apps/web/src/App.test.tsx'],
  ])
})

test('resolveMaxParallel honors an explicit bounded override', () => {
  expect(resolveMaxParallel(9, '2')).toBe(2)
  expect(resolveMaxParallel(1, '9')).toBe(1)
  expect(() => resolveMaxParallel(9, '0')).toThrow(
    'CORTANA_TEST_MAX_PARALLEL must be a positive integer'
  )
})

test('scheduleGroups batches light suites but isolates resource-heavy suites', () => {
  const groups = [
    ['shared.test.ts'],
    ['App.desktop.test.tsx'],
    ['small-a.test.ts'],
    ['small-b.test.ts'],
    ['sourceJobs.test.ts'],
  ]

  expect(
    scheduleGroups(groups, 2, new Set(['App.desktop.test.tsx', 'sourceJobs.test.ts']))
  ).toEqual([[0, 2], [3], [1], [4]])
})
