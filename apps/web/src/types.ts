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

export type BrainStatus = {
  status: string
  embedding_fingerprint: string | null
  embedding_cache_entries: number
  embedding_cache_hits: number
  documents: number
  chunks: number
  sources: SourceSummary[]
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
