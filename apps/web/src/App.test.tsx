import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { demoEvidence, demoStatus } from './demo'
import {
  answerResponse,
  canonicalDocument,
  firstDocumentsPage,
  secondDocumentsPage,
} from './test/fixtures'
import type {
  AnswerResponse,
  BrainDocument,
  BrainDocumentPage,
  BrainStatus,
  ContextBundle,
} from './types'

afterEach(cleanup)

// Capture the real api module, then register a mock that delegates every export
// to a mutable state object so each test controls the network boundary.
const realApi = await import('./api')

type DocumentsCall = {
  project: string | undefined
  source: string | undefined
  query: string | undefined
  cursor: string | undefined
}

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((next, fail) => {
    resolve = next
    reject = fail
  })
  return { promise, resolve, reject }
}

const state = {
  status: demoStatus as BrainStatus,
  statusRequest: null as (() => Promise<BrainStatus>) | null,
  documents: ((_project, _source, _query, cursor) =>
    Promise.resolve(cursor ? secondDocumentsPage : firstDocumentsPage)) as (
    project?: string,
    source?: string,
    query?: string,
    cursor?: string
  ) => Promise<BrainDocumentPage>,
  documentsCalls: [] as DocumentsCall[],
  answer: null as
    | ((
        query?: string,
        project?: string,
        source?: string,
        signal?: AbortSignal
      ) => Promise<AnswerResponse>)
    | null,
  getContext: null as
    | ((
        query: string,
        project?: string,
        source?: string,
        signal?: AbortSignal
      ) => Promise<ContextBundle>)
    | null,
  getDocument: null as ((id: string, signal?: AbortSignal) => Promise<BrainDocument>) | null,
  document: canonicalDocument,
}

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: false,
  isDemoMode: false,
  getStatus: () => (state.statusRequest ? state.statusRequest() : Promise.resolve(state.status)),
  getDocuments: (project?: string, source?: string, query?: string, cursor?: string) => {
    state.documentsCalls.push({ project, source, query, cursor })
    return state.documents(project, source, query, cursor)
  },
  getAnswer: (query?: string, project?: string, source?: string, signal?: AbortSignal) =>
    state.answer
      ? state.answer(query, project, source, signal)
      : Promise.reject(new Error('Answer request failed (503)')),
  getDocument: (id: string, signal?: AbortSignal) =>
    state.getDocument ? state.getDocument(id, signal) : Promise.resolve({ ...state.document, id }),
  getContext: (query: string, project?: string, source?: string, signal?: AbortSignal) =>
    state.getContext
      ? state.getContext(query, project, source, signal)
      : Promise.reject(new Error('Context retrieval failed (503)')),
  getDesktopSettings: () => Promise.reject(new Error('Settings are available in Cortana Desktop')),
  getDesktopInfo: () =>
    Promise.reject(new Error('Desktop information is available in Cortana Desktop')),
}))

const { App } = await import('./App')

test('workspace and source selection scopes the source tree and document requests', async () => {
  state.documentsCalls = []
  render(<App />)

  // Initial load: every configured source is visible.
  await waitFor(() => expect(screen.getByRole('button', { name: /^work-code/ })).toBeTruthy())
  expect(screen.getByRole('button', { name: /^personal-notes/ })).toBeTruthy()

  // Selecting a workspace filters the source tree to that project.
  fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } })
  await waitFor(() => expect(screen.queryByRole('button', { name: /^personal-notes/ })).toBeNull())
  expect(screen.getByRole('button', { name: /^team-slack/ })).toBeTruthy()
  expect(state.documentsCalls.at(-1)).toEqual({
    project: 'work',
    source: undefined,
    query: undefined,
    cursor: undefined,
  })

  // Selecting a source inside the workspace presses it and rescopes documents.
  const workCode = screen.getByRole('button', { name: /^work-code/ })
  fireEvent.click(workCode)
  await waitFor(() => expect(workCode.getAttribute('aria-pressed')).toBe('true'))
  expect(state.documentsCalls.at(-1)).toEqual({
    project: 'work',
    source: 'work-code',
    query: undefined,
    cursor: undefined,
  })

  // Clicking the same source again toggles the selection off.
  fireEvent.click(workCode)
  await waitFor(() => expect(workCode.getAttribute('aria-pressed')).toBe('false'))
  expect(state.documentsCalls.at(-1)?.source).toBeUndefined()

  // Back to "All workspaces": a source from another project then switches
  // both the workspace scope and the selected source.
  fireEvent.change(screen.getByRole('combobox'), { target: { value: '' } })
  await waitFor(() => expect(screen.getByRole('button', { name: /^personal-notes/ })).toBeTruthy())
  fireEvent.click(screen.getByRole('button', { name: /^personal-notes/ }))
  await waitFor(() =>
    expect(
      screen.getByRole('button', { name: /^personal-notes/ }).getAttribute('aria-pressed')
    ).toBe('true')
  )
  expect(state.documentsCalls.at(-1)).toEqual({
    project: 'personal',
    source: 'personal-notes',
    query: undefined,
    cursor: undefined,
  })

  // "All workspaces" clears the workspace and the source selection.
  fireEvent.change(screen.getByRole('combobox'), { target: { value: '' } })
  await waitFor(() =>
    expect(
      screen.getByRole('button', { name: /^personal-notes/ }).getAttribute('aria-pressed')
    ).toBe('false')
  )
  expect(state.documentsCalls.at(-1)?.project).toBeUndefined()
  expect(state.documentsCalls.at(-1)?.source).toBeUndefined()
})

