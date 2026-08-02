import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'

import { demoEvidence, demoStatus } from './demo'
import { answerResponse } from './test/fixtures'
import type {
  AnswerResponse,
  BrainDocument,
  BrainGraphPage,
  BrainStatus,
  ContextBundle,
  DesktopSourceJob,
} from './types'

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
  answer: null as ((query?: string) => Promise<AnswerResponse>) | null,
  context: null as ContextBundle | null,
  getContext: null as
    | ((
        query: string,
        project?: string,
        source?: string,
        signal?: AbortSignal
      ) => Promise<ContextBundle>)
    | null,
  getDocument: null as ((id: string, signal?: AbortSignal) => Promise<BrainDocument>) | null,
  graph: null as BrainGraphPage | null,
}

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: false,
  isDemoMode: false,
  getStatus: () => Promise.resolve(state.status),
  getDocuments: () => Promise.resolve({ documents: [], next_cursor: null }),
  getAnswer: (query?: string) =>
    state.answer ? state.answer(query) : Promise.reject(new Error('Answer request failed (503)')),
  getDocument: (id: string, signal?: AbortSignal) =>
    state.getDocument
      ? state.getDocument(id, signal)
      : Promise.reject(new Error('Document unavailable')),
  getGraph: () =>
    state.graph
      ? Promise.resolve(state.graph)
      : Promise.reject(new Error('Graph data unavailable')),
  getContext: (query: string, project?: string, source?: string, signal?: AbortSignal) =>
    state.getContext
      ? state.getContext(query, project, source, signal)
      : state.context
        ? Promise.resolve(state.context)
        : Promise.reject(new Error('Context retrieval failed (503)')),
  getDesktopSettings: () => Promise.reject(new Error('Settings are available in Cortana Desktop')),
  getDesktopInfo: () =>
    Promise.reject(new Error('Desktop information is available in Cortana Desktop')),
}))

const { App } = await import('./App')
const { UtilityView } = await import('./components/UtilityView')

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

test('titlebar controls perform real navigation actions', async () => {
  state.answer = () => Promise.resolve(answerResponse)
  await renderApp()

  fireEvent.click(screen.getByRole('button', { name: 'Filter documents' }))
  await new Promise((resolve) => setTimeout(resolve, 10))
  expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Filter documents' }))

  fireEvent.click(screen.getByRole('button', { name: 'Open conversations' }))
  expect(screen.getByRole('heading', { level: 1, name: 'Conversations' })).toBeTruthy()
})

test('Graph and Timeline rail buttons route to the existing workspace tabs', async () => {
  await renderApp()

  // Graph routes to the workspace Graph tab without leaving the workspace.
  fireEvent.click(railButton('Graph'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Graph' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(railButton('Graph').className).toContain('active')
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()

  // Timeline routes to the workspace Timeline tab and unselects Graph.
  fireEvent.click(railButton('Timeline'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Timeline' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(railButton('Timeline').className).toContain('active')
  expect(railButton('Graph').className).not.toContain('active')
  expect(screen.getByRole('tab', { name: 'Graph' }).getAttribute('aria-selected')).toBe('false')

  // Knowledge returns the workspace to its default tab.
  fireEvent.click(railButton('Knowledge'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Document' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()
})

test('graph and timeline evidence actions open the selected source', async () => {
  state.answer = () => Promise.resolve(answerResponse)
  await renderApp()

  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'release cadence' } })
  fireEvent.submit(input.closest('form')!)
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )

  for (const rail of ['Graph', 'Timeline']) {
    fireEvent.click(railButton(rail))
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: rail }).getAttribute('aria-selected')).toBe('true')
    )
    const evidenceButton = screen.getByRole('button', {
      name: new RegExp(`${rail} evidence: Deployment playbook`),
    })
    fireEvent.click(evidenceButton)
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Evidence/ }).getAttribute('aria-selected')).toBe(
        'true'
      )
    )
    expect(screen.getByRole('heading', { level: 1, name: 'Deployment playbook' })).toBeTruthy()
  }

  expect(screen.getByRole('link', { name: 'Retrieved passage' }).getAttribute('href')).toBe(
    '#passage'
  )
  expect(screen.getByRole('link', { name: 'Related evidence' }).getAttribute('href')).toBe(
    '#related'
  )
  expect(document.getElementById('passage')).toBeTruthy()
  expect(document.getElementById('related')).toBeTruthy()
})

