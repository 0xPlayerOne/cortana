import { invoke, isTauri } from '@tauri-apps/api/core'

import { demoEvidence, demoStatus } from './demo'
import { buildAgentContext, estimateTokens } from './context'
import type {
  AnswerResponse,
  BrainStatus,
  ContextBundle,
  DesktopInstallJob,
  DesktopReadiness,
  DesktopSettings,
  DesktopSettingsUpdate,
} from './types'

export const isDemoMode = new URLSearchParams(window.location.search).has('demo')
export const isDesktopApp = isTauri()

export async function getDesktopSettings(): Promise<DesktopSettings> {
  if (!isDesktopApp) throw new Error('Settings are available in Cortana Desktop')
  return invokeDesktop<DesktopSettings>('desktop_settings_get')
}

export async function saveDesktopSettings(update: DesktopSettingsUpdate): Promise<DesktopSettings> {
  if (!isDesktopApp) throw new Error('Settings are available in Cortana Desktop')
  return invokeDesktop<DesktopSettings>('desktop_settings_save', { update })
}

export async function scanDesktopReadiness(): Promise<DesktopReadiness> {
  if (!isDesktopApp) throw new Error('Readiness is available in Cortana Desktop')
  return invokeDesktop<DesktopReadiness>('desktop_readiness_scan')
}

export async function startDesktopInstaller(tool: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_start', { tool, approved: true })
}

export async function getDesktopInstaller(id: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_status', { id })
}

export async function cancelDesktopInstaller(id: string): Promise<DesktopInstallJob> {
  if (!isDesktopApp) throw new Error('Installer is available in Cortana Desktop')
  return invokeDesktop<DesktopInstallJob>('desktop_installer_cancel', { id })
}

export async function getStatus(signal?: AbortSignal): Promise<BrainStatus> {
  if (isDemoMode) return demoStatus
  if (isTauri()) return invokeDesktop<BrainStatus>('brain_status', undefined, signal)
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
  if (isTauri()) {
    return invokeDesktop<ContextBundle>(
      'brain_context',
      {
        request: {
          query,
          project: project || null,
          source: source || null,
        },
      },
      signal
    )
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
  if (isTauri()) {
    return invokeDesktop<AnswerResponse>(
      'brain_answer',
      {
        request: {
          query,
          project: project || null,
          source: source || null,
        },
      },
      signal
    )
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

async function invokeDesktop<T>(
  command: string,
  args?: Record<string, unknown>,
  signal?: AbortSignal
): Promise<T> {
  if (signal?.aborted) throw new DOMException('Request aborted', 'AbortError')
  try {
    const result = await invoke<T>(command, args)
    if (signal?.aborted) throw new DOMException('Request aborted', 'AbortError')
    return result
  } catch (caught) {
    if (caught instanceof Error) throw caught
    throw new Error(typeof caught === 'string' ? caught : 'Desktop request failed')
  }
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
