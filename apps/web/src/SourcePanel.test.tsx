import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { Bot, Code2, Database, Folder, MessageCircle } from 'lucide-react'

import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from './types'
import { demoStatus } from './demo'
import { SourcePanel, sourceIconForKind } from './components/SourcePanel'

afterEach(cleanup)

const workspace: WorkspaceSettings = {
  id: 'work',
  name: 'Work',
  account_label: null,
  color: '#5A9BD5',
}

function renderPanel(statusValue: BrainStatus | null, statusError: string, sourceJobError = '') {
  const docs: BrainDocumentSummary[] = []
  const noJobs: DesktopSourceJob[] = []
  render(
    <SourcePanel
      open={false}
      status={statusValue}
      statusError={statusError}
      sourceJobError={sourceJobError}
      workspace=""
      workspaces={[workspace]}
      documentQuery=""
      selected=""
      documents={docs}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      jobs={noJobs}
    />
  )
}

test('SourcePanel reports loading while status is still resolving', () => {
  renderPanel(null, '')
  expect(screen.getByText('Loading source index and health…')).toBeTruthy()
})

test('SourcePanel surfaces status errors instead of empty-source phantom state', () => {
  renderPanel(null, 'Status unavailable')
  expect(screen.getByText('Status unavailable')).toBeTruthy()
  expect(screen.getByText('Ingestion status unavailable')).toBeTruthy()
  expect(screen.queryByText('No indexed sources yet.')).toBeNull()
})

test('SourcePanel keeps the last known source index visible during a refresh failure', () => {
  renderPanel(demoStatus, 'Status refresh failed')
  expect(screen.getByText(/Status refresh failed Showing the last known source index/)).toBeTruthy()
  expect(screen.getByRole('button', { name: /^work-code/ })).toBeTruthy()
})

test('SourcePanel keeps cancellation failures separate from runtime health', () => {
  renderPanel(demoStatus, '', 'Source job cancellation failed')
  expect(screen.getByRole('alert').textContent).toBe('Source job cancellation failed')
  expect(screen.queryByText('Status unavailable')).toBeNull()
})

test('document filter exposes a clear action only when text is present', () => {
  let nextQuery = 'unchanged'
  render(
    <SourcePanel
      open={false}
      status={demoStatus}
      statusError=""
      workspace=""
      workspaces={[workspace]}
      documentQuery="legacy"
      selected=""
      documents={[]}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={(query) => {
        nextQuery = query
      }}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      jobs={[]}
    />
  )

  expect(screen.getByRole('button', { name: 'Clear document filter' })).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Clear document filter' }))
  expect(nextQuery).toBe('')

  cleanup()
  renderPanel(demoStatus, '')
  expect(screen.queryByRole('button', { name: 'Clear document filter' })).toBeNull()
})

test('SourcePanel source and settings shortcuts open the Sources settings section', () => {
  let sourcesOpenCalls = 0
  const openSourcesSettings = () => {
    sourcesOpenCalls += 1
  }
  render(
    <SourcePanel
      open={false}
      status={null}
      statusError=""
      workspace=""
      workspaces={[workspace]}
      documentQuery=""
      selected=""
      documents={[]}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={openSourcesSettings}
      onClose={() => {}}
      jobs={[]}
    />
  )
  fireEvent.click(screen.getByLabelText('Add source'))
  fireEvent.click(screen.getByLabelText('Source settings'))
  expect(sourcesOpenCalls).toBe(2)
})

test('source icons use the exact configured connector kind', () => {
  expect(sourceIconForKind('filesystem')).toBe(Code2)
  expect(sourceIconForKind('google-drive')).toBe(Folder)
  expect(sourceIconForKind('slack')).toBe(MessageCircle)
  expect(sourceIconForKind('buzz')).toBe(Bot)
  expect(sourceIconForKind('slack-archive')).toBe(Database)
})

test('source selection is scoped to the active workspace when names repeat', () => {
  const duplicateStatus: BrainStatus = {
    ...demoStatus,
    ingestion: {
      ...demoStatus.ingestion,
      configured_sources: [
        ...demoStatus.ingestion.configured_sources,
        {
          ...demoStatus.ingestion.configured_sources[0],
          project: 'personal',
          acl: ['personal'],
        },
      ],
    },
  }
  render(
    <SourcePanel
      open={false}
      status={duplicateStatus}
      statusError=""
      workspace="work"
      workspaces={[workspace]}
      documentQuery=""
      selected="work-code"
      documents={[]}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      jobs={[]}
    />
  )
  const rows = screen.getAllByRole('button', { name: /work-code/ })
  expect(rows).toHaveLength(2)
  expect(rows.filter((row) => row.getAttribute('aria-pressed') === 'true')).toHaveLength(1)
})

test('source-select button is rendered as a button control', () => {
  render(
    <SourcePanel
      open={false}
      status={demoStatus}
      statusError=""
      workspace="work"
      workspaces={[workspace]}
      documentQuery=""
      selected="work-code"
      documents={[]}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      jobs={[]}
    />
  )
  const selectButton = screen.getByRole('button', { name: /work-code \d+/ })
  expect(selectButton).toBeTruthy()
  expect(selectButton.className).toContain('source-select')
  expect(selectButton.getAttribute('type')).toBe('button')
  expect(selectButton.getAttribute('aria-pressed')).toBe('true')
})

test('active source jobs expose a cancellation control in the source panel', () => {
  let cancelled = ''
  const job: DesktopSourceJob = {
    id: 'source-1-1',
    operation: 'validation',
    source: 'work-code',
    kind: 'filesystem',
    project: 'work',
    acl: ['work'],
    status: 'running',
    summary: 'validating',
    log: '',
    started_at_unix_seconds: 1785000000,
    completed_at_unix_seconds: null,
    exit_code: null,
    retryable: false,
    writes_indexed_data: false,
    budget: null,
  }
  render(
    <SourcePanel
      open={false}
      status={demoStatus}
      statusError=""
      workspace="work"
      workspaces={[workspace]}
      documentQuery=""
      selected=""
      documents={[]}
      selectedDocument=""
      documentsLoading={false}
      documentsError=""
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      onCancelSourceJob={(id) => {
        cancelled = id
      }}
      jobs={[job]}
    />
  )
  fireEvent.click(screen.getByRole('button', { name: 'Cancel work work-code validation' }))
  expect(cancelled).toBe('source-1-1')
})