test('document filter bounds requests to the native query byte budget', async () => {
  const longUnicodeQuery = 'é'.repeat(200)
  const expectedQuery = (() => {
    const parts: string[] = []
    let bytes = 0
    for (const token of longUnicodeQuery) {
      const tokenBytes = new TextEncoder().encode(token).length
      if (bytes + tokenBytes > 256) break
      bytes += tokenBytes
      parts.push(token)
    }
    return parts.join('')
  })()

  state.documentsCalls = []
  render(<App />)

  fireEvent.click(screen.getByRole('button', { name: 'Open sources' }))
  const filter = await screen.findByRole('textbox', { name: 'Filter documents' })
  fireEvent.change(filter, { target: { value: longUnicodeQuery } })

  await waitFor(() => expect(state.documentsCalls.at(-1)?.query).toBe(expectedQuery))
  const lastQuery = state.documentsCalls.at(-1)?.query ?? ''
  expect(new TextEncoder().encode(lastQuery).length).toBeLessThanOrEqual(256)
  expect(new TextEncoder().encode(longUnicodeQuery).length).toBeGreaterThan(256)
  expect(lastQuery).toBe(expectedQuery)
  expect(new TextEncoder().encode(lastQuery).length).toBeLessThan(
    new TextEncoder().encode(longUnicodeQuery).length
  )

  // Unicode characters should be counted as UTF-8 bytes, not code points.
  expect(lastQuery.length).toBeLessThan(longUnicodeQuery.length)
})

test('changing workspace clears evidence from the previous security scope', async () => {
  state.answer = () => Promise.resolve({ ...answerResponse, query: 'private release query' })

  try {
    render(<App />)
    const input = screen.getByLabelText('Search your knowledge')
    fireEvent.change(input, { target: { value: 'private release query' } })
    fireEvent.submit(input.closest('form')!)

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1, name: 'private release query' })).toBeTruthy()
    )

    fireEvent.click(screen.getByRole('button', { name: 'Open sources' }))
    fireEvent.change(await screen.findByRole('combobox'), { target: { value: 'work' } })

    await waitFor(() =>
      expect(screen.queryByRole('heading', { level: 1, name: 'private release query' })).toBeNull()
    )
    expect(screen.getByText('Choose a document')).toBeTruthy()
  } finally {
    state.answer = null
  }
})

