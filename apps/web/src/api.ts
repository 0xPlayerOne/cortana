import { demoEvidence, demoStatus } from './demo'
import { buildAgentContext, estimateTokens } from './context'
import type { BrainStatus, ContextBundle } from './types'

export const isDemoMode = new URLSearchParams(window.location.search).has('demo')

export async function getStatus(signal?: AbortSignal): Promise<BrainStatus> {
  if (isDemoMode) return demoStatus
  const response = await fetch('/v1/status', { signal })
  if (!response.ok) throw new Error(`Status request failed (${response.status})`)
  return (await response.json()) as BrainStatus
}

export async function getContext(
  query: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<ContextBundle> {
  if (isDemoMode) {
    const evidence = demoEvidence
      .filter((item) => !source || item.source === source)
      .sort((left, right) => right.score - left.score)
    const context = buildAgentContext(query, evidence)
    return {
      query,
      context,
      evidence,
      metrics: {
        retrieved: evidence.length,
        included: evidence.length,
        omitted: 0,
        estimated_tokens: estimateTokens(context),
        max_tokens: 8000,
      },
    }
  }
  const response = await fetch('/v1/context', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      project: project || null,
      source: source || null,
      limit: 20,
      max_tokens: 8000,
    }),
    signal,
  })
  if (!response.ok) throw new Error(`Context retrieval failed (${response.status})`)
  return (await response.json()) as ContextBundle
}
