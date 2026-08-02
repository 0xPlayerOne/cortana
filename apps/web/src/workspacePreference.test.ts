import { afterEach, expect, test } from 'bun:test'

import { readWorkspacePreference, writeWorkspacePreference } from './workspacePreference'

const STORAGE_KEY = 'cortana.workspace-selection.v1'

afterEach(() => {
  window.localStorage.removeItem(STORAGE_KEY)
})

test('workspace preference persists a bounded local scope and supports clearing', () => {
  expect(readWorkspacePreference()).toBe('')
  writeWorkspacePreference(' work ')
  expect(readWorkspacePreference()).toBe('work')
  expect(window.localStorage.getItem(STORAGE_KEY)).toBe('work')

  writeWorkspacePreference('')
  expect(readWorkspacePreference()).toBe('')
  expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull()
})

test('malformed or oversized workspace preferences fail open', () => {
  window.localStorage.setItem(STORAGE_KEY, 'x'.repeat(129))
  expect(readWorkspacePreference()).toBe('')
  writeWorkspacePreference('x'.repeat(129))
  expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull()
})
