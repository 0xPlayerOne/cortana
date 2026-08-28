import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'

import { desktopInfo, desktopSettings } from './test/fixtures'
import type {
  BuzzCommunityList,
  DesktopSettings,
  DesktopSettingsUpdate,
  SourceSettings,
} from './types'

afterEach(cleanup)

const realApi = await import('./api')

const communities: BuzzCommunityList = {
  truncated: false,
  communities: [
    { id: 'builtin-team:welcome', name: 'Welcome Team' },
    { id: 'team:research', name: 'Research' },
  ],
}

const buzzSource: SourceSettings = {
  name: 'agent-buzz',
  kind: 'buzz',
  enabled: true,
  project: 'work',
  root: '/Users/you/Library/Application Support/xyz.block.buzz.app',
  source: null,
  channels: [],
  repositories: [],
  servers: [],
  teams: [],
  team_names: [],
  communities: [],
  community_names: [],
  token_env: null,
  token_path: null,
  oauth_client_path: null,
  query: null,
  labels: [],
  max_content_chars: null,
  max_documents: null,
  max_bytes: null,
  max_duration_seconds: null,
  exclude: [],
  acl: [],
  editable: true,
}

function settingsWith(source: SourceSettings): DesktopSettings {
  return { ...desktopSettings, sources: [source] }
}

const state = {
  settings: settingsWith(buzzSource),
  discoverCalls: [] as string[],
  communitiesResult: communities as BuzzCommunityList | null,
  communitiesError: null as Error | null,
  savedUpdates: [] as DesktopSettingsUpdate[],
  saved: null as DesktopSettings | null,
}

beforeEach(() => {
  state.settings = settingsWith(buzzSource)
  state.discoverCalls = []
  state.communitiesResult = communities
  state.communitiesError = null
  state.savedUpdates = []
  state.saved = null
})

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getDesktopSettings: () => Promise.resolve(state.settings),
  getDesktopInfo: () => Promise.resolve(desktopInfo),
  getDesktopSchedule: () =>
    Promise.resolve({ sync_interval_seconds: 900, backup_interval_seconds: 86400 }),
  saveDesktopSchedule: (schedule: {
    sync_interval_seconds: number
    backup_interval_seconds: number
  }) => Promise.resolve(schedule),
  getDesktopUpdate: () => Promise.reject(new Error('Updates unavailable')),
  getDesktopServices: () =>
    Promise.resolve({
      platform: 'macos',
      supported: true,
      services: [],
    }),
  getRuntimeAudit: () => Promise.resolve([]),
  getDesktopAudit: () => Promise.resolve([]),
  planDesktopInitialSync: () => Promise.reject(new Error('initial sync unavailable')),
  startDesktopInitialSync: () => Promise.reject(new Error('initial sync unavailable')),
  startDesktopSourceValidation: () => Promise.reject(new Error('validation unavailable')),
  getDesktopSourceValidation: () => Promise.reject(new Error('job missing')),
  cancelDesktopSourceValidation: () => Promise.reject(new Error('job missing')),
  listDesktopBuzzCommunities: (source: string) => {
    state.discoverCalls.push(source)
    if (state.communitiesError) return Promise.reject(state.communitiesError)
    return Promise.resolve(state.communitiesResult)
  },
  saveDesktopSettings: (update: DesktopSettingsUpdate) => {
    state.savedUpdates.push(update)
    const saved: DesktopSettings = {
      ...state.settings,
      workspaces: update.workspaces,
      sources: update.sources,
      auth_principals: update.auth_principals,
      embedding: update.embedding,
      query: update.query,
      memory: update.memory,
      ingestion: update.ingestion,
      runtime: update.runtime,
    }
    state.settings = saved
    state.saved = saved
    return Promise.resolve(saved)
  },
}))

const { SettingsView } = await import('./components/SettingsView')

async function renderBuzzSettings() {
  const view = render(
    <SettingsView
      initialSection="sources"
      desktopSettings={state.settings}
      onSaved={(next) => {
        state.saved = next
      }}
    />
  )
  await screen.findByLabelText(/^Source name/)
  return {
    view,
    rerender: () =>
      view.rerender(
        <SettingsView
          initialSection="sources"
          desktopSettings={state.settings}
          onSaved={(next) => {
            state.saved = next
          }}
        />
      ),
  }
}

