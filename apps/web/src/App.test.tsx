import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { demoStatus } from './demo'
import {
  answerResponse,
  canonicalDocument,
  firstDocumentsPage,
  secondDocumentsPage,
} from './test/fixtures'
import type { AnswerResponse, BrainDocumentPage, BrainStatus } from './types'

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

const state = {
  status: demoStatus as BrainStatus,
  documents: ((_project, _source, _query, cursor) =>
    Promise.resolve(cursor ? secondDocumentsPage : firstDocumentsPage)) as (
    project?: string,
    source?: string,
    query?: string,
    cursor?: string
  ) => Promise<BrainDocumentPage>,
  documentsCalls: [] as DocumentsCall[],
  answer: null as (() => Promise<AnswerResponse>) | null,
  document: canonicalDocument,
}

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: false,
  isDemoMode: false,
  getStatus: () => Promise.resolve(state.status),
  getDocuments: (project?: string, source?: string, query?: string, cursor?: string) => {
    state.documentsCalls.push({ project, source, query, cursor })
    return state.documents(project, source, query, cursor)
  },
  getAnswer: () =>
    state.answer ? state.answer() : Promise.reject(new Error('Answer request failed (503)')),
  getDocument: (id: string) => Promise.resolve({ ...state.document, id }),
  getContext: () => Promise.reject(new Error('Context retrieval failed (503)')),
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
