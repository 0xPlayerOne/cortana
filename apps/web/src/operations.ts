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
  sync_freshness_hours: number | null
  max_documents: number
  max_bytes: number
  max_duration_seconds: number
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
      sync_freshness_hours: status.ingestion.sync_freshness_hours ?? null,
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
      sync_freshness_hours: status.ingestion.sync_freshness_hours ?? null,
      max_documents: 0,
      max_bytes: 0,
      max_duration_seconds: 0,
    })
  }
  return configured
}

export function validationCoversConfiguredBudget(
  source: OperationalSource
): source is OperationalSource & {
  validation: NonNullable<OperationalSource['validation']>
} {
  if (!source.validation) return false
  if (source.validation.status !== 'succeeded') return false
  if (source.validation.fresh === false) return false
  // Only an explicit complete validation can authorize recurring or
  // full-corpus sync. Bounded samples and legacy records whose completeness is
  // unknown must be revalidated, even when their numeric budgets match.
  if (source.validation.complete !== true) return false

  const hasSufficientDocuments = source.validation.max_documents >= source.max_documents
  const hasSufficientBytes = source.validation.max_bytes >= source.max_bytes
  const hasSufficientDuration = source.validation.max_seconds >= source.max_duration_seconds
  return hasSufficientDocuments && hasSufficientBytes && hasSufficientDuration
}

/**
 * Explicit warning for a validation that only sampled the source. Even when
 * its numeric budgets match the configured limits, a bounded sample cannot
 * authorize recurring or full-corpus syncs.
 */
function boundedSampleWarning(
  validation: NonNullable<OperationalSource['validation']>,
  lead: string
): string {
  return `${lead} validation was a bounded sample (${new Date(validation.validated_at).toLocaleString()}) and cannot authorize recurring sync; re-validate the complete source`
}

function validationCompletenessWarning(
  validation: NonNullable<OperationalSource['validation']>,
  lead: string
): string {
  if (validation.complete === false) return boundedSampleWarning(validation, lead)
  return `${lead} validation completeness is unknown (${new Date(validation.validated_at).toLocaleString()}) and cannot authorize recurring sync; re-validate the complete source`
}

export function sourceHealth(source: OperationalSource, nowMilliseconds = Date.now()) {
  if (!source.enabled)
    return { state: 'disabled', label: 'Disabled; existing index remains queryable' }
  if (source.authorization && source.authorization.method !== 'none') {
    if (!source.authorization.authorized) {
      const provider =
        source.authorization.method === 'google_oauth'
          ? 'Google'
          : source.authorization.method === 'github_oauth'
            ? 'GitHub'
            : null
      return {
        state: 'warning',
        label:
          provider && source.authorization.setup_required
            ? `${provider} OAuth setup required`
            : provider
              ? `${provider} token authorization required`
              : 'Source token required for connector authorization',
      }
    }
    if (source.authorization.setup_required) {
      return {
        state: 'warning',
        label:
          source.authorization.method === 'google_oauth'
            ? 'Google OAuth needs setup'
            : source.authorization.method === 'github_oauth'
              ? 'GitHub OAuth needs setup'
              : 'Token source setup required',
      }
    }
  }
  const validationRequiredLabel = (labelSuffix: string) =>
    `Validation ${labelSuffix}; re-validate before recurring sync`

  if (!source.sync) {
    if (source.validation?.status === 'succeeded') {
      if (source.validation.fresh === false) {
        return {
          state: 'warning',
          label: `Connector validation expired ${new Date(source.validation.validated_at).toLocaleString()}; re-validate before recurring sync`,
        }
      }
      if (source.validation.complete !== true) {
        return {
          state: 'warning',
          label: validationCompletenessWarning(source.validation, 'Connector'),
        }
      }
      if (!validationCoversConfiguredBudget(source)) {
        return {
          state: 'warning',
          label: validationRequiredLabel('does not cover the configured sync budget'),
        }
      }
      return {
        state: 'healthy',
        label: `Connector validated ${new Date(source.validation.validated_at).toLocaleString()}`,
      }
    }
    if (source.validation?.status === 'failed') {
      return {
        state: 'failed',
        label: `Validation failed${source.validation.error_category ? ` (${source.validation.error_category})` : ''} ${new Date(source.validation.validated_at).toLocaleString()}`,
      }
    }
    return {
      state: 'warning',
      label: validationRequiredLabel('not yet completed'),
    }
  }
  const completed = source.sync.completed_at
    ? new Date(source.sync.completed_at).toLocaleString()
    : 'in progress'
  if (source.sync.status === 'succeeded') {
    const freshnessHours = source.sync_freshness_hours
    const completedAt = Date.parse(source.sync.completed_at ?? '')
    if (
      freshnessHours !== null &&
      freshnessHours > 0 &&
      Number.isFinite(completedAt) &&
      nowMilliseconds - completedAt > freshnessHours * 3_600_000
    ) {
      const ageSeconds = Math.max(0, Math.floor((nowMilliseconds - completedAt) / 1000))
      return {
        state: 'warning',
        label: `Last sync succeeded ${completed}; stale after ${formatDuration(freshnessHours * 3_600)} (age ${formatDuration(ageSeconds)}); run sync`,
      }
    }
    if (source.validation && source.validation.complete !== true) {
      return {
        state: 'warning',
        label: validationCompletenessWarning(
          source.validation,
          `Last sync succeeded ${completed} but its`
        ),
      }
    }
    if (!validationCoversConfiguredBudget(source)) {
      return {
        state: 'warning',
        label: validationRequiredLabel('has not been fully validated for configured limits'),
      }
    }
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
