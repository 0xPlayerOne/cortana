import { getLargeDemoDocument, getLargeDemoDocumentPage } from './demoLarge'
import type { BrainDocument, BrainDocumentPage } from './types'

export function getLargeDemoDocuments(
  project?: string,
  source?: string,
  query?: string,
  cursor?: string
): BrainDocumentPage {
  return getLargeDemoDocumentPage(project, source, query, cursor)
}

export function getLargeDemoDocumentDetails(id: string): BrainDocument {
  const match = /^large-demo-(\d+)$/.exec(id)
  const index = match ? Number(match[1]) : Number.NaN
  const summary = Number.isSafeInteger(index) ? getLargeDemoDocument(index) : null
  if (!summary) throw new Error('Document not found')
  return {
    ...summary,
    content: `${summary.title} is a bounded large-corpus acceptance fixture.\n\nIt verifies that document navigation retrieves one canonical record while the surrounding document and graph views remain paginated and bounded.`,
    metadata: { fixture: 'large-demo', ordinal: index + 1 },
    acl: [],
    backlinks: [],
    surrounding: [],
    truncated: false,
  }
}
