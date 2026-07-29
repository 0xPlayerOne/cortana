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
  documents: number
  chunks: number
  sources: SourceSummary[]
}
