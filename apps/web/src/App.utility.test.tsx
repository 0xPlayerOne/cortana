import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'

import { demoEvidence, demoStatus } from './demo'
import { answerResponse } from './test/fixtures'
import type { AnswerResponse, BrainStatus, ContextBundle } from './types'

afterEach(cleanup)

// Capture the real api module, then register a mock that delegates every export
// to a mutable state object so each test controls the network boundary.
const realApi = await import('./api')

const contextBundle: ContextBundle = {
  query: 'How do releases work?',
  context: demoEvidence.map((item) => item.content).join('\n\n'),
  evidence: demoEvidence,
  metrics: {
    retrieved: 4,
    included: 4,
    omitted: 0,
    estimated_tokens: 512,
    max_tokens: 8000,
  },
}

const state = {
  status: demoStatus as BrainStatus | null,
  answer: null as (() => Promise<AnswerResponse>) | null,
  context: null as ContextBundle | null,
}

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: false,
  isDemoMode: false,
  getStatus: () => Promise.resolve(state.status),
  getDocuments: () => Promise.resolve({ documents: [], next_cursor: null }),
  getAnswer: () =>
    state.answer ? state.answer() : Promise.reject(new Error('Answer request failed (503)')),
  getDocument: () => Promise.reject(new Error('Document unavailable')),
  getContext: () =>
    state.context
      ? Promise.resolve(state.context)
      : Promise.reject(new Error('Context retrieval failed (503)')),
  getDesktopSettings: () => Promise.reject(new Error('Settings are available in Cortana Desktop')),
  getDesktopInfo: () =>
    Promise.reject(new Error('Desktop information is available in Cortana Desktop')),
}))

const { App } = await import('./App')

const RAIL_LABELS = [
  'Search',
  'Knowledge',
  'Graph',
  'Inbox',
  'Conversations',
  'Agent tools',
  'Timeline',
  'Index',
  'Settings',
  'Help',
]

function railButton(label: string) {
  const rail = screen.getByRole('navigation', { name: 'Primary' })
  return within(rail).getByRole('button', { name: label })
}

async function renderApp() {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
}

test('every rail button is enabled and Search focuses the query input', async () => {
  await renderApp()

  for (const label of RAIL_LABELS) {
    const button = railButton(label)
    expect(button.hasAttribute('disabled')).toBe(false)
    expect(button.getAttribute('title')).toBe(label)
  }

  fireEvent.click(railButton('Search'))
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByLabelText('Search your knowledge'))
  )
})

test('Graph and Timeline rail buttons route to the existing workspace tabs', async () => {
  await renderApp()

  // Graph routes to the workspace Graph tab without leaving the workspace.
  fireEvent.click(railButton('Graph'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Graph' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()

  // Timeline routes to the workspace Timeline tab and unselects Graph.
  fireEvent.click(railButton('Timeline'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Timeline' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByRole('tab', { name: 'Graph' }).getAttribute('aria-selected')).toBe('false')

  // Knowledge returns the workspace to its default tab.
  fireEvent.click(railButton('Knowledge'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Document' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()
})

test('Inbox renders current sync attention and a truthful idle empty state', async () => {
  // The demo status contains a budget-exceeded sync run that needs attention.
  await renderApp()
  fireEvent.click(railButton('Inbox'))
  await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Inbox' })).toBeTruthy())
  expect(screen.getByText('community-discord')).toBeTruthy()
  expect(screen.getByText('Budget exceeded')).toBeTruthy()
  // A clean sync run must not be listed as attention.
  expect(screen.queryByText('Succeeded')).toBeNull()

  // An idle index renders the truthful empty state instead of fabricated history.
  cleanup()
  state.status = { ...demoStatus, sync_runs: [] }
  await renderApp()
  fireEvent.click(railButton('Inbox'))
  await waitFor(() => expect(screen.getByText('No sync attention')).toBeTruthy())
  expect(screen.getByRole('button', { name: 'Open settings' })).toBeTruthy()
})

test('Index renders live BrainStatus metrics and an offline empty state', async () => {
  await renderApp()
  fireEvent.click(railButton('Index'))
  await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Index' })).toBeTruthy())

  // Real metric content from the current status snapshot.
  expect(screen.getByText('9,834')).toBeTruthy()
  expect(screen.getByText('128,412')).toBeTruthy()
  expect(screen.getByText('42,891 entries')).toBeTruthy()
  expect(screen.getByText('synthesized')).toBeTruthy()

  // An unreachable brain renders the offline empty state.
  cleanup()
  state.status = null
  await renderApp()
  fireEvent.click(railButton('Index'))
  await waitFor(() => expect(screen.getByText('Index offline')).toBeTruthy())
  expect(screen.getByText('Open settings')).toBeTruthy()
})

test('Agent tools prompts retrieval and then shows the generated context metrics', async () => {
  await renderApp()
  fireEvent.click(railButton('Agent tools'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Agent tools' })).toBeTruthy()
  )

  // No bundle has been generated yet: the view clearly prompts retrieval.
  expect(screen.getByText('No context generated yet')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Retrieve context' })).toBeTruthy()
  // The agent context window reflects the real current session state.
  expect(screen.getByText(/tokens assembled from the active query/)).toBeTruthy()

  // Retrieval succeeds and the generated bundle's real metrics render.
  state.context = contextBundle
  fireEvent.click(screen.getByRole('button', { name: 'Retrieve context' }))
  await waitFor(() => expect(screen.getByText('Retrieved')).toBeTruthy())
  expect(screen.getByText('512')).toBeTruthy()
  expect(screen.getByText('8,000')).toBeTruthy()
  expect(screen.getByText('Deployment playbook')).toBeTruthy()
  expect(screen.getByText('How do releases work?')).toBeTruthy()
})

test('Conversations shows the session state and offers search focus', async () => {
  await renderApp()
  fireEvent.click(railButton('Conversations'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Conversations' })).toBeTruthy()
  )

  // No question has been answered yet: truthful empty state with search focus.
  expect(screen.getByText('No conversation yet')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Search the brain' }))
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByLabelText('Search your knowledge'))
  )

  // After a successful search, the current query/answer/evidence state renders.
  state.answer = () => Promise.resolve(answerResponse)
  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'release cadence' } })
  fireEvent.submit(input.closest('form')!)
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )
  fireEvent.click(railButton('Conversations'))
  await waitFor(() => expect(screen.getByText('4 cited passages')).toBeTruthy())
  expect(screen.getByText(/Promote short-lived changes through staging/)).toBeTruthy()
  expect(screen.getByText('How do releases work?')).toBeTruthy()
})

test('Help lists the real keyboard shortcuts and project links', async () => {
  await renderApp()
  fireEvent.click(railButton('Help'))
  await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Help' })).toBeTruthy())

  expect(screen.getByText('Focus the search bar')).toBeTruthy()
  expect(screen.getByText('Toggle the command palette')).toBeTruthy()
  expect(screen.getByText('Open the document filter')).toBeTruthy()
  expect(screen.getByText('Close panels and the palette')).toBeTruthy()

  const project = screen.getByRole('link', { name: /GitHub project/ })
  expect(project.getAttribute('href')).toBe('https://github.com/0xPlayerOne/cortana')
  const docs = screen.getByRole('link', { name: /Documentation/ })
  expect(docs.getAttribute('href')).toBe('https://github.com/0xPlayerOne/cortana/tree/main/docs')
  // The desktop-only project opener must not appear in web mode.
  expect(screen.queryByRole('button', { name: 'Open project page' })).toBeNull()
})
