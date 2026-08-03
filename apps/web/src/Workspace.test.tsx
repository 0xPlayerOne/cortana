import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'

import { Workspace } from './components/Workspace'
import { safeSourceLink } from './sourceLinks'
import { canonicalDocument } from './test/fixtures'

afterEach(cleanup)

const props = {
  query: 'release',
  answer: null,
  evidence: [],
  selected: 0,
  loading: false,
  error: '',
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
