import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopInfo, desktopSettings } from './test/fixtures'
import type {
  DesktopSettings,
  DesktopSettingsUpdate,
  SlackWorkspaceList,
  SourceSettings,
} from './types'

afterEach(cleanup)

const realApi = await import('./api')

const workspaces: SlackWorkspaceList = {
  truncated: false,
  teams: [
    { id: 'T0123456789', name: 'Acme Engineering' },
    { id: 'T9876543210', name: 'Acme Community' },
  ],
}

const slackSource: SourceSettings = {
  name: 'work-slack',
  kind: 'slack',
  enabled: true,
  project: 'work',
  root: null,
  source: null,
  channels: ['C0123456789'],
  repositories: [],
  servers: [],
  teams: [],
  team_names: [],
  communities: [],
  community_names: [],
  token_env: 'SLACK_BOT_TOKEN',
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
  settings: settingsWith(slackSource),
  discoverCalls: [] as string[],
  workspacesResult: workspaces as SlackWorkspaceList | null,
  workspacesError: null as Error | null,
  authorizationCalls: [] as string[],
  savedUpdates: [] as DesktopSettingsUpdate[],
  saved: null as DesktopSettings | null,
}

beforeEach(() => {
  state.settings = settingsWith(slackSource)
  state.discoverCalls = []
  state.workspacesResult = workspaces
  state.workspacesError = null
  state.authorizationCalls = []
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
  listDesktopSlackWorkspaces: (source: string) => {
    state.discoverCalls.push(source)
    if (state.workspacesError) return Promise.reject(state.workspacesError)
    return Promise.resolve(state.workspacesResult)
  },
  startDesktopSourceAuthorization: (source: string) => {
    state.authorizationCalls.push(source)
    return Promise.reject(new Error('authorization job cannot start in tests'))
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

async function renderSlackSettings() {
  render(
    <SettingsView
      initialSection="sources"
      desktopSettings={state.settings}
      onSaved={(next) => {
        state.saved = next
      }}
    />
  )
  await screen.findByLabelText(/^Source name/)
}

test('slack workspace chooser discovers teams and persists per-workspace assignment', async () => {
  await renderSlackSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover workspaces/ }))
  await waitFor(() => expect(screen.getByText('Acme Engineering')).toBeTruthy())
  expect(state.discoverCalls).toEqual(['work-slack'])
  expect(screen.getByText('Acme Community')).toBeTruthy()

  // Team selection lands in the `teams` field with the display name kept
  // index-aligned in `team_names`, persisted per source (each Slack source
  // belongs to exactly one workspace). A Slack user token is scoped to
  // exactly one workspace, so assigning a second team replaces the first.
  fireEvent.click(screen.getByRole('checkbox', { name: /Acme Engineering/ }))
  fireEvent.click(screen.getByRole('checkbox', { name: /Acme Community/ }))

  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  expect(state.savedUpdates[0].sources[0].teams).toEqual(['T9876543210'])
  expect(state.savedUpdates[0].sources[0].team_names).toEqual(['Acme Community'])
  expect(state.saved?.sources[0].teams).toEqual(['T9876543210'])
  expect(state.saved?.sources[0].team_names).toEqual(['Acme Community'])
})

test('slack workspace chooser refuses to discover unsaved changes and surfaces failures', async () => {
  await renderSlackSettings()

  // Editing the source makes the native command unsafe until it is saved, so
  // the discovery button is disabled and no IPC call can start.
  fireEvent.change(screen.getByLabelText(/^Source name/), {
    target: { value: 'work-slack-renamed' },
  })
  const discoverButton = screen.getByRole('button', {
    name: /Discover workspaces/,
  }) as HTMLButtonElement
  expect(discoverButton.disabled).toBe(true)
  fireEvent.click(discoverButton)
  expect(state.discoverCalls).toEqual([])

  // After saving the edit, a native failure surfaces the bounded diagnostic.
  state.workspacesError = new Error(
    'Slack workspace discovery failed; check browser authorization: Slack workspace discovery requires browser authorization; run `cortana authorize-slack work-slack-renamed` first'
  )
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  fireEvent.click(screen.getByRole('button', { name: /Discover workspaces/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain('requires browser authorization')
  )
})

test('slack workspace chooser warns when discovery is truncated at 100 teams', async () => {
  state.workspacesResult = { ...workspaces, truncated: true }
  await renderSlackSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover workspaces/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(
      'Slack returned more than 100 teams; select from the first 100.'
    )
  )
})

test('slack authorize action names Slack and starts browser authorization', async () => {
  state.settings = settingsWith({
    ...slackSource,
    token_path: '/Users/you/.config/cortana/slack-user-token.json',
    oauth_client_path: '/Users/you/.config/cortana/slack-oauth-client.json',
  })
  await renderSlackSettings()

  const confirm = mock((message?: string) => {
    confirmMessage = message ?? ''
    return true
  })
  let confirmMessage = ''
  const originalConfirm = window.confirm
  window.confirm = confirm

  fireEvent.click(screen.getByRole('button', { name: 'Authorize' }))
  await waitFor(() => expect(state.authorizationCalls).toEqual(['work-slack']))
  expect(confirmMessage).toContain('Authorize work-slack with Slack')

  window.confirm = originalConfirm
})

test('slack authorize action stays disabled until OAuth paths are saved', async () => {
  await renderSlackSettings()

  const authorize = screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement
  expect(authorize.disabled).toBe(true)
  expect(state.authorizationCalls).toEqual([])

  // A token destination without a client JSON is still incomplete, and the
  // native runtime must not be invoked with unsaved edits anyway.
  fireEvent.change(
    screen.getByPlaceholderText('/Users/you/.config/cortana/slack-user-token.json'),
    { target: { value: '/Users/you/.config/cortana/slack-user-token.json' } }
  )
  fireEvent.change(
    screen.getByPlaceholderText('/Users/you/.config/cortana/slack-oauth-client.json'),
    { target: { value: '/Users/you/.config/cortana/slack-oauth-client.json' } }
  )
  expect((screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement).disabled).toBe(
    true
  )

  // Once the paths are saved, the same source card offers browser
  // authorization for Slack.
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  await waitFor(() =>
    expect((screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement).disabled).toBe(
      false
    )
  )
  expect(state.saved?.sources[0].token_path).toBe(
    '/Users/you/.config/cortana/slack-user-token.json'
  )
  expect(state.saved?.sources[0].oauth_client_path).toBe(
    '/Users/you/.config/cortana/slack-oauth-client.json'
  )
})
