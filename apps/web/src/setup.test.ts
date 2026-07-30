import { describe, expect, test } from 'bun:test'

import { buildSetupSteps } from './setup'

describe('guided setup progress', () => {
  test('requires a verified runtime and at least one source', () => {
    const steps = buildSetupSteps(
      {
        workspaces: [{ id: 'work', name: 'Work', account_label: null, color: null }],
        embedding: {
          provider: 'local',
          base_url: 'http://127.0.0.1:6999/v1',
          model: 'Qwen/Qwen3-Embedding-0.6B',
          api_key_env: null,
          dimension: 1024,
          cache_max_entries: 100,
          request_timeout_seconds: 30,
          request_concurrency: 1,
          startup_timeout_seconds: 60,
          memory_limit_mb: 4096,
        },
        sources: [],
      },
      {
        scanned_at_unix_seconds: 1,
        platform: 'macos',
        tools_ready: true,
        core: { passed: true, query_mode: 'extractive', checks: [] },
        core_error: null,
        tools: [],
      }
    )

    expect(steps.filter((step) => step.complete).map((step) => step.section)).toEqual([
      'readiness',
      'workspaces',
      'embedding',
    ])
    expect(steps.find((step) => step.section === 'sources')?.detail).toBe('Add at least one source')
  })
})
