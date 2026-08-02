import { afterEach, expect, test } from 'bun:test'

import {
  readSourceSelectionPreference,
  readWorkspacePreference,
  writeSourceSelectionPreference,
  writeWorkspacePreference,
} from './workspacePreference'

const WORKSPACE_SELECTION_KEY = 'cortana.workspace-selection.v1'
const SOURCE_SELECTION_KEY = 'cortana.source-selection.v1'

afterEach(() => {
  window.localStorage.removeItem(WORKSPACE_SELECTION_KEY)
  window.localStorage.removeItem(SOURCE_SELECTION_KEY)
})

test('workspace preference persists a bounded local scope and supports clearing', () => {
  expect(readWorkspacePreference()).toBe('')
  writeWorkspacePreference(' work ')
  expect(readWorkspacePreference()).toBe('work')
  expect(window.localStorage.getItem(WORKSPACE_SELECTION_KEY)).toBe('work')

  writeWorkspacePreference('')
  expect(readWorkspacePreference()).toBe('')
  expect(window.localStorage.getItem(WORKSPACE_SELECTION_KEY)).toBeNull()
})

test('malformed or oversized workspace preferences fail open', () => {
  window.localStorage.setItem(WORKSPACE_SELECTION_KEY, 'x'.repeat(129))
  expect(readWorkspacePreference()).toBe('')
  writeWorkspacePreference('x'.repeat(129))
  expect(window.localStorage.getItem(WORKSPACE_SELECTION_KEY)).toBeNull()
})

test('source preference persists a bounded local scope and supports clearing', () => {
  expect(readSourceSelectionPreference()).toBe('')
  writeSourceSelectionPreference(' work-code ')
  expect(readSourceSelectionPreference()).toBe('work-code')
  expect(window.localStorage.getItem(SOURCE_SELECTION_KEY)).toBe('work-code')

  writeSourceSelectionPreference('')
  expect(readSourceSelectionPreference()).toBe('')
  expect(window.localStorage.getItem(SOURCE_SELECTION_KEY)).toBeNull()
})

test('malformed or oversized source preferences fail open', () => {
  window.localStorage.setItem(SOURCE_SELECTION_KEY, 'x'.repeat(129))
  expect(readSourceSelectionPreference()).toBe('')
  writeSourceSelectionPreference('x'.repeat(129))
  expect(window.localStorage.getItem(SOURCE_SELECTION_KEY)).toBeNull()
})

test('storage failures fail open for source preference reads and writes', () => {
  const originalGetItem = window.localStorage.getItem.bind(window.localStorage)
  const originalSetItem = window.localStorage.setItem.bind(window.localStorage)
  const originalRemoveItem = window.localStorage.removeItem.bind(window.localStorage)

  Object.defineProperty(window, 'localStorage', {
    value: {
      getItem: () => {
        throw new Error('denied')
      },
      setItem: () => {
        throw new Error('denied')
      },
      removeItem: () => {
        throw new Error('denied')
      },
    },
    configurable: true,
  })

  expect(readSourceSelectionPreference()).toBe('')
  expect(() => writeSourceSelectionPreference('work-code')).not.toThrow()

  Object.defineProperty(window, 'localStorage', {
    value: {
      getItem: originalGetItem,
      setItem: originalSetItem,
      removeItem: originalRemoveItem,
    },
    configurable: true,
  })
})
