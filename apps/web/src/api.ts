import { demoEvidence, demoStatus } from './demo'
import type { BrainStatus, Evidence } from './types'

export const isDemoMode = new URLSearchParams(window.location.search).has('demo')

export async function getStatus(signal?: AbortSignal): Promise<BrainStatus> {
  if (isDemoMode) return demoStatus
  const response = await fetch('/v1/status', { signal })
  if (!response.ok) throw new Error(`Status request failed (${response.status})`)
  return (await response.json()) as BrainStatus
}

export async function searchEvidence(
  query: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<Evidence[]> {
  if (isDemoMode) {
    return demoEvidence
      .filter((item) => !source || item.source === source)
      .sort((left, right) => right.score - left.score)
  }
  const response = await fetch('/v1/search', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      project: project || null,
      source: source || null,
      limit: 20,
    }),
    signal,
  })
  if (!response.ok) throw new Error(`Search failed (${response.status})`)
  return (await response.json()) as Evidence[]
}
