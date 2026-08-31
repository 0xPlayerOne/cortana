import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'

import { Workspace } from './components/Workspace'
import { safeSourceLink } from './sourceLinks'
import { canonicalDocument } from './test/fixtures'

afterEach(cleanup)

const props = {
  query: 'release',
  answer: null,
  reflection: null,
  evidence: [],
  selected: 0,
  loading: false,
  error: '',
  document: null,
  documentLoading: false,
  graph: null,
  graphLoading: false,
  graphError: '',
  tab: 'document' as const,
  onTabChange: () => {},
  onSelect: () => {},
  onSelectDocument: () => {},
  onRetry: () => {},
}

test('source links reject executable and credential-bearing URLs', () => {
  expect(safeSourceLink('javascript:alert(1)')).toBeNull()
  expect(safeSourceLink('https://user:secret@example.test/private')).toBeNull()
  expect(safeSourceLink('file://remote.example/private.txt')).toBeNull()
  expect(safeSourceLink('file:///tmp/note.md')).toBeNull()
  expect(safeSourceLink('file:///tmp/note.md', { allowLocalFile: true })).toBe(
    'file:///tmp/note.md'
  )
  expect(safeSourceLink('slack://channel?team=&id=C123ABC&message=1712345678.000100')).toBe(
    'slack://channel?team=&id=C123ABC&message=1712345678.000100'
  )
  expect(safeSourceLink('slack://channel?id=C123ABC')).toBeNull()
  expect(safeSourceLink('slack://channel?id=C123ABC&message=1&message=2')).toBeNull()
  expect(
    safeSourceLink('slack://channel?id=C123ABC&message=1&redirect=https://evil.test')
  ).toBeNull()
  expect(safeSourceLink('notes://showNote?identifier=x-coredata%3A%2F%2Fnote-1')).toBe(
    'notes://showNote?identifier=x-coredata%3A%2F%2Fnote-1'
  )
  expect(safeSourceLink('notes://showNote?identifier=')).toBeNull()
  expect(safeSourceLink('notes://showNote?identifier=note-1&extra=1')).toBeNull()
  expect(safeSourceLink('buzz://persona/npub123/session%3A1')).toBe(
    'buzz://persona/npub123/session%3A1'
  )
  expect(safeSourceLink('buzz://persona/npub123/session/extra')).toBeNull()
  expect(safeSourceLink('buzz://persona/npub123/')).toBeNull()
  expect(safeSourceLink('buzz://persona/npub%2F123/session')).toBeNull()
  expect(safeSourceLink('buzz://persona/npub123/session?')).toBeNull()
  expect(safeSourceLink('https://example.test/releases')).toBe('https://example.test/releases')
})

