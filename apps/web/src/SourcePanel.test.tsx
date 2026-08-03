import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { Bot, Code2, Cloud, Database, MessageCircle } from 'lucide-react'

import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from './types'
import { demoStatus } from './demo'
import { SourcePanel } from './components/SourcePanel'
import { sourceIconForKind } from './components/sourceIconData'

afterEach(cleanup)

const workspace: WorkspaceSettings = {
  id: 'work',
  name: 'Work',
  account_label: null,
  color: '#5A9BD5',
}

const personalWorkspace: WorkspaceSettings = {
  id: 'personal',
  name: 'Personal',
  account_label: null,
  color: '#E8A83B',
}

function renderPanel(
  statusValue: BrainStatus | null,
  statusError: string,
  sourceJobError = '',
  onRetryStatus?: () => void,
  workspaceId = 'work'
) {
  const docs: BrainDocumentSummary[] = []
  const noJobs: DesktopSourceJob[] = []
  render(
    <SourcePanel
      open={false}
      status={statusValue}
      statusError={statusError}
      onRetryStatus={onRetryStatus}
      sourceJobError={sourceJobError}
      workspace={workspaceId}
      workspaces={workspaceId === 'work' ? [workspace] : [personalWorkspace]}
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

test('workspace picker shows the display scope without account metadata', () => {
  renderPanel(demoStatus, '')
  const picker = screen.getByRole('combobox', { name: 'Workspace' }) as HTMLSelectElement
  expect(picker.textContent).toContain('Work')
  expect(picker.textContent).not.toContain('All workspaces')
  expect(document.querySelector('.workspace-picker-mark')).toBeTruthy()
})

test('SourcePanel never falls back to an all-workspaces source tree', () => {
  render(
    <SourcePanel
      open={false}
      status={demoStatus}
      statusError=""
      workspace=""
      workspaces={[workspace, personalWorkspace]}
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
      jobs={[]}
    />
  )

  expect(screen.getByRole('button', { name: /^work-code/ })).toBeTruthy()
  expect(screen.queryByRole('button', { name: /^personal-gmail/ })).toBeNull()
})

test('SourcePanel surfaces status errors instead of empty-source phantom state', () => {
  renderPanel(null, 'Status unavailable')
  expect(screen.getByText('Status unavailable')).toBeTruthy()
  expect(screen.getByText('Ingestion status unavailable')).toBeTruthy()
  expect(screen.queryByText('No indexed sources yet.')).toBeNull()
})

test('SourcePanel exposes a bounded retry action for status errors', () => {
  let retries = 0
  renderPanel(null, 'Status unavailable', '', () => {
    retries += 1
  })
  fireEvent.click(screen.getByRole('button', { name: 'Retry status' }))
  expect(retries).toBe(1)
})

test('SourcePanel keeps the last known source index visible during a refresh failure', () => {
  renderPanel(demoStatus, 'Status refresh failed')
  expect(screen.getByText(/Status refresh failed Showing the last known source index/)).toBeTruthy()
  expect(screen.getByRole('button', { name: /^work-code/ })).toBeTruthy()
})

test('SourcePanel exposes a retry action for document list failures', () => {
  let retries = 0
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
      documentsError="Document list unavailable"
      hasMoreDocuments={false}
      onSelect={() => {}}
      onSelectWorkspace={() => {}}
      onDocumentQueryChange={() => {}}
      onSelectDocument={() => {}}
      onLoadMoreDocuments={() => {}}
      onRetryDocuments={() => {
        retries += 1
      }}
      onOpenSourcesSettings={() => {}}
      onClose={() => {}}
      jobs={[]}
    />
  )

  fireEvent.click(screen.getByRole('button', { name: 'Retry documents' }))
  expect(retries).toBe(1)
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
      workspace="work"
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
      onOpenSourcesSettings={openSourcesSettings}
      onClose={() => {}}
      jobs={[]}
    />
  )
  const add = screen.getByLabelText('Add source')
  const settings = screen.getByLabelText('Source settings')
  expect(add.getAttribute('title')).toBe('Add source')
  expect(settings.getAttribute('title')).toBe('Source settings')
  fireEvent.click(add)
  fireEvent.click(settings)
  expect(sourcesOpenCalls).toBe(2)
})

