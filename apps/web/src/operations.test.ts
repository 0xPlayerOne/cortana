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
})
