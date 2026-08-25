import { describe, expect, it } from 'bun:test'

import { CONTEXT_CONTRACT_VERSION, isContextBundleV1, RETRIEVAL_CONTRACT_VERSION } from './contracts'

describe('public contract fixtures', () => {
  it('accepts the required v1 ContextBundle envelope', () => {
    expect(
      isContextBundleV1({
        contract_version: CONTEXT_CONTRACT_VERSION,
        context_bundle_id: `ctx_${'a'.repeat(64)}`,
        canonical_digest: 'a'.repeat(64),
        created_at: '2026-01-01T00:00:00.000Z',
        token_budget: 8000,
        query: 'release process',
        context: '# Cortana evidence context',
        evidence: [],
        memories: [],
        metrics: {
          retrieved: 0,
          included: 0,
          omitted: 0,
          estimated_tokens: 1,
          max_tokens: 8000,
        },
        retrieval_mode: 'hybrid',
        degradation: null,
        retrieval_warning: null,
        corpus_revision: 3,
        memory_revision: 2,
        embedding_fingerprint: 'deterministic:16',
        retrieval_contract_version: RETRIEVAL_CONTRACT_VERSION,
        privacy_scope_digest: 'b'.repeat(64),
      })
    ).toBe(true)
  })

  it('rejects a bundle that omits its pinning metadata', () => {
    expect(
      isContextBundleV1({
        query: 'release process',
        context: '',
        evidence: [],
        metrics: { retrieved: 0, included: 0, omitted: 0, estimated_tokens: 1, max_tokens: 256 },
      })
    ).toBe(false)
  })
})
