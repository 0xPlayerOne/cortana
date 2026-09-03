import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'

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

afterEach(async () => {
  await act(async () => {
    // Stop UtilityView effects before draining pending work so a late shell
    // update cannot reach the renderer owned by the next test.
    cleanup()
    await new Promise((resolve) => setTimeout(resolve, 0))
    await Promise.resolve()
    await Promise.resolve()
  })

  // Keep mutable API fixtures isolated between tests. In particular, a test
  // that enables answer/context data must not unlock result-only navigation in
  // the next test or leak a graph page into a fresh shell.
  resetState()
})

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
  getGraph: null as
    | ((
        project?: string,
        signal?: AbortSignal,
        options?: { edgeKind?: BrainGraphPage['edges'][number]['kind'] }
      ) => Promise<BrainGraphPage>)
    | null,
  graph: null as BrainGraphPage | null,
}

const resetState = () => {
  state.status = demoStatus
  state.answer = null
  state.context = null
  state.getContext = null
  state.getDocument = null
  state.getGraph = null
  state.graph = null
}

beforeEach(() => {
  // A preceding test file can leave a renderer behind when Bun reuses the
  // worker. Start each utility test from a clean DOM and API fixture state.
  cleanup()
  resetState()
})

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
  getGraph: (
    _project?: string,
    _source?: string,
    _query?: string,
    _cursor?: string,
    signal?: AbortSignal,
    options?: { edgeKind?: BrainGraphPage['edges'][number]['kind'] }
  ) =>
    state.getGraph
      ? state.getGraph(_project, signal, options)
      : state.graph
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
const { M7ActivityInbox } = await import('./components/m7/M7ActivityInbox')

const RAIL_LABELS = [
  'Knowledge',
  'Graph',
  'Inbox',
  'Conversations',
  'Agent tools',
  'Index',
  'Settings',
  'Help',
]

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

function railButton(label: string) {
  const rail = screen.getByRole('navigation', { name: 'Primary navigation' })
  return within(rail).getByRole('button', { name: label })
}

async function selectHeaderAction(label: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Actions' }))
  fireEvent.click(await screen.findByRole('menuitem', { name: label }))
}

async function renderApp() {
  render(<App />)
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
}

test('every sidebar destination is enabled and the persistent search remains available', async () => {
  await renderApp()

  for (const label of RAIL_LABELS) {
    const button = railButton(label)
    expect(button.hasAttribute('disabled')).toBe(false)
    expect(button.getAttribute('data-slot')).toBe('sidebar-menu-button')
  }
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()
})

test('titlebar controls perform real navigation actions', async () => {
  state.answer = () => Promise.resolve(answerResponse)
  await renderApp()

  await selectHeaderAction('Filter documents')
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
  expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Filter documents' }))

  await selectHeaderAction('Open conversations')
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Conversations' })).toBeTruthy()
  )
})

test('navigation and source-header actions use the shared icon-button primitive', async () => {
  await renderApp()

  for (const label of ['Add source', 'Source settings']) {
    expect(screen.getByRole('button', { name: label }).getAttribute('data-slot')).toBe(
      'tooltip-trigger'
    )
  }
  expect(screen.getByRole('button', { name: 'Actions' }).getAttribute('data-slot')).toBe(
    'dropdown-menu-trigger'
  )
})

test('Graph is a separate full-screen sidebar view and Timeline remains result-only', async () => {
  await renderApp()

  // Graph routes to a full-screen workspace view without a duplicate top tab.
  fireEvent.click(railButton('Graph'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Graph unavailable' })).toBeTruthy()
  )
  expect(
    within(screen.getByRole('alert')).getByRole('heading', { name: 'Graph unavailable' })
  ).toBeTruthy()
  expect(screen.queryByRole('tab', { name: 'Graph' })).toBeNull()
  expect(railButton('Graph').hasAttribute('data-active')).toBe(true)
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()

  // Without a search result the Timeline rail stays disabled and cannot
  // select the result-only timeline tab.
  expect(screen.queryByRole('tab', { name: 'Timeline' })).toBeNull()

  // A search result unlocks Timeline, which routes to the workspace Timeline
  // tab and unselects Graph.
  state.answer = () => Promise.resolve(answerResponse)
  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'release cadence' } })
  fireEvent.submit(input.closest('form')!)
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('tab', { name: 'Timeline' }))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Timeline' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(railButton('Graph').hasAttribute('data-active')).toBe(false)
  expect(screen.queryByRole('tab', { name: 'Graph' })).toBeNull()

  // Knowledge returns the workspace to its default tab.
  fireEvent.click(railButton('Knowledge'))
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: 'Document' }).getAttribute('aria-selected')).toBe('true')
  )
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()
})

