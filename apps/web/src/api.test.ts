import { afterEach, describe, expect, mock, test } from 'bun:test'

import { getGraph, getReflection, getStatus } from './api'
import type { BrainGraphPage } from './types'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('status transport', () => {
  test('preserves the bounded warm-up message for a retryable status response', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(
        new Response('Cortana is warming up; live status will be available shortly', {
          status: 503,
        })
      )
    ) as unknown as typeof fetch

    await expect(getStatus()).rejects.toThrow(
      'Cortana is warming up; live status will be available shortly'
    )
  })

  test('keeps unknown status failures generic', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(new Response('private database details', { status: 503 }))
    ) as unknown as typeof fetch

    await expect(getStatus()).rejects.toThrow('Status request failed (503)')
  })
})

describe('graph transport', () => {
  const validGraph: BrainGraphPage = {
    nodes: [
      {
        id: 'document:one',
        kind: 'document',
        label: 'One',
        project: 'work',
        source: 'work-code',
        document_id: 'one',
      },
    ],
    edges: [],
    next_cursor: null,
  }

  test('accepts a bounded graph response', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(Response.json(validGraph))
    ) as unknown as typeof fetch

    await expect(getGraph('work')).resolves.toEqual(validGraph)
  })

  test('rejects malformed, dangling, or unsafe graph records before rendering', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(
        Response.json({
          ...validGraph,
          edges: [{ source: 'document:one', target: 'document:missing', kind: 'references' }],
        })
      )
    ) as unknown as typeof fetch

    await expect(getGraph('work')).rejects.toThrow('Graph response was malformed')

    globalThis.fetch = mock(() =>
      Promise.resolve(
        Response.json({
          ...validGraph,
          edges: [
            {
              source: 'document:one',
              target: 'document:one',
              kind: 'references',
              origin: 'inferred',
              confidence: 2,
              support: 'not provenance',
            },
          ],
        })
      )
    ) as unknown as typeof fetch

    await expect(getGraph('work')).rejects.toThrow('Graph response was malformed')
  })
})

describe('reflection transport', () => {
  test('posts a bounded scoped request to the reflection endpoint', async () => {
    let requestInput: RequestInfo | URL | undefined
    let requestInit: RequestInit | undefined
    globalThis.fetch = mock((input: RequestInfo | URL, init?: RequestInit) => {
      requestInput = input
      requestInit = init
      return Promise.resolve(
        Response.json({
          contract_version: 'memory-reflection.v1',
          request_digest: 'request-digest',
          status: 'completed',
          objective: 'Review launch risk',
          project: 'work',
          memory_revision: 4,
          privacy_scope_digest: 'scope-digest',
          provider: {
            policy: 'deterministic-only',
            selected: 'deterministic',
            status: 'succeeded',
          },
          claims: [],
          patterns: [],
          tensions: [],
          recommendations: [],
          chronology: [],
          proposed_candidates: [],
          evidence_ids: [],
          metrics: {
            memories_considered: 0,
            memories_included: 0,
            evidence_considered: 0,
            evidence_included: 0,
            estimated_tokens: 0,
            canonical_memory_mutated: false,
          },
        })
      )
    }) as unknown as typeof fetch

    await getReflection('Review launch risk', 'work', 'github')

    expect(String(requestInput)).toBe('/v1/memory/reflect')
    expect(requestInit?.method).toBe('POST')
    expect(JSON.parse(String(requestInit?.body))).toEqual({
      objective: 'Review launch risk',
      project: 'work',
      memory: { limit: 32 },
      include_evidence: true,
      token_budget: 2048,
      provider_policy: 'deterministic-only',
      deadline_ms: 5000,
      source: 'github',
    })
  })
})
