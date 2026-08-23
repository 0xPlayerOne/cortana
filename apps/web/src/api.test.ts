import { afterEach, describe, expect, mock, test } from 'bun:test'

import { getStatus } from './api'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('status transport', () => {
  test('preserves the bounded warm-up message for a retryable status response', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(
        new Response('Cortana is warming up; live status will be available shortly', {
          status: 503,
        })
      )
    ) as unknown as typeof fetch

    await expect(getStatus()).rejects.toThrow(
      'Cortana is warming up; live status will be available shortly'
    )
  })

  test('keeps unknown status failures generic', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(new Response('private database details', { status: 503 }))
    ) as unknown as typeof fetch

    await expect(getStatus()).rejects.toThrow('Status request failed (503)')
  })
})