test('Graph expands to full width and hides the source and context panels', async () => {
  await renderApp()

  // The tablet document layout keeps sources inline and context in its Sheet.
  expect(document.querySelector('.source-panel')).toBeTruthy()
  expect(screen.queryByText('Agent context')).toBeNull()

  fireEvent.click(railButton('Graph'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Graph unavailable' })).toBeTruthy()
  )

  // Full-screen graph: no source panel, no context panel, no workspace tabs,
  // and the shell marks the layout so the graph spans the full width.
  expect(document.querySelector('.source-panel')).toBeNull()
  expect(screen.queryByText('Agent context')).toBeNull()
  expect(screen.queryByRole('tab', { name: 'Graph' })).toBeNull()
  expect(screen.getByLabelText('Search your knowledge')).toBeTruthy()

  // The title-bar source action leaves the full-screen graph so the panel
  // becomes reachable again instead of silently doing nothing.
  await selectHeaderAction('Open sources')
  await waitFor(() => expect(document.querySelector('.source-panel')).toBeTruthy())
  expect(screen.getByRole('tab', { name: 'Document' }).getAttribute('aria-selected')).toBe('true')

  // Re-entering the graph hides the panels again; Knowledge restores them.
  fireEvent.click(railButton('Graph'))
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Graph unavailable' })).toBeTruthy()
  )
  expect(document.querySelector('.source-panel')).toBeNull()
  fireEvent.click(railButton('Knowledge'))
  await waitFor(() => expect(document.querySelector('.source-panel')).toBeTruthy())
  expect(screen.queryByText('Agent context')).toBeNull()
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
  for (const surface of ['Graph', 'Timeline']) {
    if (surface === 'Graph') {
      fireEvent.click(railButton('Graph'))
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: 'Open evidence: Deployment playbook' })
        ).toBeTruthy()
      )
      expect(screen.queryByRole('tab', { name: 'Graph' })).toBeNull()
    } else {
      fireEvent.click(screen.getByRole('tab', { name: 'Timeline' }))
      await waitFor(() =>
        expect(screen.getByRole('tab', { name: surface }).getAttribute('aria-selected')).toBe(
          'true'
        )
      )
    }
    const evidenceButton = screen.getByRole('button', {
      name:
        surface === 'Graph'
          ? 'Open evidence: Deployment playbook'
          : new RegExp(`${surface} evidence: Deployment playbook`),
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
    await waitFor(() => expect(screen.getByText('Showing 1 of 1 document · 0 links')).toBeTruthy())
    const node = screen.getByRole('button', { name: 'Open document: Release process' })
    node.focus()
    fireEvent.keyDown(node, { key: 'Enter' })
    fireEvent.click(node)
    const selection = await waitFor(() =>
      screen.getByRole('complementary', { name: 'Selected graph node' })
    )
    expect(selection.getAttribute('aria-live')).toBe('polite')
    expect(within(selection).getByRole('button', { name: 'Open document' })).toBeTruthy()
  } finally {
    state.graph = null
  }
})

test('large graph pages render through a bounded keyboard-operable window', async () => {
  state.graph = {
    nodes: Array.from({ length: 30 }, (_, index) => ({
      id: `document:${index}`,
      kind: 'document' as const,
      label: `Document ${index}`,
      project: 'work',
      source: 'work-code',
      document_id: `document-${index}`,
    })),
    edges: [],
    next_cursor: null,
  }

  try {
    await renderApp()
    fireEvent.click(railButton('Graph'))
    await waitFor(
      () => expect(screen.getByText('Showing 12 of 30 documents · 0 links')).toBeTruthy(),
      { timeout: 15_000 }
    )
    expect(screen.getAllByRole('button', { name: /^Open document: Document/ })).toHaveLength(12)
    const showMore = screen.getByRole('button', { name: 'Show more nodes' })
    showMore.focus()
    fireEvent.click(showMore)
    await waitFor(
      () => expect(screen.getByText('Showing 24 of 30 documents · 0 links')).toBeTruthy(),
      { timeout: 15_000 }
    )
    expect(screen.getAllByRole('button', { name: /^Open document: Document/ })).toHaveLength(24)
  } finally {
    state.graph = null
  }
})

