import { demoEvidence, demoStatus } from './demo'
import { buildAgentContext, estimateTokens } from './context'
import type { AnswerResponse, BrainStatus, ContextBundle } from './types'

export const isDemoMode = new URLSearchParams(window.location.search).has('demo')

export async function getStatus(signal?: AbortSignal): Promise<BrainStatus> {
  if (isDemoMode) return demoStatus
  const response = await authorizedFetch('/v1/status', { signal })
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
  const response = await authorizedFetch('/v1/context', {
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

export async function getAnswer(
  query: string,
  project?: string,
  source?: string,
  signal?: AbortSignal
): Promise<AnswerResponse> {
  if (isDemoMode) {
    const evidence = demoEvidence
      .filter((item) => !source || item.source === source)
      .sort((left, right) => right.score - left.score)
    return {
      query,
      answer:
        'Promote short-lived changes through staging after the full test and security suite passes, then monitor the release and roll back if health regresses [1]. Keep an explicit rollback owner in the checklist [2].',
      evidence,
      plan: {
        queries: [query, 'release promotion staging checks', 'rollback owner health regression'],
        model_generated: true,
      },
      mode: 'synthesized',
      cached: false,
      latency_ms: 184,
      warnings: [],
    }
  }
  const response = await authorizedFetch('/v1/answer', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      project: project || null,
      source: source || null,
    }),
    signal,
  })
  if (!response.ok) throw new Error(`Answer request failed (${response.status})`)
  return (await response.json()) as AnswerResponse
}

async function authorizedFetch(input: string, init: RequestInit): Promise<Response> {
  const request = (token: string | null) => {
    const headers = new Headers(init.headers)
    if (token) headers.set('Authorization', `Bearer ${token}`)
    return fetch(input, { ...init, headers })
  }
  const current = window.sessionStorage.getItem('cortana_api_token')
  let response = await request(current)
  if (response.status !== 401) return response

  window.sessionStorage.removeItem('cortana_api_token')
  const token = window.prompt('Enter the Cortana access token')
  if (!token) return response
  window.sessionStorage.setItem('cortana_api_token', token)
  response = await request(token)
  if (response.status === 401) window.sessionStorage.removeItem('cortana_api_token')
  return response
}