test('buzz community chooser discovers the identity file and persists per-workspace assignment', async () => {
  const { rerender } = await renderBuzzSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover communities/ }))
  await waitFor(() => expect(screen.getByText('Welcome Team')).toBeTruthy())
  expect(state.discoverCalls).toEqual(['agent-buzz'])
  expect(screen.getByText('Research')).toBeTruthy()
  const chooser = screen.getByRole('group', { name: 'Community chooser' })
  expect(within(chooser).getAllByRole('checkbox')).toHaveLength(2)
  expect(chooser.getAttribute('aria-describedby')).toBeTruthy()

  // Community selection lands in the `communities` field with display names
  // kept index-aligned in `community_names`, persisted per source (each Buzz
  // source belongs to exactly one workspace, so the chooser is scoped to the
  // selected workspace). Unlike Slack's single-team contract, multiple
  // communities can be assigned.
  fireEvent.click(screen.getByRole('checkbox', { name: /Welcome Team/ }))
  fireEvent.click(screen.getByRole('checkbox', { name: /Research/ }))

  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  expect(state.savedUpdates[0].sources[0].communities).toEqual([
    'builtin-team:welcome',
    'team:research',
  ])
  expect(state.savedUpdates[0].sources[0].community_names).toEqual(['Welcome Team', 'Research'])
  expect(state.saved?.sources[0].communities).toEqual(['builtin-team:welcome', 'team:research'])
  expect(state.saved?.sources[0].community_names).toEqual(['Welcome Team', 'Research'])

  // The shell hands the saved snapshot back to the view; refresh the prop so
  // the next interaction starts from the persisted assignment.
  rerender()

  // Unchecking one community removes exactly that id and its aligned name.
  fireEvent.click(screen.getByRole('checkbox', { name: /Research/ }))
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(2))
  expect(state.savedUpdates[1].sources[0].communities).toEqual(['builtin-team:welcome'])
  expect(state.savedUpdates[1].sources[0].community_names).toEqual(['Welcome Team'])
})

test('buzz community chooser refuses to discover unsaved changes and surfaces failures', async () => {
  await renderBuzzSettings()

  // Editing the source makes the native command unsafe until it is saved, so
  // the discovery button is disabled and no IPC call can start.
  fireEvent.change(screen.getByLabelText(/^Source name/), {
    target: { value: 'agent-buzz-renamed' },
  })
  const discoverButton = screen.getByRole('button', {
    name: /Discover communities/,
  }) as HTMLButtonElement
  expect(discoverButton.disabled).toBe(true)
  fireEvent.click(discoverButton)
  expect(state.discoverCalls).toEqual([])

  // After saving the edit, a native failure surfaces the bounded diagnostic.
  state.communitiesError = new Error(
    'Buzz community discovery failed; check the configured Buzz data directory: Buzz community discovery for agent-buzz-renamed found no identity file at /Users/you/Library/Application Support/xyz.block.buzz.app/agents/teams.json; make sure the Buzz data directory is configured as the source root and Buzz has written agents/teams.json'
  )
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  fireEvent.click(screen.getByRole('button', { name: /Discover communities/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain('found no identity file')
  )
})

test('buzz community chooser warns when discovery is truncated at 100 communities', async () => {
  state.communitiesResult = { ...communities, truncated: true }
  await renderBuzzSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover communities/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(
      'Buzz returned more than 100 communities; select from the first 100.'
    )
  )
})

test('buzz community chooser is scoped to the selected workspace', async () => {
  // The chooser renders only sources assigned to the selected workspace tab:
  // a Buzz source assigned to "personal" surfaces under the Personal tab,
  // disappears while the Work tab is active, and reappears with its
  // per-workspace community chooser when switching back.
  state.settings = settingsWith({ ...buzzSource, project: 'personal' })
  await renderBuzzSettings()

  // The workspace tab with sources is selected initially.
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Discover communities/ })).toBeTruthy()
  )

  fireEvent.click(screen.getByRole('tab', { name: /Work/ }))
  await waitFor(() =>
    expect(screen.queryByRole('button', { name: /Discover communities/ })).toBeNull()
  )

  fireEvent.click(screen.getByRole('tab', { name: /Personal/ }))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Discover communities/ })).toBeTruthy()
  )
  fireEvent.click(screen.getByRole('button', { name: /Discover communities/ }))
  await waitFor(() => expect(state.discoverCalls).toEqual(['agent-buzz']))
})
