import { afterEach, expect, test } from 'bun:test'

import { shortcutLabel, shortcutModifier } from './shortcuts'

const originalPlatform = navigator.platform

afterEach(() => {
  Object.defineProperty(navigator, 'platform', {
    configurable: true,
    value: originalPlatform,
  })
})

test('uses the macOS modifier label on Apple platforms', () => {
  Object.defineProperty(navigator, 'platform', { configurable: true, value: 'MacIntel' })
  expect(shortcutModifier()).toBe('⌘')
  expect(shortcutLabel('MOD ⇧ F')).toBe('⌘ ⇧ F')
})

test('uses Control on Windows and Linux platforms', () => {
  Object.defineProperty(navigator, 'platform', { configurable: true, value: 'Linux x86_64' })
  expect(shortcutModifier()).toBe('Ctrl')
  expect(shortcutLabel('MOD K')).toBe('Ctrl K')

  Object.defineProperty(navigator, 'platform', { configurable: true, value: 'Win32' })
  expect(shortcutModifier()).toBe('Ctrl')
  expect(shortcutLabel('MOD P')).toBe('Ctrl P')
})
