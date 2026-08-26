import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { M7ShadcnPrototype } from './M7ShadcnPrototype'

afterEach(() => cleanup())

test('switches evidence tabs without losing the document workflow', async () => {
  render(<M7ShadcnPrototype />)

  expect(screen.getByRole('heading', { name: 'Release evidence' })).toBeTruthy()
  fireEvent.click(screen.getByRole('tab', { name: 'Answer' }))
  expect(
    await screen.findByText(
      'Synthesized answers will compose the same evidence cards and citations.'
    )
  ).toBeTruthy()
  fireEvent.click(screen.getByRole('tab', { name: 'Document' }))
  expect(await screen.findByText('Evidence spine')).toBeTruthy()
})

test('composes dense retrieval settings from shared controls', () => {
  render(<M7ShadcnPrototype />)

  expect((screen.getByRole('textbox', { name: 'Retrieval mode' }) as HTMLInputElement).value).toBe(
    'Balanced'
  )
  expect(
    (screen.getByRole('spinbutton', { name: 'Maximum sources' }) as HTMLInputElement).value
  ).toBe('12')

  const citations = screen.getByRole('switch', { name: 'Require citations' })
  expect(citations.getAttribute('aria-checked')).toBe('true')
  fireEvent.click(citations)
  expect(citations.getAttribute('aria-checked')).toBe('false')
})

test('uses the shared feedback contract for exceptional retrieval states', () => {
  const { rerender } = render(<M7ShadcnPrototype previewState="loading" />)
  expect(
    screen.getByRole('status', {
      name: 'Loading release evidence: Retrieving bounded sources.',
    })
  ).toBeTruthy()

  rerender(<M7ShadcnPrototype previewState="empty" />)
  expect(screen.getByText('No release evidence')).toBeTruthy()

  rerender(<M7ShadcnPrototype previewState="error" />)
  expect(screen.getByRole('button', { name: 'Retry' })).toBeTruthy()
})

test('opens the command palette by trigger and shortcut and returns focus', async () => {
  const user = userEvent.setup()
  render(<M7ShadcnPrototype />)

  const trigger = screen.getByRole('button', { name: 'Open command palette' })
  await user.click(trigger)
  expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeTruthy()
  expect(document.activeElement).toBe(screen.getByRole('combobox', { name: 'Search commands' }))

  await user.keyboard('{Escape}')
  await new Promise((resolve) => requestAnimationFrame(resolve))
  expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull()
  expect(document.activeElement).toBe(trigger)

  await user.keyboard('{Control>}k{/Control}')
  expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeTruthy()
  await user.keyboard('{Escape}')
  const search = screen.getByRole('textbox', { name: 'Search your knowledge' })
  search.focus()
  await user.keyboard('{Control>}k{/Control}')
  await user.keyboard('{Escape}')
  await waitFor(() => expect(document.activeElement).toBe(search))
})

test('navigates workspace and filter overlays with keyboard semantics', async () => {
  const user = userEvent.setup()
  render(<M7ShadcnPrototype />)

  const workspace = screen.getByRole('button', {
    name: 'Switch workspace. Current workspace: Personal',
  })
  fireEvent.click(workspace)
  fireEvent.click(await screen.findByRole('menuitem', { name: 'Product' }))
  expect(
    screen.getByRole('button', { name: 'Switch workspace. Current workspace: Product' })
  ).toBeTruthy()

  const filters = screen.getByRole('button', { name: 'Filter evidence' })
  fireEvent.click(filters)
  expect(
    screen.getByText('Narrow the visible evidence without expanding retrieval scope.')
  ).toBeTruthy()
  await user.keyboard('{Escape}')
  await waitFor(() => {
    expect(
      screen.queryByText('Narrow the visible evidence without expanding retrieval scope.')
    ).toBeNull()
    expect(document.activeElement).toBe(filters)
  })
})

test('groups evidence context actions and preserves pagination link semantics', async () => {
  const user = userEvent.setup()
  render(<M7ShadcnPrototype />)

  const evidenceActions = screen.getByRole('button', { name: 'Open evidence actions' })
  evidenceActions.focus()
  await user.keyboard('{Enter}')
  const action = await screen.findByRole('menuitem', { name: 'Copy citation' })
  fireEvent.click(action)
  expect(screen.getByText('Copied citation').getAttribute('role')).toBe('status')

  fireEvent.contextMenu(screen.getByText('How do releases work?'))
  expect(await screen.findByRole('menuitem', { name: 'Open source' })).toBeTruthy()
  await user.keyboard('{Escape}')

  expect(screen.getByRole('link', { name: '2' }).getAttribute('href')).toBe('#page-2')
  expect(screen.queryByRole('button', { name: '2' })).toBeNull()
})
