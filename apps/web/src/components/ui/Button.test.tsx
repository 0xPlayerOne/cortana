import { expect, test } from 'bun:test'

import { buttonClassName } from './buttonClasses'

test('maps the shared button variants to stable semantic classes', () => {
  expect(buttonClassName('primary')).toBe('cortana-button cortana-button--primary')
  expect(buttonClassName('secondary')).toBe('cortana-button cortana-button--secondary')
  expect(buttonClassName('compact')).toBe('cortana-button cortana-button--compact')
  expect(buttonClassName('ghost')).toBe('cortana-button cortana-button--ghost')
  expect(buttonClassName('icon')).toBe('cortana-button cortana-button--icon')
})

test('merges an optional class without dropping the semantic variant', () => {
  expect(buttonClassName('secondary', 'settings-refresh')).toBe(
    'cortana-button cortana-button--secondary settings-refresh'
  )
})