test('closing and reopening graph aborts and discards a stale expansion response', async () => {
  const stale = deferred<BrainGraphPage>()
  const fresh = deferred<BrainGraphPage>()
  let staleSignal: AbortSignal | undefined
  state.getGraph = (_project, signal) => {
    if (staleSignal) return fresh.promise
    staleSignal = signal
    return stale.promise
  }
  const page = (label: string): BrainGraphPage => ({
    nodes: [
      {
        id: `document:${label}`,
        kind: 'document',
        label,
        project: 'work',
        source: 'work-code',
        document_id: `${label}-id`,
      },
    ],
    edges: [],
    next_cursor: null,
  })

  try {
    await renderApp()
    fireEvent.click(railButton('Graph'))
    const loading = await screen.findByRole('heading', { name: 'Loading knowledge graph' })
    expect(loading.closest('[role="status"]')).toBeTruthy()
    fireEvent.click(railButton('Knowledge'))
    await waitFor(() => expect(staleSignal?.aborted).toBe(true))
    fireEvent.click(railButton('Graph'))

    fresh.resolve(page('Fresh graph node'))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Open document: Fresh graph node' })).toBeTruthy()
    )
    stale.resolve(page('Stale graph node'))
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Open document: Stale graph node' })).toBeNull()
    )
  } finally {
    state.getGraph = null
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

  fireEvent.click(screen.getByRole('tab', { name: 'Timeline' }))
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
  await screen.findByRole('heading', { level: 1, name: 'Inbox' })
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

test('shadcn Inbox shares the responsive utility-page spacing contract', () => {
  render(<M7ActivityInbox status={demoStatus} sourceJobs={[]} onOpenSettings={() => {}} />)

  const inbox = screen.getByRole('heading', { level: 1, name: 'Inbox' }).closest('main')
  expect(inbox?.className).toContain('m7-utility-view')
  expect(inbox?.querySelector('.utility-header')).toBeTruthy()
  expect(inbox?.querySelector('[data-m7-activity-body].utility-body')).toBeTruthy()
  expect(inbox?.querySelector('.activity-card-grid')).toBeTruthy()
  expect(inbox?.querySelector('.activity-card-title-line')).toBeTruthy()
  expect(inbox?.querySelector('.activity-card-detail-row')).toBeTruthy()
  expect(inbox?.querySelector('[data-slot="card-description"]')).toBeNull()
  expect(inbox?.querySelector('.max-w-5xl')).toBeNull()
})

test('Inbox does not claim clean sync history while runtime status is unavailable', () => {
  let retries = 0
  render(
    <UtilityView
      kind="inbox"
      status={null}
      statusError="Runtime status unavailable"
      sourceJobs={[]}
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
      onSearchFocus={() => {}}
      onRetrieveContext={() => {}}
      onOpenSettings={() => {}}
      onOpenProject={() => {}}
      onRetryStatus={() => {
        retries += 1
      }}
    />
  )
  expect(screen.getByText('Sync health unavailable')).toBeTruthy()
  expect(screen.getByText('Runtime status unavailable')).toBeTruthy()
  expect(screen.queryByText('No sync attention')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Retry status' }))
  expect(retries).toBe(1)
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
  fireEvent.click(screen.getByText('View job log'))
  expect(screen.getByText('permission denied')).toBeTruthy()
  expect(screen.getByRole('alert').textContent).toBe('Source job cancellation failed')
})

test('Inbox keeps a cancelling source job visibly in progress until it exits', () => {
  const job: DesktopSourceJob = {
    id: 'source-cancelling',
    operation: 'trial-sync',
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    status: 'cancelling',
    summary: 'Cancelling source trial-sync…',
    log: '',
    started_at_unix_seconds: 1_785_000_000,
    completed_at_unix_seconds: null,
    exit_code: null,
    retryable: false,
    writes_indexed_data: true,
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
      onSearchFocus={() => {}}
      onRetrieveContext={() => {}}
      onOpenSettings={() => {}}
      onOpenProject={() => {}}
      onCancelSourceJob={() => {}}
    />
  )

  expect(screen.getByText('Cancelling…')).toBeTruthy()
  expect(
    screen
      .getByRole('button', { name: 'Cancel work work-code trial-sync' })
      .hasAttribute('disabled')
  ).toBe(true)
})

test('Index renders live BrainStatus metrics and a truthful loading empty state', async () => {
  await renderApp()
  fireEvent.click(railButton('Index'))
  await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Index' })).toBeTruthy())

  // Real metric content from the current status snapshot.
  expect(screen.getByText('9,834')).toBeTruthy()
  expect(screen.getByText('128,412')).toBeTruthy()
  expect(screen.getByText('42,891 entries')).toBeTruthy()
  expect(screen.getByText('synthesized')).toBeTruthy()

  // Without a status error, the shell is still waiting for the runtime rather
  // than claiming that the index is offline.
  cleanup()
  state.status = null
  await renderApp()
  fireEvent.click(railButton('Index'))
  await waitFor(() => expect(screen.getByText('Loading index')).toBeTruthy())
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

test('Agent tools copies the exact generated context bundle for local agent handoff', async () => {
  let copiedText = ''
  const originalClipboard = navigator.clipboard
  Object.defineProperty(navigator, 'clipboard', {
    value: {
      writeText: (value: string) => {
        copiedText = value
        return Promise.resolve()
      },
    },
    configurable: true,
  })

  try {
    render(
      <UtilityView
        kind="agent-tools"
        status={demoStatus}
        sourceJobs={[]}
        query={contextBundle.query}
        answer={answerResponse}
        evidence={contextBundle.evidence}
        loading={false}
        error=""
        contextBundle={contextBundle}
        contextLoading={false}
        contextError=""
        contextTokens={contextBundle.metrics.estimated_tokens}
        desktopAvailable={false}
        onSearchFocus={() => {}}
        onRetrieveContext={() => {}}
        onOpenSettings={() => {}}
        onOpenProject={() => {}}
      />
    )

    fireEvent.click(screen.getByRole('button', { name: 'Copy MCP-equivalent context' }))
    await waitFor(() => expect(screen.getByText('Context copied')).toBeTruthy())
    expect(copiedText).toBe(contextBundle.context)
  } finally {
    Object.defineProperty(navigator, 'clipboard', { value: originalClipboard, configurable: true })
  }
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
  expect(screen.getByText(/Merge short-lived changes into main/)).toBeTruthy()
  expect(screen.getByText('How do releases work?')).toBeTruthy()
})

test('utility actions use the shared token-backed button primitive', () => {
  render(
    <UtilityView
      kind="help"
      status={demoStatus}
      sourceJobs={[]}
      query=""
      answer={null}
      evidence={[]}
      loading={false}
      error=""
      contextBundle={null}
      contextLoading={false}
      contextError=""
      contextTokens={0}
      desktopAvailable
      onSearchFocus={() => {}}
      onRetrieveContext={() => {}}
      onOpenSettings={() => {}}
      onOpenProject={() => {}}
    />
  )

  const openProject = screen.getByRole('button', { name: 'Open project page' })
  expect(openProject.getAttribute('data-slot')).toBe('button')
  expect(openProject.className).toContain('bg-secondary')
})

test('shadcn conversations compose cards and actions from the generated primitives', () => {
  render(
    <UtilityView
      kind="conversations"
      status={demoStatus}
      sourceJobs={[]}
      query={answerResponse.query}
      answer={answerResponse}
      evidence={demoEvidence}
      loading={false}
      error=""
      contextBundle={null}
      contextLoading={false}
      contextError=""
      contextTokens={0}
      desktopAvailable
      onSearchFocus={() => {}}
      onRetrieveContext={() => {}}
      onOpenSettings={() => {}}
      onOpenProject={() => {}}
    />
  )

  expect(document.querySelector('[data-m7-utility-view="conversations"]')).toBeTruthy()
  expect(document.querySelector('[data-slot="card"]')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Search the brain' }).getAttribute('data-slot')).toBe(
    'button'
  )
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