test('graph view renders indexed document nodes when the graph endpoint responds', async () => {
  state.graph = {
    nodes: [
      {
        id: 'document:release-process',
        kind: 'document',
        label: 'Release process',
        project: 'work',
        source: 'work-code',
        document_id: 'release-process-id',
      },
    ],
    edges: [],
    next_cursor: null,
  }

  try {
    await renderApp()
    fireEvent.click(railButton('Graph'))
    await waitFor(() => expect(screen.getByText('1 nodes · 0 links')).toBeTruthy())
    expect(screen.getByRole('button', { name: 'Graph evidence: Release process' })).toBeTruthy()
  } finally {
    state.graph = null
  }
})

test('timeline order controls navigate to the selected evidence entry', async () => {
  state.answer = () =>
    Promise.resolve({
      ...answerResponse,
      evidence: [
        {
          chunk_id: 'old-notes',
          source: 'personal-notes',
          source_id: 'notes-old',
          title: 'Old notes',
          uri: null,
          content: 'Old evidence excerpt',
          score: 0.55,
          semantic_rank: 4,
          lexical_rank: null,
          updated_at: '2025-01-01T00:00:00Z',
        },
        {
          chunk_id: 'new-notes',
          source: 'personal-notes',
          source_id: 'notes-new',
          title: 'Newest notes',
          uri: null,
          content: 'New evidence excerpt',
          score: 0.92,
          semantic_rank: 1,
          lexical_rank: null,
          updated_at: '2026-01-01T00:00:00Z',
        },
      ],
      query: 'timeline sort',
      answer: 'Sorted evidence appears in a date timeline.',
      plan: {
        ...answerResponse.plan,
        queries: ['timeline sort'],
      },
    })

  await renderApp()

  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'timeline sort' } })
  fireEvent.submit(input.closest('form')!)

  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'timeline sort' })).toBeTruthy()
  )

  fireEvent.click(railButton('Timeline'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Timeline' }).getAttribute('aria-selected')).toBe('true')
  )

  fireEvent.click(screen.getByRole('button', { name: 'Timeline evidence: Old notes' }))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: /Evidence/ }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByRole('heading', { level: 1, name: 'Old notes' })).toBeTruthy()
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

test('Inbox retains terminal source-job history after the job stops running', () => {
  const job: DesktopSourceJob = {
    id: 'source-1',
    operation: 'validation',
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    status: 'failed',
    summary: 'Connector validation failed.',
    log: 'permission denied',
    started_at_unix_seconds: 1_785_000_000,
    completed_at_unix_seconds: 1_785_000_012,
    exit_code: 1,
    retryable: true,
    writes_indexed_data: false,
    budget: null,
  }
  render(
    <UtilityView
      kind="inbox"
      status={{ ...demoStatus, sync_runs: [] }}
      sourceJobs={[job]}
      query=""
      answer={null}
      evidence={[]}
      loading={false}
      error=""
      contextBundle={null}
      contextLoading={false}
      contextError=""
      contextTokens={0}
      desktopAvailable={false}
      sourceJobError="Source job cancellation failed"
      onSearchFocus={() => {}}
      onRetrieveContext={() => {}}
      onOpenSettings={() => {}}
      onOpenProject={() => {}}
    />
  )
  expect(screen.getByText('Recent source jobs')).toBeTruthy()
  expect(screen.getByText('work-code · validation')).toBeTruthy()
  expect(screen.getByText('Failed')).toBeTruthy()
  expect(screen.getByText(/Connector validation failed/)).toBeTruthy()
  expect(screen.getByRole('alert').textContent).toBe('Source job cancellation failed')
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

test('search history arrows navigate previous and next queries', async () => {
  state.answer = (query?: string) =>
    Promise.resolve({
      ...answerResponse,
      query: query || answerResponse.query,
      answer: `Answer for ${query || answerResponse.query}`,
    })
  render(<App />)

  const input = screen.getByLabelText('Search your knowledge')
  const submit = (value: string) => {
    fireEvent.change(input, { target: { value } })
    fireEvent.submit(input.closest('form')!)
  }

  submit('release cadence')
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )
  submit('desktop status')
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'desktop status' })).toBeTruthy()
  )

  const previous = screen.getByRole('button', { name: 'Previous search query' })
  const next = screen.getByRole('button', { name: 'Next search query' })
  expect(previous.getAttribute('disabled')).toBeNull()
  expect(next.getAttribute('disabled')).not.toBeNull()

  fireEvent.click(previous)
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )
  expect((input as HTMLInputElement).value).toBe('release cadence')
  expect(next.getAttribute('disabled')).toBeNull()

  fireEvent.click(next)
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'desktop status' })).toBeTruthy()
  )
})

test('Help lists the real keyboard shortcuts and project links', async () => {
  state.answer = () => Promise.resolve(answerResponse)
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
