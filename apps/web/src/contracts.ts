import type { ContextBundle } from './types'

export const API_CONTRACT_VERSION = 'cortana.api.v1'
export const CONTEXT_CONTRACT_VERSION = 'cortana.context.v1'
export const RETRIEVAL_CONTRACT_VERSION = 'cortana.retrieval.v1'

/** Validate the transport-safe fields required by a v1 ContextBundle. */
export function isContextBundleV1(value: ContextBundle): boolean {
  return (
    value.contract_version === CONTEXT_CONTRACT_VERSION &&
    typeof value.context_bundle_id === 'string' &&
    value.context_bundle_id.startsWith('ctx_') &&
    typeof value.canonical_digest === 'string' &&
    value.canonical_digest.length === 64 &&
    typeof value.created_at === 'string' &&
    value.token_budget !== undefined &&
    value.corpus_revision !== undefined &&
    value.retrieval_contract_version === RETRIEVAL_CONTRACT_VERSION &&
    typeof value.privacy_scope_digest === 'string' &&
    value.privacy_scope_digest.length === 64
  )
}