test('keyset pagination appends the next page and document selection opens the canonical view', async () => {
  state.documentsCalls = []
  render(<App />)

  // First keyset page renders; the explicit load-more action is available.
  await waitFor(() =>
    expect(screen.getByRole('option', { name: /How do releases work/ })).toBeTruthy()
  )
  expect(screen.getByRole('option', { name: /Deployment playbook/ })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Load next page' })).toBeTruthy()
  expect(screen.getByText('2 loaded')).toBeTruthy()

  // Loading the next keyset page appends the new document and consumes the cursor.
  fireEvent.click(screen.getByRole('button', { name: 'Load next page' }))
  await waitFor(() => expect(screen.getByText('3 loaded')).toBeTruthy())
  expect(state.documentsCalls.at(-1)?.cursor).toBe('cursor-2')
  expect(screen.getByRole('option', { name: /Slack: #releases/ })).toBeTruthy()
  // The cursor is consumed, so the load-more action disappears.
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Load next page' })).toBeNull())
  await waitFor(() => expect(screen.queryByText('Loading more…')).toBeNull())

  // Selecting a document fetches the canonical record and renders it.
  fireEvent.click(screen.getByRole('option', { name: /Deployment playbook/ }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Deployment playbook' })).toBeTruthy()
  )
  expect(
    screen.getByRole('option', { name: /Deployment playbook/ }).getAttribute('aria-selected')
  ).toBe('true')
  expect(screen.getByText(/^work · work-code · /)).toBeTruthy()
  expect(screen.getByText(/Promote staging only after unit, integration/)).toBeTruthy()
  expect(screen.getByText(/Observe the deployment before closing the release/)).toBeTruthy()
  expect(screen.getByText('Backlinks')).toBeTruthy()
  expect(screen.getByRole('button', { name: /Deployment rollback checklist/ })).toBeTruthy()
  expect(screen.getByText('Surrounding documents')).toBeTruthy()
  expect(screen.getByText(/Canonical content protected by workspace ACLs/)).toBeTruthy()
  // Document tabs switch to the canonical document view.
  expect(screen.getByRole('tab', { name: /Document/ })).toBeTruthy()

  // The document action is local and explicit rather than a dead decorative button.
  const favorite = screen.getByRole('button', { name: 'Add favorite' })
  expect(favorite.getAttribute('aria-pressed')).toBe('false')
  fireEvent.click(favorite)
  expect(screen.getByRole('button', { name: 'Remove favorite' }).getAttribute('aria-pressed')).toBe(
    'true'
  )

  fireEvent.click(screen.getByRole('button', { name: /Deployment rollback checklist/ }))
  await waitFor(() => expect(screen.getByRole('button', { name: 'Add favorite' })).toBeTruthy())
})

test('settings navigation explains the desktop-only view in web mode', async () => {
  render(<App />)
  await waitFor(() => expect(screen.getByRole('button', { name: 'Settings' })).toBeTruthy())

  fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Desktop settings' })).toBeTruthy()
  )
  expect(screen.getByText(/Install Cortana Desktop to manage local models/)).toBeTruthy()
  // The desktop-only updates shortcut must not appear in the web footer.
  expect(screen.queryByRole('button', { name: /Updates/ })).toBeNull()

  // Navigating back returns to the knowledge workspace.
  fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
  await waitFor(() => expect(screen.getByLabelText('Search your knowledge')).toBeTruthy())
})

test('a failed search surfaces the error state and Try again recovers', async () => {
  state.answer = () => Promise.reject(new Error('Answer request failed (503)'))
  render(<App />)
  await waitFor(() => expect(screen.getByText('Choose a document')).toBeTruthy())

  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'release cadence' } })
  fireEvent.submit(input.closest('form')!)

  await waitFor(() => expect(screen.getByText('Cortana could not reach the brain')).toBeTruthy())
  expect(screen.getByText(/Answer request failed \(503\)/)).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy()

  // The same query succeeds on retry and the synthesized answer renders.
  state.answer = () => Promise.resolve(answerResponse)
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'release cadence' })).toBeTruthy()
  )
  expect(screen.getByText(/Promote short-lived changes through staging/)).toBeTruthy()
  expect(screen.getByText('Read-only preview')).toBeTruthy()
  expect(screen.getByText('4 cited passages')).toBeTruthy()
})

test('stale search responses do not overwrite the latest query', async () => {
  const oldSearch = deferred<AnswerResponse>()
  const freshSearch = deferred<AnswerResponse>()
  state.answer = (query?: string) => {
    if (query === 'first query') return oldSearch.promise
    if (query === 'latest query') return freshSearch.promise
    return Promise.resolve(answerResponse)
  }

  render(<App />)
  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'first query' } })
  fireEvent.submit(input.closest('form')!)
  fireEvent.change(input, { target: { value: 'latest query' } })
  fireEvent.submit(input.closest('form')!)

  freshSearch.resolve({
    ...answerResponse,
    query: 'latest query',
    answer: 'Fresh answer content',
    evidence: demoEvidence,
  })
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'latest query' })).toBeTruthy()
  )
  oldSearch.resolve({
    ...answerResponse,
    query: 'first query',
    answer: 'Stale answer content',
    evidence: demoEvidence,
  })

  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'latest query' })).toBeTruthy()
  )
  expect(screen.getByText('Fresh answer content')).toBeTruthy()
  expect(screen.queryByText('Stale answer content')).toBeNull()
})

