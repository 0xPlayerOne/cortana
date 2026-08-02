import { afterEach, expect, test } from 'bun:test'
import { cleanup, render, screen } from '@testing-library/react'

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
