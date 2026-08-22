import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

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

test('legacy settings selectors leave the shared primitive in control', () => {
  const settingsCss = readFileSync(
    fileURLToPath(new URL('../../styles/settings.css', import.meta.url)),
    'utf8'
  )

  expect(settingsCss).toContain('source-card-actions button:not(.cortana-button)')
  expect(settingsCss).toContain('source-validation-job button:not(.cortana-button)')
  expect(settingsCss).toContain('secret-input button:not(.cortana-button)')
  expect(settingsCss).toContain('readiness-list button:not(.cortana-button)')
})
