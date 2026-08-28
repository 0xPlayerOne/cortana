import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'

import { desktopSettings } from './test/fixtures'

afterEach(cleanup)

const realApi = await import('./api')

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: false,
}))

const { SettingsView } = await import('./components/SettingsView')

test('browser settings adopt a demo fixture that arrives after the view mounts', async () => {
  const view = render(<SettingsView onSaved={() => undefined} />)
  expect(screen.getByRole('heading', { name: 'Desktop settings' })).toBeTruthy()

  view.rerender(<SettingsView desktopSettings={desktopSettings} onSaved={() => undefined} />)

  expect(await screen.findByRole('heading', { level: 1, name: 'Settings' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Services' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Readiness' }).getAttribute('aria-current')).toBe(
    'page'
  )

  fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
  expect(screen.getByRole('button', { name: 'Sources' }).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('button', { name: 'Readiness' }).hasAttribute('aria-current')).toBe(false)
})
