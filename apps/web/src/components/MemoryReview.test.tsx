import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { MemoryReview, type MemoryReviewClient } from './MemoryReview'

afterEach(cleanup)

const candidate = {
  id: 'candidate-1',
  observation_kind: 'evidence-backed',
  content_type: 'semantic',
  retention_tier: 'durable',
  scope: 'workspace',
  project: 'work',
  title: 'Release preference',
  content: 'Prefer concise release notes.',
  source: 'agent',
  source_id: 'run-1',
  confidence: 0.91,
  importance: 0.8,
  sensitivity: 'normal',
  status: 'pending',
  acl: ['work'],
  provenance: { evidence_ids: ['document-1'] },
  expires_at: '2099-01-01T00:00:00Z',
  rejection_reason: null,
  created_at: '2026-08-25T00:00:00Z',
  updated_at: '2026-08-25T00:00:00Z',
  consolidation: {
    status: 'paused',
    decision: 'review',
    classification: 'new',
    policy_version: 'cortana.memory.consolidation.v1:abcd',
    attempts: 0,
    memory_id: null,
    last_error: null,
    updated_at: '2026-08-25T00:00:00Z',
    reason_code: 'review-required',
    explanation: 'Candidate requires review.',
    supporting_memory_ids: ['memory-1'],
  },
}

function client(): MemoryReviewClient & { actions: string[] } {
  const actions: string[] = []
  return {
    actions,
    listCandidates: (_project, query, status) =>
      Promise.resolve(
        (!query ||
          `${candidate.title} ${candidate.content}`.toLowerCase().includes(query.toLowerCase())) &&
          (!status || status === 'pending')
          ? [candidate]
          : []
      ),
    classifyCandidate: () =>
      Promise.resolve({
        candidate_id: candidate.id,
        classification: 'new',
        confidence: 0.92,
        supporting_memory_ids: ['memory-1'],
        explanation: 'A new scoped preference.',
      }),
    listDerived: () =>
      Promise.resolve({
        contract_version: 'cortana.memory-derived.v1',
        derivation_version: 'native-derived-v1',
        memory_revision: 4,
        canonical_memory_mutated: false,
        recomputed: true,
        representations: [
          {
            id: 'derived-1',
            kind: 'mental-model',
            statement: 'Concise notes are preferred',
            confidence: 0.8,
            supporting_memory_ids: ['memory-1'],
            contradicting_memory_ids: ['memory-2'],
            citation_authority: false,
          },
        ],
        relations: [],
      }),
    listCanonical: () =>
      Promise.resolve([
        {
          id: 'memory-1',
          kind: 'preference',
          project: 'work',
          title: 'Release preference',
          content: 'Prefer concise release notes.',
          confidence: 0.9,
          importance: 0.8,
          updated_at: '2026-08-25T00:00:00Z',
        },
      ]),
    act: (_id, action) => {
      actions.push(action)
      return Promise.resolve({ status: 'complete', memory_id: 'memory-1' })
    },
    setConsolidationPaused: (paused) => {
      actions.push(paused ? 'pause' : 'resume')
      return Promise.resolve()
    },
    getConsolidationState: () => Promise.resolve({ paused: false, canControl: true }),
  }
}

test('renders a bounded searchable queue with inspectable policy and provenance', async () => {
  const api = client()
  render(<MemoryReview client={api} />)

  expect(await screen.findByRole('list', { name: 'Memory candidate queue' })).toBeTruthy()
  fireEvent.click(await screen.findByRole('button', { name: /Release preference/ }))
  expect(await screen.findByText('A new scoped preference.')).toBeTruthy()
  expect(screen.getByText('cortana.memory.consolidation.v1:abcd')).toBeTruthy()
  fireEvent.click(screen.getByText('Provenance and support'))
  expect(screen.getByText(/document-1/)).toBeTruthy()
  expect(screen.getByRole('heading', { name: 'Canonical memory' })).toBeTruthy()
  expect(screen.getByRole('heading', { name: 'Derived · not canonical' })).toBeTruthy()

  fireEvent.change(screen.getByRole('searchbox', { name: 'Search memory candidates' }), {
    target: { value: 'missing' },
  })
  expect(await screen.findByText('No candidates match this view.')).toBeTruthy()
})

test('requires confirmation for canonical approval and keeps queue controls explicit', async () => {
  const api = client()
  const originalConfirm = window.confirm
  window.confirm = mock(() => true)
  render(<MemoryReview client={api} />)

  fireEvent.click(await screen.findByRole('button', { name: /Release preference/ }))
  fireEvent.click(screen.getByRole('button', { name: 'Approve canonical memory' }))
  await waitFor(() => expect(api.actions).toContain('approve'))

  fireEvent.click(screen.getByRole('button', { name: 'Pause consolidation' }))
  await waitFor(() => expect(api.actions).toContain('pause'))
  window.confirm = originalConfirm
})

test('keeps the review queue available while disabling owner-only consolidation controls', async () => {
  const api = client()
  api.getConsolidationState = () => Promise.resolve({ paused: false, canControl: false })
  render(<MemoryReview client={api} />)

  expect(await screen.findByRole('list', { name: 'Memory candidate queue' })).toBeTruthy()
  const pause = screen.getByRole('button', { name: 'Pause consolidation' })
  expect(pause.hasAttribute('disabled')).toBe(true)
  expect(pause.getAttribute('title')).toMatch(/owner/i)
})

test('surfaces bounded-response truncation instead of presenting a partial queue as complete', async () => {
  const api = client()
  api.listCandidates = () =>
    Promise.reject(
      new Error('Memory candidate review was truncated; narrow the search or status filter')
    )
  render(<MemoryReview client={api} />)

  expect((await screen.findByRole('alert')).textContent).toMatch(/truncated.*narrow/i)
})

test('reports review-only supersession without claiming a canonical write', async () => {
  const api = client()
  api.act = (_id, action) => {
    api.actions.push(action)
    return Promise.resolve({
      status: 'review',
      memory_id: null,
      decision: {
        decision: 'review',
        classification: 'contradiction',
        reason_code: 'conflict',
        explanation: 'Supersession requires review.',
      },
    })
  }
  const originalConfirm = window.confirm
  window.confirm = mock(() => true)
  render(<MemoryReview client={api} />)

  fireEvent.click(await screen.findByRole('button', { name: /Release preference/ }))
  fireEvent.click(screen.getByRole('button', { name: 'Review and supersede' }))
  expect(await screen.findByText(/remain in review; no canonical memory changed/)).toBeTruthy()
  window.confirm = originalConfirm
})

test('terminal candidates show stored outcome without reclassification or actions', async () => {
  const api = client()
  const terminal = {
    ...candidate,
    status: 'accepted',
    consolidation: {
      ...candidate.consolidation,
      status: 'complete',
      decision: 'approve',
      memory_id: 'memory-terminal',
      attempts: 2,
    },
  }
  api.listCandidates = () => Promise.resolve([terminal])
  let classifications = 0
  api.classifyCandidate = () => {
    classifications += 1
    return Promise.reject(new Error('must not classify terminal candidates'))
  }
  render(<MemoryReview client={api} />)

  fireEvent.click(await screen.findByRole('button', { name: /Release preference/ }))
  expect(await screen.findByText('memory-terminal')).toBeTruthy()
  expect(screen.getByText(/stored outcome is shown above/)).toBeTruthy()
  expect(screen.queryByRole('button', { name: 'Approve canonical memory' })).toBeNull()
  expect(classifications).toBe(0)
})
