import type { BrainStatus, ConfiguredSourceSummary, SourceSyncSummary } from './types'

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
            ? 'Google OAuth client setup required'
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
