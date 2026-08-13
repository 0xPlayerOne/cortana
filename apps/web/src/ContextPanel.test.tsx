import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import type { AnswerResponse, ContextBundle, Evidence } from './types'
import { ContextPanel } from './components/ContextPanel'

afterEach(cleanup)

const baseEvidence: Evidence[] = [
  {
    chunk_id: 'chunk-1',
    source: 'work-code',
    source_id: 'code-1',
    title: 'Deployment playbook',
    uri: null,
    content: 'A sample result.',
    score: 0.74,
    semantic_rank: 0,
    lexical_rank: 0,
    updated_at: '2026-01-01T00:00:00Z',
  },
]

const baseAnswer: AnswerResponse = {
  query: 'How do releases work?',
  answer: 'Context copy scenario.',
  evidence: baseEvidence,
  mode: 'synthesized',
  cached: false,
  latency_ms: 120,
  warnings: [],
  plan: {
    queries: ['How do releases work?'],
    model_generated: false,
  },
}

function renderPanel(overrides: Partial<ContextBundle | null> = {}) {
  const contextBundle: ContextBundle | null =
    overrides === null
      ? null
      : {
          query: 'How do releases work?',
          context: 'server-context',
          evidence: baseEvidence,
          metrics: {
            retrieved: 4,
            included: 4,
            omitted: 0,
            estimated_tokens: 512,
            max_tokens: 8000,
          },
          ...overrides,
        }

  return render(
    <ContextPanel
      open
      query="How do releases work?"
      evidence={baseEvidence}
      answer={baseAnswer}
      selected={0}
      status={null}
      context="preview context"
      contextTokens={128}
      serverContext={contextBundle}
      contextLoading={false}
      contextError=""
      onRetrieveContext={() => {}}
      onSelect={() => {}}
      onClose={() => {}}
    />
  )
}

test('Context panel copy action surfaces failures instead of failing silently', async () => {
  const originalClipboard = navigator.clipboard
  Object.defineProperty(navigator, 'clipboard', {
    value: {
      writeText: () => Promise.reject(new Error('clipboard blocked')),
    },
    configurable: true,
  })
  renderPanel({ context: 'server context' })
  fireEvent.click(screen.getByRole('button', { name: 'Copy agent context' }))
  await waitFor(() => expect(screen.getByText('clipboard blocked')).toBeTruthy())
  expect(screen.getByRole('alert').textContent).toBe('clipboard blocked')
  Object.defineProperty(navigator, 'clipboard', { value: originalClipboard, configurable: true })
})

test('Context panel copy action confirms successful copy', async () => {
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

  renderPanel()
  const button = screen.getByRole('button', { name: 'Copy agent context' })
  expect(button.getAttribute('title')).toBeNull()
  expect(button.getAttribute('data-tooltip')).toBe('Copy agent context')
  fireEvent.click(button)
  await waitFor(() => expect(screen.getByText('Context copied')).toBeTruthy())
  expect(copiedText).toBe('server-context')

  Object.defineProperty(navigator, 'clipboard', { value: originalClipboard, configurable: true })
})

test('Context panel copy falls back when the async clipboard API is unavailable', async () => {
  const originalClipboard = navigator.clipboard
  const originalExecCommand = document.execCommand
  let copiedCommand = ''
  Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true })
  Object.defineProperty(document, 'execCommand', {
    value: (command: string) => {
      copiedCommand = command
      return true
    },
    configurable: true,
  })

  try {
    renderPanel()
    fireEvent.click(screen.getByRole('button', { name: 'Copy agent context' }))
    await waitFor(() => expect(screen.getByText('Context copied')).toBeTruthy())
    expect(copiedCommand).toBe('copy')
  } finally {
    Object.defineProperty(navigator, 'clipboard', { value: originalClipboard, configurable: true })
    Object.defineProperty(document, 'execCommand', {
      value: originalExecCommand,
      configurable: true,
    })
  }
})
