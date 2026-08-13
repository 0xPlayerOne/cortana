import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { Bot, Code2, Cloud, Database, Github, MessageCircle } from 'lucide-react'

import type {
  BrainDocumentSummary,
  BrainStatus,
  DesktopSourceJob,
  WorkspaceSettings,
} from './types'
import { demoStatus } from './demo'
import { SourcePanel } from './components/SourcePanel'
import { SourceIcon } from './components/sourceIcons'
import { sourceBrandForKind, sourceIconForKind } from './components/sourceIconData'

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
  expect(add.getAttribute('title')).toBeNull()
  expect(add.getAttribute('data-tooltip')).toBe('Add source')
  expect(settings.getAttribute('title')).toBeNull()
  expect(settings.getAttribute('data-tooltip')).toBe('Source settings')
  fireEvent.click(add)
  fireEvent.click(settings)
  expect(sourcesOpenCalls).toBe(2)
})

test('source icons use the exact configured connector kind', () => {
  expect(sourceIconForKind('filesystem')).toBe(Code2)
  expect(sourceIconForKind('google-drive')).toBe(Cloud)
  expect(sourceIconForKind('github')).toBe(Github)
  expect(sourceIconForKind('slack')).toBe(MessageCircle)
  expect(sourceIconForKind('buzz')).toBe(Bot)
  expect(sourceIconForKind('slack-archive')).toBe(Database)
})

test('source icons keep brand fidelity for Notes and Drive and fall back to lucide glyphs', () => {
  // Apple Notes must not render as the generic code/folder glyph, and Drive
  // must render its brand mark rather than the plain cloud fallback.
  expect(sourceBrandForKind('apple-notes')).toBeDefined()
  expect(sourceBrandForKind('google-drive')).toBeDefined()
  expect(sourceBrandForKind('gmail')).toBeDefined()
  expect(sourceBrandForKind('google-calendar')).toBeDefined()

  const { container: notesContainer } = render(<SourceIcon kind="apple-notes" />)
  const notesPath = notesContainer.querySelector('svg path')
  expect(notesPath).toBeTruthy()
  expect(notesPath?.getAttribute('d')).toBe(sourceBrandForKind('apple-notes')?.path)

  const { container: driveContainer } = render(<SourceIcon kind="google-drive" />)
  const drivePath = driveContainer.querySelector('svg path')
  expect(drivePath).toBeTruthy()
  expect(drivePath?.getAttribute('d')).toBe(sourceBrandForKind('google-drive')?.path)

  // Connectors without a brand glyph render their lucide fallback icon.
  const { container: filesContainer } = render(<SourceIcon kind="filesystem" />)
  const filesSvg = filesContainer.querySelector('svg')
  expect(filesSvg).toBeTruthy()
  // Lucide glyphs are stroke-based (fill="none"); brand marks are filled.
  expect(filesSvg?.getAttribute('fill')).toBe('none')
  expect(filesSvg?.querySelector('path')).toBeTruthy()
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

test('source panel exposes setup and browser authorization actions only when required', () => {
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
  actionStatus.ingestion.configured_sources.push({
    name: 'work-github',
    source: 'work-github',
    kind: 'github',
    project: 'work',
    enabled: true,
    acl: ['work'],
    max_documents: 100,
    max_bytes: 1_048_576,
    max_duration_seconds: 300,
    authorization: { method: 'github_oauth', setup_required: false, authorized: false },
  })
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
  fireEvent.click(screen.getByRole('button', { name: 'Authorize work-github' }))
  expect(authorizedSource).toBe('work-github')
  expect(authorizedProject).toBe('work')

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
  expect(setup.getAttribute('title')).toBeNull()
  expect(setup.getAttribute('data-tooltip')).toBe('Open Google source settings')
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

const explorerDocs: BrainDocumentSummary[] = [
  {
    id: 'doc-1',
    source: 'work-code',
    source_id: 'src/main.rs',
    title: 'Main entrypoint',
    uri: null,
    updated_at: '2026-07-28T14:42:00Z',
    project: 'work',
    chunk_count: 3,
    content_chars: 1200,
  },
  {
    id: 'doc-2',
    source: 'work-drive',
    source_id: 'release-process',
    title: 'Release checklist',
    uri: 'https://example.test/releases',
    updated_at: '2026-07-24T10:12:00Z',
    project: 'work',
    chunk_count: 2,
    content_chars: 800,
  },
]

function renderExplorer(selected: string) {
  return render(
    <SourcePanel
      open={false}
      status={demoStatus}
      statusError=""
      workspace="work"
      workspaces={[workspace]}
      documentQuery=""
      selected={selected}
      documents={explorerDocs}
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
      jobs={[]}
    />
  )
}

test('document explorer heading follows the workspace -> source hierarchy', () => {
  renderExplorer('work-code')
  // Selected source: the breadcrumb names the workspace and then the
  // human-facing source label, never a legacy workflow/folder label.
  const heading = screen.getByLabelText('Documents in Work / Files & code')
  expect(heading.textContent).toContain('Work')
  expect(heading.textContent).toContain('Files & code')
})

test('document explorer heading stays workspace-scoped when no source is selected', () => {
  renderExplorer('')
  // Unselected: the explorer is scoped to the active workspace's sources,
  // not to an all-workspaces view.
  expect(screen.getByLabelText('Documents in Work / All sources')).toBeTruthy()
  expect(screen.queryByLabelText(/Documents in Personal/)).toBeNull()
})

test('document rows are indented nodes with no legacy workflow/folder labels', () => {
  const { container } = renderExplorer('')
  const rows = container.querySelectorAll('.virtual-document-space button.document-node')
  expect(rows).toHaveLength(explorerDocs.length)
  // Each row keeps its source disambiguation and the indented hierarchy
  // class that the stylesheet nests under the workspace/source breadcrumb.
  expect(screen.getByRole('option', { name: /Main entrypoint/ })).toBeTruthy()
  expect(screen.getByRole('option', { name: /Release checklist/ })).toBeTruthy()
  // Legacy workflow/folder terminology must not appear anywhere in the
  // Knowledge sources panel or its document explorer.
  expect(screen.queryByText(/workflow/i)).toBeNull()
  expect(screen.queryByText(/folder/i)).toBeNull()
})

test('source panel is the only Knowledge surface with enable switches', () => {
  const { container } = renderExplorer('')
  const switches = container.querySelectorAll('[role="switch"]')
  expect(switches.length).toBeGreaterThan(0)
  for (const control of Array.from(switches)) {
    expect(control.closest('.source-panel')).toBeTruthy()
  }
  // The document explorer itself never offers an enable/disable control.
  expect(container.querySelector('.document-explorer [role="switch"]')).toBeNull()
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
