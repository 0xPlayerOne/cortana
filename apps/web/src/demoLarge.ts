import type { BrainDocumentPage, BrainDocumentSummary } from './types'

export const LARGE_DEMO_DOCUMENT_COUNT = 2_500
export const LARGE_DEMO_PAGE_SIZE = 50

const LARGE_DEMO_SOURCES = Object.freeze([
  'work-drive',
  'work-code',
  'team-slack',
  'personal-notes',
])

export function getLargeDemoDocument(index: number, project = 'demo'): BrainDocumentSummary {
  if (!Number.isSafeInteger(index) || index < 0 || index >= LARGE_DEMO_DOCUMENT_COUNT) {
    throw new Error(`large demo document index is out of range: ${index}`)
  }
  const source = LARGE_DEMO_SOURCES[index % LARGE_DEMO_SOURCES.length]
  const ordinal = String(index + 1).padStart(4, '0')
  return {
    id: `large-demo-${index}`,
    source,
    source_id: `large-demo-${source}`,
    title: `Large corpus document ${ordinal}`,
    uri: `https://example.test/large/${index}`,
    updated_at: new Date(Date.UTC(2026, 0, 1 + (index % 365))).toISOString(),
    project,
    chunk_count: 2,
    content_chars: 320,
  }
}

function largeDemoCursorOffset(cursor: string | undefined): number {
  if (!cursor) return 0
  const match = /^large-demo:(\d+)$/.exec(cursor)
  const offset = match ? Number(match[1]) : Number.NaN
  if (!Number.isSafeInteger(offset) || offset < 0 || offset > LARGE_DEMO_DOCUMENT_COUNT) {
    throw new Error('large demo document cursor was malformed')
  }
  return offset
}

export function getLargeDemoDocumentPage(
  project?: string,
  source?: string,
  query?: string,
  cursor?: string
): BrainDocumentPage {
  const offset = largeDemoCursorOffset(cursor)
  const normalizedQuery = query?.trim().toLowerCase()
  const documents: BrainDocumentSummary[] = []
  let matched = 0
  let hasMore = false
  for (let index = 0; index < LARGE_DEMO_DOCUMENT_COUNT; index += 1) {
    const document = getLargeDemoDocument(index, project)
    if (
      (source && document.source !== source) ||
      (normalizedQuery &&
        ![document.title, document.source, document.source_id].some((value) =>
          value.toLowerCase().includes(normalizedQuery)
        ))
    ) {
      continue
    }
    if (matched < offset) {
      matched += 1
      continue
    }
    if (documents.length < LARGE_DEMO_PAGE_SIZE) {
      documents.push(document)
      matched += 1
      continue
    }
    hasMore = true
    break
  }
  return {
    documents,
    next_cursor: hasMore ? `large-demo:${offset + LARGE_DEMO_PAGE_SIZE}` : null,
  }
}
