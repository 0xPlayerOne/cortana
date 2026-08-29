import { expect, test } from 'bun:test'

import { normalizePathSeparators } from './check-web-ui-contract.mjs'

test('normalizes Windows source paths before applying UI contract rules', () => {
  expect(normalizePathSeparators('components\\shadcn\\textarea.tsx')).toBe(
    'components/shadcn/textarea.tsx'
  )
  expect(normalizePathSeparators('components\\Workspace.tsx')).toBe('components/Workspace.tsx')
})
