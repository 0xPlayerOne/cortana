import { describe, expect, test } from 'bun:test'

import { demoStatus } from './demo'
import { operationalSources, sourceHealth } from './operations'

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

  test('distinguishes a validated connector from an unproven source', () => {
    const status = structuredClone(demoStatus)
    const buzz = status.ingestion.configured_sources.find((source) => source.name === 'buzz')!
    buzz.validation = {
      source: 'buzz',
      project: 'agents',
      kind: 'buzz',
      status: 'succeeded',
      validated_at: '2026-07-30T06:00:00Z',
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
})