test('unsafe document links are not rendered into the web shell', () => {
  const unsafe = { ...canonicalDocument, uri: 'javascript:alert(1)' }
  render(<Workspace {...props} document={unsafe} />)

  expect(screen.getByRole('heading', { name: unsafe.title })).toBeTruthy()
  expect(screen.queryByRole('link', { name: 'Open original source' })).toBeNull()
  expect(screen.getByRole('button', { name: 'Copy content' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Copy citation' })).toBeTruthy()
})

test('shadcn renderer composes workspace navigation and empty states from shared primitives', () => {
  render(<Workspace {...props} />)

  expect(document.querySelector('[data-m7-knowledge-workspace]')).toBeTruthy()
  expect(document.querySelector('[data-slot="tabs"]')).toBeTruthy()
  expect(document.querySelector('[data-slot="tabs-list"]')).toBeTruthy()
  expect(document.querySelector('[data-slot="tabs-trigger"]')).toBeTruthy()
  expect(document.querySelector('[data-slot="empty"]')).toBeTruthy()
  expect(screen.getByText('Choose a document')).toBeTruthy()
})

test('shadcn workspace names revoked, loading, and malformed-content states without widening scope', () => {
  let retries = 0
  render(
    <Workspace
      {...props}
      error="Workspace access revoked"
      onRetry={() => {
        retries += 1
      }}
    />
  )
  expect(screen.getByText(/Workspace access revoked/)).toBeTruthy()
  const retry = screen.getByRole('button', { name: 'Try again' })
  expect(retry.getAttribute('data-slot')).toBe('button')
  fireEvent.click(retry)
  expect(retries).toBe(1)

  cleanup()
  render(<Workspace {...props} tab="answer" loading />)
  expect(screen.getByText('Searching your brain')).toBeTruthy()

  cleanup()
  const malformed = {
    ...canonicalDocument,
    content: '<script>window.location="https://example.test"</script>',
  }
  render(<Workspace {...props} document={malformed} />)
  expect(screen.getByText(malformed.content)).toBeTruthy()
  expect(document.querySelector('.canonical-content script')).toBeNull()
})

test('unsafe retrieved-evidence links are not rendered into the web shell', () => {
  render(
    <Workspace
      {...props}
      document={null}
      tab="sources"
      evidence={[
        {
          chunk_id: 'unsafe-evidence',
          source: 'notes',
          source_id: 'note-1',
          title: 'Unsafe evidence',
          uri: 'javascript:alert(1)',
          content: 'Evidence excerpt',
          score: 0.9,
          semantic_rank: 1,
          lexical_rank: null,
          updated_at: '2026-01-01T00:00:00Z',
        },
      ]}
    />
  )

  expect(screen.getByRole('heading', { name: 'Unsafe evidence' })).toBeTruthy()
  expect(screen.queryByRole('link', { name: 'Open original source' })).toBeNull()
})

test('supported app source links keep an explicit open-source affordance', () => {
  for (const uri of [
    'notes://showNote?identifier=x-coredata%3A%2F%2Fnote-1',
    'buzz://persona/npub123/session%3A1',
  ]) {
    cleanup()
    render(<Workspace {...props} document={{ ...canonicalDocument, uri }} />)
    const link = screen.getByRole('link', { name: 'Open original source' })
    expect(link.getAttribute('href')).toBe(uri)
    expect(link.getAttribute('title')).toBe('Open original source')
  }
})

test('graph failures expose a retry action', () => {
  let retries = 0
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      graphError="Graph data unavailable"
      onRetryGraph={() => {
        retries += 1
      }}
    />
  )

  expect(screen.getByRole('heading', { name: 'Graph unavailable' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Try again' }).className).toContain('bg-primary')
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
  expect(retries).toBe(1)
})

test('graph retries stay outside the live summary and document nodes describe their destination', () => {
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      graph={{
        nodes: [
          {
            id: 'document:one',
            kind: 'document',
            label: 'A very long document title that remains readable when the graph card is narrow',
            project: 'work',
            source: 'notes',
            document_id: 'one',
          },
        ],
        edges: [{ source: 'source:notes', target: 'document:one', kind: 'contains' }],
        next_cursor: null,
      }}
      graphError="Graph data unavailable"
      onRetryGraph={() => {}}
      onSelectDocument={() => {}}
    />
  )

  const summary = screen.getByRole('status')
  expect(summary.querySelector('button')).toBeNull()
  expect(screen.getByRole('button', { name: 'Retry graph' })).toBeTruthy()
  expect(screen.getByRole('button', { name: /Open document:/ })).toBeTruthy()
  expect(screen.getByRole('button', { name: /Open document:/ }).getAttribute('title')).toBe(
    'Open document'
  )
  expect(document.querySelector('.graph-links line')).toBeTruthy()
})

test('graph exposes bounded pagination when another page is available', () => {
  let loads = 0
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      graph={{
        nodes: Array.from({ length: 13 }, (_, index) => ({
          id: `document:${index}`,
          kind: 'document' as const,
          label: `Document ${index}`,
          project: 'work',
          source: 'notes',
          document_id: String(index),
        })),
        edges: [],
        next_cursor: 'next-page',
      }}
      onLoadMoreGraph={() => {
        loads += 1
      }}
      onSelectDocument={() => {}}
    />
  )

  expect(screen.getByText('Showing 12 of 13 documents · 0 links')).toBeTruthy()
  const loadMore = screen.getByRole('button', { name: 'Load more nodes' })
  expect(loadMore.className).toContain('bg-secondary')
  fireEvent.click(loadMore)
  expect(loads).toBe(1)
})

test('an empty graph page does not reuse unrelated retrieved evidence', () => {
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      evidence={[
        {
          chunk_id: 'stale-evidence',
          source: 'notes',
          source_id: 'note-1',
          title: 'Stale filtered result',
          uri: null,
          content: 'This result belongs to a previous query.',
          score: 0.8,
          semantic_rank: 1,
          lexical_rank: null,
          updated_at: '2026-01-01T00:00:00Z',
        },
      ]}
      graph={{ nodes: [], edges: [], next_cursor: null }}
    />
  )

  expect(screen.getByRole('heading', { name: 'No graph data' })).toBeTruthy()
  expect(screen.queryByRole('button', { name: /Stale filtered result/ })).toBeNull()
})

test('graph supports bounded filtering and explains selected relationships', () => {
  const focused: string[] = []
  const relationshipFilters: string[] = []
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      graph={{
        nodes: [
          {
            id: 'document:one',
            kind: 'document',
            label: 'Release notes',
            project: 'work',
            source: 'code',
            document_id: 'one',
          },
          {
            id: 'document:two',
            kind: 'document',
            label: 'Personal journal',
            project: 'personal',
            source: 'notes',
            document_id: 'two',
          },
        ],
        edges: [
          {
            source: 'source:code',
            target: 'document:one',
            kind: 'contains',
            origin: 'explicit',
            citation_authority: true,
            support: {
              record_ids: ['one'],
              invalidation_keys: ['document:one@sha256:abc'],
            },
          },
        ],
        next_cursor: null,
      }}
      onSelectDocument={() => {}}
      onFocusGraphNode={(node) => focused.push(node.id)}
      onGraphEdgeKindChange={(kind) => relationshipFilters.push(kind)}
      onGraphOriginChange={(origin) => relationshipFilters.push(origin)}
      onGraphMinConfidenceChange={(confidence) => relationshipFilters.push(String(confidence))}
    />
  )

  const filter = screen.getByRole('searchbox', { name: 'Filter graph nodes' })
  fireEvent.change(filter, { target: { value: 'release' } })
  expect(screen.getByRole('button', { name: /Open document: Release notes/ })).toBeTruthy()
  expect(screen.queryByRole('button', { name: /Open document: Personal journal/ })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: /Open document: Release notes/ }))
  expect(screen.getByRole('complementary', { name: 'Selected graph node' })).toBeTruthy()
  expect(screen.getByText('Contained by its workspace or source')).toBeTruthy()
  expect(
    screen.getByText('Explicit relationship · 1 supporting record · citation-capable')
  ).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Open document' })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Pin node' }))
  expect(screen.getByRole('button', { name: 'Unpin node' })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Expand one-hop relationships' }))
  expect(focused).toEqual(['document:one'])
  fireEvent.change(screen.getByRole('combobox', { name: 'Filter graph relationships' }), {
    target: { value: 'contains' },
  })
  fireEvent.change(screen.getByRole('combobox', { name: 'Filter graph relationship origin' }), {
    target: { value: 'explicit' },
  })
  fireEvent.change(screen.getByRole('combobox', { name: 'Filter graph minimum confidence' }), {
    target: { value: '0.75' },
  })
  expect(relationshipFilters).toEqual(['contains', 'explicit', '0.75'])
}, 15_000)

