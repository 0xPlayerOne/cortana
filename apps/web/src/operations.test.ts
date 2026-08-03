import { describe, expect, test } from 'bun:test'

import { demoStatus } from './demo'
import {
  describeSyncRunProgress,
  embeddingLabel,
  isLoopbackUrl,
  operationalSources,
  sourceHealth,
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
      max_documents: 100,
      max_bytes: 1_048_576,
      max_seconds: 30,
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

  test('treats a validation without freshness metadata as current', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    // A server predating the freshness fields never reports `fresh`; the
    // workspace must not invent an expiry for a record it cannot judge.
    const legacy = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded' as const,
      validated_at: '2026-07-30T06:00:00Z',
      documents: 45,
      bytes: 4096,
      max_documents: 100,
      max_bytes: 1_048_576,
      max_seconds: 30,
      error: null,
    }
    buzz.validation = legacy

    const source = operationalSources(status).find((item) => item.name === 'buzz')
    expect(sourceHealth(source!).state).toBe('healthy')
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
