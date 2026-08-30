import { expect, test } from 'bun:test'

import { codeRevisionLabel } from './codeEvidence'
import type { Evidence } from './types'

const evidence: Evidence = {
  chunk_id: 'code:1',
  source: 'code',
  source_id: 'repo:src/lib.rs',
  title: 'src/lib.rs',
  uri: 'code://repo/src/lib.rs',
  content: 'pub fn main() {}',
  score: 1,
  semantic_rank: 1,
  lexical_rank: 1,
  updated_at: '2026-08-30T00:00:00Z',
  metadata: {
    code: {
      repository: 'cortana',
      branch: 'main',
      commit_sha: '1234567890abcdef',
      dirty: true,
    },
  },
}

test('code evidence exposes a bounded repository and revision label', () => {
  expect(codeRevisionLabel(evidence)).toBe('cortana · main · 12345678 · dirty')
  expect(codeRevisionLabel({ ...evidence, metadata: undefined })).toBeNull()
})