test('source icons use the exact configured connector kind', () => {
  expect(sourceIconForKind('filesystem')).toBe(Code2)
  expect(sourceIconForKind('google-drive')).toBe(Cloud)
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

test('source panel exposes setup and Google authorization actions only when required', () => {
  let setupSource = ''
  let setupProject = ''
  let authorizedSource = ''
  let authorizedProject = ''
  const actionStatus: BrainStatus = {
    ...demoStatus,
    ingestion: {
      ...demoStatus.ingestion,
      configured_sources: demoStatus.ingestion.configured_sources.map((source) =>
        source.source === 'team-slack'
          ? {
              ...source,
              authorization: { method: 'token' as const, setup_required: true, authorized: false },
            }
          : source.source === 'personal-drive'
            ? {
                ...source,
                authorization: {
                  method: 'google_oauth' as const,
                  setup_required: false,
                  authorized: false,
                },
              }
            : source
      ),
    },
  }
  render(
    <SourcePanel
      open={false}
      status={actionStatus}
      statusError=""
      workspace="work"
      workspaces={[workspace, personalWorkspace]}
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
      onOpenSourceSetup={(source, project) => {
        setupSource = source
        setupProject = project
      }}
      onAuthorizeSource={(source, project) => {
        authorizedSource = source
        authorizedProject = project
      }}
      onClose={() => {}}
      jobs={[]}
    />
  )
  fireEvent.click(screen.getByRole('button', { name: 'Open team-slack setup' }))
  expect(setupSource).toBe('team-slack')
  expect(setupProject).toBe('work')

  cleanup()

  render(
    <SourcePanel
      open={false}
      status={actionStatus}
      statusError=""
      workspace="personal"
      workspaces={[personalWorkspace, workspace]}
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
      onAuthorizeSource={(source, project) => {
        authorizedSource = source
        authorizedProject = project
      }}
      onClose={() => {}}
      jobs={[]}
    />
  )
  fireEvent.click(screen.getByRole('button', { name: 'Authorize personal-drive' }))
  expect(authorizedSource).toBe('personal-drive')
  expect(authorizedProject).toBe('personal')
  expect(screen.queryByRole('button', { name: 'Authorize team-slack' })).toBeNull()
})

test('Google setup action identifies the source editor instead of a provider URL', () => {
  let setupSource = ''
  const actionStatus: BrainStatus = {
    ...demoStatus,
    ingestion: {
      ...demoStatus.ingestion,
      configured_sources: demoStatus.ingestion.configured_sources.map((source) =>
        source.source === 'personal-drive'
          ? {
              ...source,
              authorization: {
                method: 'google_oauth' as const,
                setup_required: true,
                authorized: false,
              },
            }
          : source
      ),
    },
  }
  render(
    <SourcePanel
      open={false}
      status={actionStatus}
      statusError=""
      workspace="personal"
      workspaces={[personalWorkspace]}
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
      onOpenSourceSetup={(source) => {
        setupSource = source
      }}
      onClose={() => {}}
      jobs={[]}
    />
  )

  const setup = screen.getByRole('button', { name: 'Open personal-drive setup' })
  expect(setup.getAttribute('title')).toBe('Open Google source settings')
  fireEvent.click(setup)
  expect(setupSource).toBe('personal-drive')
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

test('active source jobs lock a source that uses a canonical label', () => {
  const labeledStatus: BrainStatus = {
    ...demoStatus,
    ingestion: {
      ...demoStatus.ingestion,
      configured_sources: demoStatus.ingestion.configured_sources.map((source) =>
        source.source === 'work-code' ? { ...source, source: 'code-label', enabled: true } : source
      ),
    },
    sources: demoStatus.sources.map((source) =>
      source.source === 'work-code' ? { ...source, source: 'code-label' } : source
    ),
  }
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
      status={labeledStatus}
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
      onToggleSource={() => {}}
      onClose={() => {}}
      jobs={[job]}
    />
  )
  const toggle = screen.getByRole('switch', { name: 'Disable work-code' }) as HTMLButtonElement
  expect(toggle.disabled).toBe(true)
})
