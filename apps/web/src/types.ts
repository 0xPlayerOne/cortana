export type Evidence = {
  chunk_id: string
  source: string
  source_id: string
  title: string
  uri: string | null
  content: string
  score: number
  semantic_rank: number | null
  lexical_rank: number | null
  updated_at: string
}

export type SourceSummary = {
  source: string
  project: string
  documents: number
  chunks: number
  latest_updated_at: string | null
}

export type SourceSyncSummary = {
  source: string
  project: string
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | 'budget_exceeded'
  started_at: string
  completed_at: string | null
  documents: number | null
  bytes: number | null
  deleted: number | null
  budget_documents: number
  budget_bytes: number
  budget_seconds: number
}

export type ConfiguredSourceSummary = {
  name: string
  source: string
  kind: string
  project: string
  enabled: boolean
  max_documents: number
  max_bytes: number
  max_duration_seconds: number
  validation?: {
    source: string
    project: string
    kind: string
    status: 'succeeded' | 'failed'
    validated_at: string
    documents: number | null
    bytes: number | null
    max_documents: number
    max_bytes: number
    max_seconds: number
    error: string | null
  } | null
}

export type IngestionStatus = {
  mode: 'manual' | 'scheduled'
  scheduled: boolean
  max_documents_per_source: number
  max_bytes_per_source: number
  max_duration_seconds: number
  request_concurrency: number
  validation_state_error?: string | null
  configured_sources: ConfiguredSourceSummary[]
}

export type BrainStatus = {
  status: string
  embedding_fingerprint: string | null
  embedding_cache_entries: number
  embedding_cache_hits: number
  query_cache_entries: number
  query_cache_hits: number
  answers_total: number
  query: {
    mode: 'extractive' | 'synthesized'
    model: string | null
    max_planned_queries: number
    retrieval_limit: number
    result_limit: number
    cache_ttl_seconds: number
    answer_timeout_seconds: number
  }
  documents: number
  chunks: number
  sources: SourceSummary[]
  sync_runs: SourceSyncSummary[]
  ingestion: IngestionStatus
}

export type AnswerResponse = {
  query: string
  answer: string
  evidence: Evidence[]
  plan: {
    queries: string[]
    model_generated: boolean
  }
  mode: 'extractive' | 'synthesized'
  cached: boolean
  latency_ms: number
  warnings: string[]
}

export type ContextBundle = {
  query: string
  context: string
  evidence: Evidence[]
  metrics: {
    retrieved: number
    included: number
    omitted: number
    estimated_tokens: number
    max_tokens: number
  }
}