test('initial status completion does not hide a search that started first', async () => {
  const status = deferred<BrainStatus>()
  const answer = deferred<AnswerResponse>()
  state.statusRequest = () => status.promise
  state.answer = () => answer.promise

  try {
    render(<App />)
    const input = screen.getByLabelText('Search your knowledge')
    fireEvent.change(input, { target: { value: 'status race query' } })
    fireEvent.submit(input.closest('form')!)

    // Health can arrive after the query has started, but the query remains
    // visibly in flight until its own response settles.
    status.resolve(demoStatus)
    await waitFor(() => expect(screen.getByText('Searching your brain')).toBeTruthy())

    answer.resolve({ ...answerResponse, query: 'status race query' })
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'status race query' })).toBeTruthy()
    )
  } finally {
    state.statusRequest = null
    state.answer = null
  }
})

test('stale document responses do not overwrite the currently selected document', async () => {
  const staleDocument = deferred<BrainDocument>()
  const freshDocument = deferred<BrainDocument>()
  const first = firstDocumentsPage.documents[0]
  const second = firstDocumentsPage.documents[1]
  state.getDocument = (id: string) => {
    if (id === first.id) return staleDocument.promise
    if (id === second.id) return freshDocument.promise
    return Promise.resolve({ ...canonicalDocument, id })
  }

  render(<App />)
  await waitFor(() =>
    expect(screen.getByRole('option', { name: /How do releases work/ })).toBeTruthy()
  )

  fireEvent.click(screen.getByRole('option', { name: /How do releases work/ }))
  fireEvent.click(screen.getByRole('option', { name: /Deployment playbook/ }))

  freshDocument.resolve({
    ...canonicalDocument,
    id: second.id,
    title: 'Freshly selected document',
    source: second.source,
    source_id: second.source_id,
    updated_at: second.updated_at,
    project: second.project,
  })

  await waitFor(() =>
    expect(
      screen.getByRole('heading', { level: 1, name: 'Freshly selected document' })
    ).toBeTruthy()
  )

  staleDocument.resolve({
    ...canonicalDocument,
    id: first.id,
    title: 'Stale document result',
    source: first.source,
    source_id: first.source_id,
    updated_at: first.updated_at,
    project: first.project,
  })

  await waitFor(() =>
    expect(
      screen.getByRole('heading', { level: 1, name: 'Freshly selected document' })
    ).toBeTruthy()
  )
  expect(screen.queryByText('Stale document result')).toBeNull()
})

test('scope-changed context request does not overwrite newer state', async () => {
  const oldContext = deferred<ContextBundle>()
  const newContext = deferred<ContextBundle>()
  const oldBundle: ContextBundle = {
    query: 'first context query',
    context: 'Old context',
    evidence: [
      {
        ...demoEvidence[1],
        chunk_id: 'stale-context-chunk',
        title: 'Stale context evidence',
      },
    ],
    metrics: {
      retrieved: 1,
      included: 1,
      omitted: 0,
      estimated_tokens: 11,
      max_tokens: 8000,
    },
  }
  const newBundle: ContextBundle = {
    query: 'first context query',
    context: 'Fresh context',
    evidence: [
      {
        ...demoEvidence[0],
        chunk_id: 'fresh-context-chunk',
        title: 'Fresh context evidence',
      },
    ],
    metrics: {
      retrieved: 1,
      included: 1,
      omitted: 0,
      estimated_tokens: 19,
      max_tokens: 8000,
    },
  }

  state.answer = () => Promise.resolve(answerResponse)
  state.getContext = (_query, project?: string) => {
    if (project === 'work') return newContext.promise
    return oldContext.promise
  }

  render(<App />)
  const input = screen.getByLabelText('Search your knowledge')
  fireEvent.change(input, { target: { value: 'first context query' } })
  fireEvent.submit(input.closest('form')!)

  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'first context query' })).toBeTruthy()
  )

  fireEvent.click(screen.getByRole('button', { name: 'Agent tools' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Agent tools' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Retrieve context' }))

  fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'first context query' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Open sources' }))
  const workspaceSelect = await screen.findByRole('combobox')
  fireEvent.change(workspaceSelect, { target: { value: 'work' } })

  fireEvent.click(screen.getByRole('button', { name: 'Agent tools' }))
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1, name: 'Agent tools' })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: 'Retrieve context' }))

  newContext.resolve(newBundle)
  oldContext.resolve(oldBundle)

  await waitFor(() => expect(screen.getByText('Fresh context evidence')).toBeTruthy())
  expect(screen.queryByText('Stale context evidence')).toBeNull()
})
