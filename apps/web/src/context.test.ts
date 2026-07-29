import { describe, expect, test } from 'bun:test'

import { demoEvidence } from './demo'
import { buildAgentContext, estimateTokens } from './context'

describe('agent context', () => {
  test('packs provenance and numbered evidence', () => {
    const context = buildAgentContext('release', demoEvidence.slice(0, 2))
    expect(context).toContain('Query: release')
    expect(context).toContain('### [1] How do releases work?')
    expect(context).toContain('Source: work-drive')
  })

  test('estimates a non-zero token count', () => {
    expect(estimateTokens('12345678')).toBe(2)
    expect(estimateTokens('')).toBe(1)
  })
})