test('graph exposes workspace and source nodes with bounded type filters', () => {
  const focused: string[] = []
  render(
    <Workspace
      {...props}
      document={null}
      tab="graph"
      graph={{
        nodes: [
          {
            id: 'workspace:work',
            kind: 'workspace',
            label: 'work',
            project: 'work',
            source: null,
            document_id: null,
          },
          {
            id: 'source:work:notes',
            kind: 'source',
            label: 'notes',
            project: 'work',
            source: 'notes',
            document_id: null,
          },
          {
            id: 'document:one',
            kind: 'document',
            label: 'Release notes',
            project: 'work',
            source: 'notes',
            document_id: 'one',
          },
        ],
        edges: [
          { source: 'workspace:work', target: 'source:work:notes', kind: 'contains' },
          { source: 'source:work:notes', target: 'document:one', kind: 'contains' },
        ],
        next_cursor: null,
      }}
      onFocusGraphNode={(node) => focused.push(`${node.kind}:${node.project}:${node.source ?? ''}`)}
    />
  )

  expect(screen.getByRole('button', { name: 'Focus workspace: work' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Focus source: notes' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Open document: Release notes' })).toBeTruthy()

  fireEvent.click(screen.getByRole('button', { name: 'Focus workspace: work' }))
  fireEvent.click(screen.getByRole('button', { name: 'Focus source: notes' }))
  expect(focused).toEqual(['workspace:work:', 'source:work:notes'])

  fireEvent.click(screen.getByRole('button', { name: 'Sources' }))
  expect(screen.getByRole('button', { name: 'Focus source: notes' })).toBeTruthy()
  expect(screen.queryByRole('button', { name: 'Focus workspace: work' })).toBeNull()
  expect(screen.queryByRole('button', { name: 'Open document: Release notes' })).toBeNull()
  expect(screen.getByText('Showing 1 of 1 node · 2 links')).toBeTruthy()
})

