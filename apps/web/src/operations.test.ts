import { describe, expect, test } from 'bun:test'

import { demoStatus } from './demo'
import {
  describeSyncRunProgress,
  embeddingLabel,
  isLoopbackUrl,
  operationalSources,
  sourceHealth,
  validationCoversConfiguredBudget,
} from './operations'

describe('operational source visibility', () => {
  test('includes configured disabled sources that remain indexed', () => {
    const sources = operationalSources(demoStatus)
    const code = sources.find((source) => source.name === 'work-code')

    expect(code).toBeDefined()
    expect(code?.enabled).toBe(false)
    expect(code?.documents).toBe(1105)
    expect(sourceHealth(code!).state).toBe('disabled')
  })

  test('surfaces the latest failed safety outcome', () => {
    const sources = operationalSources(demoStatus)
    const discord = sources.find((source) => source.name === 'community-discord')

    expect(discord?.sync?.status).toBe('budget_exceeded')
    expect(sourceHealth(discord!).state).toBe('failed')
  })

  test('surfaces a safe validation failure category', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'failed',
      validated_at: '2026-08-03T00:00:00Z',
      fresh: true,
      age_seconds: 60,
      documents: null,
      bytes: null,
      max_documents: 25,
      max_bytes: 1_048_576,
      max_seconds: 60,
      error: 'source validation failed',
      error_category: 'authorization',
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(sourceHealth(source!).label).toContain('(authorization)')
  })

  test('flags a stale successful sync instead of claiming it is healthy forever', () => {
    const status = structuredClone(demoStatus)
    const gmail = status.ingestion.configured_sources.find(
      (source) => source.name === 'personal-gmail'
    )!
    const source = operationalSources(status).find((item) => item.name === gmail.name)!
    const health = sourceHealth(source, Date.parse('2026-08-03T00:00:00Z'))

    expect(health.state).toBe('warning')
    expect(health.label).toContain('stale')
    expect(health.label).toContain('run sync')
  })

  test('reports elapsed sync time against the persisted safety budget', () => {
    const run = {
      ...demoStatus.sync_runs[0],
      started_at: '2026-01-01T00:00:00Z',
      completed_at: null,
      budget_seconds: 900,
    }
    expect(describeSyncRunProgress(run, Date.parse('2026-01-01T00:01:05Z'))).toBe('1m 5s / 15m')
  })

  test('surfaces authorization readiness warnings in source health', () => {
    const status = structuredClone(demoStatus)
    const slack = status.ingestion.configured_sources.find(
      (source) => source.name === 'team-slack'
    )!
    slack.authorization = {
      method: 'token',
      setup_required: true,
      authorized: false,
    }

    const source = operationalSources(status).find((item) => item.name === 'team-slack')
    const health = sourceHealth(source!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('Source token required')
  })

  test('surfaces google oauth readiness in source health', () => {
    const status = structuredClone(demoStatus)
    const gmail = status.ingestion.configured_sources.find(
      (source) => source.name === 'personal-gmail'
    )!
    gmail.authorization = {
      method: 'google_oauth',
      setup_required: true,
      authorized: false,
    }

    const source = operationalSources(status).find((item) => item.name === 'personal-gmail')
    const health = sourceHealth(source!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('Google OAuth setup required')
  })

  test('surfaces GitHub oauth readiness in source health', () => {
    const status = structuredClone(demoStatus)
    const source = status.ingestion.configured_sources.find((item) => item.name === 'work-code')!
    source.enabled = true
    source.authorization = {
      method: 'github_oauth',
      setup_required: false,
      authorized: false,
    }

    const operational = operationalSources(status).find((item) => item.name === 'work-code')
    const health = sourceHealth(operational!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('GitHub token authorization required')
  })

  test('distinguishes a validated connector from an unproven source', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      age_seconds: 60,
      documents: 45,
      bytes: 4096,
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      complete: true,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(sourceHealth(source!).state).toBe('healthy')
    expect(sourceHealth(source!).label).toContain('Connector validated')
  })

  test('flags an expired succeeded validation as needing re-validation', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-06-30T06:00:00Z',
      fresh: false,
      age_seconds: 30 * 24 * 3_600,
      documents: 45,
      bytes: 4096,
      max_documents: 100,
      max_bytes: 1_048_576,
      max_seconds: 30,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    const health = sourceHealth(source!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('expired')
    expect(health.label.toLowerCase()).toContain('re-validate')
  })

  test('treats a complete validation without freshness metadata as current', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    // A server predating the freshness fields never reports `fresh`; the
    // workspace must not invent an expiry for a complete record it cannot age.
    const legacy = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded' as const,
      validated_at: '2026-07-30T06:00:00Z',
      documents: 45,
      bytes: 4096,
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      complete: true,
      error: null,
    }
    buzz.validation = legacy

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(sourceHealth(source!).state).toBe('healthy')
  })

  test('fails closed when validation completeness is unknown', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 45,
      bytes: 4096,
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(validationCoversConfiguredBudget(source!)).toBe(false)
    const health = sourceHealth(source!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('completeness is unknown')
    expect(health.label).toContain('re-validate')
  })

  test('fails closed when the succeeded validation was a bounded sample', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 45,
      bytes: 4096,
      // Numeric budgets match the configured limits, but a bounded sample
      // never authorizes a recurring or full-corpus sync.
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      complete: false,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(validationCoversConfiguredBudget(source!)).toBe(false)
    const health = sourceHealth(source!)
    expect(health.state).toBe('warning')
    expect(health.label).toContain('bounded sample')
    expect(health.label.toLowerCase()).toContain('recurring sync')
    expect(health.label).toContain('re-validate')
  })

  test('keeps an explicitly complete validation healthy', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 45,
      bytes: 4096,
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      complete: true,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(validationCoversConfiguredBudget(source!)).toBe(true)
    expect(sourceHealth(source!).state).toBe('healthy')
  })

  test('warns after a successful sync whose validation was only a bounded sample', () => {
    const status = structuredClone(demoStatus)
    const gmail = status.ingestion.configured_sources.find(
      (source) => source.name === 'personal-gmail'
    )!
    gmail.validation = {
      source: 'personal-gmail',
      project: 'personal',
      kind: 'gmail',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 1200,
      bytes: 100_000,
      // Budgets comfortably cover the configured limits, so only the sample
      // marker stands between this source and recurrence.
      max_documents: 3_000,
      max_bytes: 268_435_456,
      max_seconds: 900,
      complete: false,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === gmail.name)
    const health = sourceHealth(source!, Date.parse('2026-07-29T15:00:00Z'))

    expect(health.state).toBe('warning')
    expect(health.label).toContain('bounded sample')
    expect(health.label.toLowerCase()).toContain('recurring sync')
    expect(health.label).toContain('re-validate')
  })

  test('warns when a successful sync used an undersized validation budget', () => {
    const status = structuredClone(demoStatus)
    const gmail = status.ingestion.configured_sources.find(
      (source) => source.name === 'personal-gmail'
    )!
    gmail.validation = {
      source: 'personal-gmail',
      project: 'personal',
      kind: 'gmail',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 1200,
      bytes: 100_000,
      max_documents: 100,
      max_bytes: 1000,
      max_seconds: 30,
      complete: true,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === gmail.name)
    const health = sourceHealth(source!, Date.parse('2026-07-29T15:00:00Z'))

    expect(health.state).toBe('warning')
    expect(health.label).toContain('re-validate before recurring sync')
    expect(health.label).toContain('configured limits')
  })

  test('warns when a successful sync has no validation for recurrence', () => {
    const status = structuredClone(demoStatus)
    const gmail = status.ingestion.configured_sources.find(
      (source) => source.name === 'personal-gmail'
    )!
    gmail.validation = undefined

    const source = operationalSources(status).find((item) => item.name === gmail.name)
    const health = sourceHealth(source!, Date.parse('2026-07-29T15:00:00Z'))

    expect(health.state).toBe('warning')
    expect(health.label).toContain('re-validate before recurring sync')
    expect(health.label).toContain('has not been fully validated')
  })

  test('warns when validation-only results undershoot configured source limits', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
      fresh: true,
      documents: 15,
      bytes: 4_096,
      max_documents: 100,
      max_bytes: 1_000,
      max_seconds: 30,
      complete: true,
      error: null,
    }

    const source = operationalSources(status).find((item) => item.name === buzz.name)
    const health = sourceHealth(source!)

    expect(health.state).toBe('warning')
    expect(health.label).toContain('does not cover the configured sync budget')
  })
})

describe('embedding status labels', () => {
  test('keeps URL colons from truncating the model label', () => {
    expect(embeddingLabel('openai:http://127.0.0.1:6999/v1:Qwen/Qwen3-Embedding-0.6B:1024')).toBe(
      'Qwen/Qwen3-Embedding-0.6B · 1024d'
    )
  })

  test('does not invent a model label for short or malformed fingerprints', () => {
    expect(embeddingLabel('deterministic:16')).toBe('deterministic:16')
    expect(embeddingLabel('openai:missing-dimension')).toBe('openai:missing-dimension')
    expect(embeddingLabel(null)).toBe('—')
  })
})

describe('provider endpoint classification', () => {
  test('recognizes only exact loopback hosts', () => {
    expect(isLoopbackUrl('http://127.0.0.1:6999/v1')).toBe(true)
    expect(isLoopbackUrl('http://[::1]:6999/v1')).toBe(true)
    expect(isLoopbackUrl('https://localhost/v1')).toBe(true)
    expect(isLoopbackUrl('https://api.localhost.example/v1')).toBe(false)
  })

  test('fails closed for malformed endpoints', () => {
    expect(isLoopbackUrl('not a URL')).toBe(false)
  })
})
