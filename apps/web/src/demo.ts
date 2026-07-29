import type { BrainStatus, Evidence } from './types'

export const demoEvidence: Evidence[] = [
  {
    chunk_id: 'release:0',
    source: 'work-drive',
    source_id: 'release-process',
    title: 'How do releases work?',
    uri: 'https://example.test/releases',
    content:
      'Our releases follow trunk-based development with short-lived feature branches and automated delivery to staging.\n\nPlan against the roadmap, build behind feature flags, validate in staging, then cut and monitor the release. Roll back to the previous stable tag if health checks regress.',
    score: 0.98,
    semantic_rank: 1,
    lexical_rank: 1,
    updated_at: '2026-07-28T14:42:00Z',
  },
  {
    chunk_id: 'playbook:0',
    source: 'work-code',
    source_id: 'deployment-playbook',
    title: 'Deployment playbook',
    uri: 'https://example.test/playbook',
    content:
      'Promote staging only after unit, integration, end-to-end, and security checks pass. Observe the deployment before closing the release.',
    score: 0.91,
    semantic_rank: 2,
    lexical_rank: 3,
    updated_at: '2026-07-24T10:12:00Z',
  },
  {
    chunk_id: 'slack:0',
    source: 'team-slack',
    source_id: 'C1:release',
    title: 'Slack: #releases — cadence',
    uri: null,
    content:
      'Ada: Minor releases ship weekly. Major releases remain monthly unless the roadmap marks an exception.\nSam: Keep the rollback owner in the release checklist.',
    score: 0.84,
    semantic_rank: 4,
    lexical_rank: 2,
    updated_at: '2026-07-21T18:05:00Z',
  },
  {
    chunk_id: 'incident:0',
    source: 'personal-notes',
    source_id: 'incident-response',
    title: 'Incident response playbook',
    uri: null,
    content:
      'When a release causes an incident, halt promotion, assign an incident lead, roll back, and preserve the evidence needed for the postmortem.',
    score: 0.78,
    semantic_rank: 3,
    lexical_rank: 6,
    updated_at: '2026-07-13T09:30:00Z',
  },
]

export const demoStatus: BrainStatus = {
  status: 'ok',
  embedding_fingerprint: 'openai-compatible:Qwen/Qwen3-Embedding-0.6B:1024',
  embedding_cache_entries: 42891,
  embedding_cache_hits: 10642,
  documents: 9834,
  chunks: 128412,
  sources: [
    {
      source: 'work-code',
      project: 'work',
      documents: 1105,
      chunks: 33150,
      latest_updated_at: '2026-07-29T14:42:00Z',
    },
    {
      source: 'personal-gmail',
      project: 'personal',
      documents: 2763,
      chunks: 19234,
      latest_updated_at: '2026-07-29T14:38:00Z',
    },
    {
      source: 'personal-drive',
      project: 'personal',
      documents: 1982,
      chunks: 30210,
      latest_updated_at: '2026-07-29T14:35:00Z',
    },
    {
      source: 'personal-notes',
      project: 'personal',
      documents: 312,
      chunks: 4130,
      latest_updated_at: '2026-07-29T14:31:00Z',
    },
    {
      source: 'community-discord',
      project: 'community',
      documents: 431,
      chunks: 12092,
      latest_updated_at: '2026-07-29T13:51:00Z',
    },
    {
      source: 'team-slack',
      project: 'work',
      documents: 623,
      chunks: 8850,
      latest_updated_at: '2026-07-29T14:40:00Z',
    },
    {
      source: 'buzz',
      project: 'agents',
      documents: 128,
      chunks: 3102,
      latest_updated_at: '2026-07-29T14:39:00Z',
    },
  ],
}