const evidenceItem = {
  chunk_id: 'release-notes',
  source: 'work-code',
  source_id: 'release-1',
  title: 'Release notes',
  uri: null,
  content: 'Evidence excerpt',
  score: 0.9,
  semantic_rank: 1,
  lexical_rank: null,
  updated_at: '2026-01-01T00:00:00Z',
}

test('document is the default primary view and result tabs stay hidden until a search returns evidence', () => {
  render(<Workspace {...props} />)

  expect(screen.getByRole('tab', { name: 'Document' }).getAttribute('aria-selected')).toBe('true')
  expect(screen.getByRole('heading', { name: 'Choose a document' })).toBeTruthy()
  for (const name of ['Answer', 'Evidence', 'Timeline']) {
    expect(screen.queryByRole('tab', { name })).toBeNull()
  }
  // Document remains first-class without a search result; Graph is a rail-only view.
  expect(screen.getByRole('tab', { name: 'Document' }).hasAttribute('disabled')).toBe(false)
})

test('result tabs cannot be activated before a search because they are hidden', () => {
  let changed = ''
  render(
    <Workspace
      {...props}
      onTabChange={(next) => {
        changed = next
      }}
    />
  )

  expect(screen.queryByRole('tab', { name: 'Answer' })).toBeNull()
  expect(screen.queryByRole('tab', { name: 'Evidence' })).toBeNull()
  expect(screen.queryByRole('tab', { name: 'Timeline' })).toBeNull()
  expect(changed).toBe('')
})

test('answer, evidence, and timeline tabs enable once evidence arrives and keep direct result navigation', () => {
  let changed = ''
  render(
    <Workspace
      {...props}
      tab="answer"
      answer={null}
      evidence={[evidenceItem]}
      onTabChange={(next) => {
        changed = next
      }}
    />
  )

  for (const name of ['Answer', 'Evidence', 'Timeline']) {
    expect(screen.getByRole('tab', { name: new RegExp(name) }).hasAttribute('disabled')).toBe(false)
  }
  expect(screen.getByRole('tab', { name: /Evidence/ }).textContent).toContain('1')

  // Clicking a cited passage still routes straight to the Evidence tab.
  fireEvent.click(screen.getByRole('button', { name: /Release notes/ }))
  expect(changed).toBe('sources')
})

test('workspace renders the shipped app icon asset with decorative alt/aria behavior', () => {
  render(
    <Workspace
      {...props}
      tab="answer"
      answer={{
        query: 'release',
        answer: 'Evidence brief',
        evidence: [evidenceItem],
        plan: { queries: ['release'], model_generated: false },
        mode: 'extractive',
        cached: false,
        latency_ms: 0,
        warnings: [],
      }}
      evidence={[evidenceItem]}
    />
  )

  // The Answer tab uses the shipped asset as its product mark.
  const icons = Array.from(document.querySelectorAll<HTMLImageElement>('img[src="/app-icon.svg"]'))
  expect(icons.length).toBeGreaterThanOrEqual(1)
  for (const icon of icons) {
    expect(icon.getAttribute('alt')).toBe('')
    expect(icon.getAttribute('aria-hidden')).toBe('true')
  }
  expect(
    screen.getByRole('tab', { name: 'Answer' }).querySelector('img[src="/app-icon.svg"]')
  ).toBeTruthy()
  // No inline CortanaBrandMark markup remains anywhere in the workspace.
  expect(document.querySelector('svg[viewBox="0 0 1024 1024"]')).toBeNull()
})
