import { expect, test } from 'bun:test'

import {
  LARGE_DEMO_DOCUMENT_COUNT,
  LARGE_DEMO_PAGE_SIZE,
  getLargeDemoDocument,
  getLargeDemoDocumentPage,
} from './demoLarge'

test('large demo pagination remains bounded and cursor-addressable', () => {
  expect(LARGE_DEMO_DOCUMENT_COUNT).toBe(2500)
  expect(LARGE_DEMO_PAGE_SIZE).toBe(50)

  const first = getLargeDemoDocumentPage()
  expect(first.documents).toHaveLength(LARGE_DEMO_PAGE_SIZE)
  expect(first.documents[0]).toMatchObject({ id: 'large-demo-0', project: 'demo' })
  expect(first.next_cursor).toBe('large-demo:50')

  const second = getLargeDemoDocumentPage(
    undefined,
    undefined,
    undefined,
    first.next_cursor ?? undefined
  )
  expect(second.documents).toHaveLength(LARGE_DEMO_PAGE_SIZE)
  expect(second.documents[0]?.id).toBe('large-demo-50')
  expect(second.next_cursor).toBe('large-demo:100')

  const last = getLargeDemoDocumentPage(undefined, undefined, undefined, 'large-demo:2450')
  expect(last.documents).toHaveLength(LARGE_DEMO_PAGE_SIZE)
  expect(last.documents.at(-1)?.id).toBe('large-demo-2499')
  expect(last.next_cursor).toBeNull()
})

test('large demo filtering does not materialize the complete matching page', () => {
  const page = getLargeDemoDocumentPage('work', 'work-code', 'document 00')
  expect(page.documents.length).toBeGreaterThan(0)
  expect(page.documents.length).toBeLessThanOrEqual(LARGE_DEMO_PAGE_SIZE)
  expect(page.documents.every((document) => document.source === 'work-code')).toBe(true)
  expect(
    page.documents.every((document) => document.title.toLowerCase().includes('document 00'))
  ).toBe(true)
  expect(getLargeDemoDocument(2497, 'work')).toMatchObject({
    id: 'large-demo-2497',
    project: 'work',
    source: 'work-code',
  })
})
