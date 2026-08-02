import type { BrainStatus, ConfiguredSourceSummary, SourceSyncSummary } from './types'

export function isLoopbackUrl(value: string): boolean {
  try {
    const hostname = new URL(value).hostname.toLowerCase()
    return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]'
  } catch {
    return false
  }
}

export function embeddingLabel(fingerprint: string | null | undefined): string {
  if (!fingerprint) return '—'
  const dimensionSeparator = fingerprint.lastIndexOf(':')
  if (dimensionSeparator <= 0 || dimensionSeparator === fingerprint.length - 1) {
    return fingerprint
  }
  const dimension = fingerprint.slice(dimensionSeparator + 1)
  if (!/^\d+$/.test(dimension)) return fingerprint
  const modelSeparator = fingerprint.lastIndexOf(':', dimensionSeparator - 1)
  if (modelSeparator <= 0 || modelSeparator === dimensionSeparator - 1) {
    return fingerprint
  }
  const model = fingerprint.slice(modelSeparator + 1, dimensionSeparator)
  return model ? `${model} · ${dimension}d` : fingerprint
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const remainder = seconds % 60
  if (minutes < 60) return remainder === 0 ? `${minutes}m` : `${minutes}m ${remainder}s`
  const hours = Math.floor(minutes / 60)
  const remainingMinutes = minutes % 60
  return remainingMinutes === 0 ? `${hours}h` : `${hours}h ${remainingMinutes}m`
}

/**
 * Return bounded wall-clock telemetry for a persisted source sync. The API
 * records a budget even while a run is active, so the UI can show progress
 * without inventing document counts that are only known after completion.
 */
export function describeSyncRunProgress(
  run: SourceSyncSummary,
  nowMilliseconds = Date.now()
): string {
  const started = Date.parse(run.started_at)
  if (!Number.isFinite(started)) return 'elapsed unavailable'
  const completed = run.completed_at ? Date.parse(run.completed_at) : nowMilliseconds
  const end = Number.isFinite(completed) ? completed : nowMilliseconds
  const elapsed = Math.max(0, Math.floor((end - started) / 1000))
  const budget = Math.max(0, Math.floor(run.budget_seconds))
  return budget > 0
    ? `${formatDuration(elapsed)} / ${formatDuration(budget)}`
    : `${formatDuration(elapsed)} elapsed`
}

export type OperationalSource = {
  name: string
  source: string
  project: string
  kind: string
  enabled: boolean
  documents: number
  chunks: number
  latest_updated_at: string | null
  sync: SourceSyncSummary | null
  authorization?: ConfiguredSourceSummary['authorization']
  validation?: ConfiguredSourceSummary['validation']
}

export function operationalSources(status: BrainStatus | null): OperationalSource[] {
  if (!status) return []
  const indexed = new Map(
    status.sources.map((source) => [`${source.project}\0${source.source}`, source])
  )
  const runs = new Map(status.sync_runs.map((run) => [`${run.project}\0${run.source}`, run]))
  const configured: OperationalSource[] = status.ingestion.configured_sources.map((source) => {
    const key = `${source.project}\0${source.source}`
    const stats = indexed.get(key)
    indexed.delete(key)
    return {
      ...source,
      documents: stats?.documents ?? 0,
      chunks: stats?.chunks ?? 0,
      latest_updated_at: stats?.latest_updated_at ?? null,
      sync: runs.get(key) ?? null,
    }
  })
  for (const [key, source] of indexed) {
    configured.push({
      name: source.source,
      source: source.source,
      project: source.project,
      kind: 'indexed',
      enabled: false,
      documents: source.documents,
      chunks: source.chunks,
      latest_updated_at: source.latest_updated_at,
      sync: runs.get(key) ?? null,
    })
  }
  return configured
}

export function sourceHealth(source: OperationalSource) {
  if (!source.enabled)
    return { state: 'disabled', label: 'Disabled; existing index remains queryable' }
  if (source.authorization && source.authorization.method !== 'none') {
    if (!source.authorization.authorized) {
      return {
        state: 'warning',
        label:
          source.authorization.method === 'google_oauth' && source.authorization.setup_required
            ? 'Google OAuth setup required'
            : source.authorization.method === 'google_oauth'
              ? 'Google token authorization required'
              : 'Source token required for connector authorization',
      }
    }
    if (source.authorization.setup_required) {
      return {
        state: 'warning',
        label:
          source.authorization.method === 'google_oauth'
            ? 'Google OAuth needs setup'
            : 'Token source setup required',
      }
    }
  }
  if (!source.sync) {
    if (source.validation?.status === 'succeeded') {
      return {
        state: 'healthy',
        label: `Connector validated ${new Date(source.validation.validated_at).toLocaleString()}`,
      }
    }
    if (source.validation?.status === 'failed') {
      return {
        state: 'failed',
        label: `Validation failed ${new Date(source.validation.validated_at).toLocaleString()}`,
      }
    }
    return { state: 'never', label: 'Enabled; connector not yet validated' }
  }
  const completed = source.sync.completed_at
    ? new Date(source.sync.completed_at).toLocaleString()
    : 'in progress'
  if (source.sync.status === 'succeeded') {
    return { state: 'healthy', label: `Last sync succeeded ${completed}` }
  }
  if (source.sync.status === 'running') {
    return {
      state: 'running',
      label: `Sync started ${new Date(source.sync.started_at).toLocaleString()}`,
    }
  }
  return {
    state: 'failed',
    label: `Last sync ${source.sync.status.replace('_', ' ')} ${completed}`,
  }
}
