import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopSettings } from './test/fixtures'
import type {
  DesktopSettings,
  DesktopSettingsUpdate,
  DiscordChannelList,
  DiscordServerList,
  SourceSettings,
} from './types'

afterEach(cleanup)

const realApi = await import('./api')

const channels: DiscordChannelList = {
  truncated: false,
  guilds: [
    {
      id: '175928847299117063',
      name: 'Engineering',
      truncated: false,
      channels: [{ id: '175928847299117064', name: 'release', kind: 'text' }],
    },
    {
      id: '175928847299117067',
      name: 'Community',
      truncated: false,
      channels: [{ id: '175928847299117068', name: 'announcements', kind: 'announcement' }],
    },
  ],
}

const servers: DiscordServerList = {
  truncated: false,
  guilds: [
    { id: '175928847299117063', name: 'Engineering' },
    { id: '175928847299117067', name: 'Community' },
  ],
}

const discordSource: SourceSettings = {
  name: 'work-discord',
  kind: 'discord',
  enabled: true,
  project: 'work',
  root: null,
  source: null,
  channels: ['175928847299117064'],
  repositories: [],
  servers: [],
  token_env: 'DISCORD_BOT_TOKEN',
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
  settings: settingsWith(discordSource),
  discoverCalls: [] as string[],
  serversResult: servers as DiscordServerList | null,
  serversError: null as Error | null,
  authorizationCalls: [] as string[],
  savedUpdates: [] as DesktopSettingsUpdate[],
  saved: null as DesktopSettings | null,
}

beforeEach(() => {
  state.settings = settingsWith(discordSource)
  state.discoverCalls = []
  state.serversResult = servers
  state.serversError = null
  state.authorizationCalls = []
  state.savedUpdates = []
  state.saved = null
})

mock.module('./api', () => ({
  ...realApi,
  isDesktopApp: true,
  getDesktopSettings: () => Promise.resolve(state.settings),
  getDesktopInfo: () =>
    Promise.resolve({
      desktop_version: '0.27.3',
      backend_origin: 'http://127.0.0.1:7331',
      autostart_enabled: false,
      platform: 'macos',
    }),
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
  listDesktopDiscordChannels: () => Promise.resolve(channels),
  listDesktopDiscordServers: (source: string) => {
    state.discoverCalls.push(source)
    if (state.serversError) return Promise.reject(state.serversError)
    return Promise.resolve(state.serversResult)
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
      hindsight: update.hindsight,
      honcho: update.honcho,
      ingestion: update.ingestion,
      runtime: update.runtime,
    }
    state.settings = saved
    state.saved = saved
    return Promise.resolve(saved)
  },
}))

const { SettingsView } = await import('./components/SettingsView')

function renderDiscordSettings() {
  render(
    <SettingsView
      initialSection="sources"
      desktopSettings={state.settings}
      onSaved={(next) => {
        state.saved = next
      }}
    />
  )
}

test('discord server chooser discovers guilds and persists per-workspace assignment', async () => {
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover servers/ }))
  await waitFor(() => expect(screen.getByText('Engineering')).toBeTruthy())
  expect(state.discoverCalls).toEqual(['work-discord'])
  expect(screen.getByText('Community')).toBeTruthy()

  // Server selection lands in the `servers` field, which is persisted per
  // source (each Discord source belongs to exactly one workspace).
  fireEvent.click(screen.getByRole('checkbox', { name: /Engineering/ }))
  fireEvent.click(screen.getByRole('checkbox', { name: /Community/ }))

  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  expect(state.savedUpdates[0].sources[0].servers).toEqual([
    '175928847299117063',
    '175928847299117067',
  ])
  expect(state.saved?.sources[0].servers).toEqual(['175928847299117063', '175928847299117067'])
})

test('discord server chooser refuses to discover unsaved changes and surfaces failures', async () => {
  renderDiscordSettings()

  // Editing the source makes the native command unsafe until it is saved, so
  // the discovery button is disabled and no IPC call can start.
  fireEvent.change(screen.getByLabelText(/^Source name/), {
    target: { value: 'work-discord-renamed' },
  })
  const discoverButton = screen.getByRole('button', {
    name: /Discover servers/,
  }) as HTMLButtonElement
  expect(discoverButton.disabled).toBe(true)
  fireEvent.click(discoverButton)
  expect(state.discoverCalls).toEqual([])

  // After saving the edit, a native failure surfaces the bounded diagnostic.
  state.serversError = new Error(
    'Discord server discovery failed; check browser authorization: Discord server discovery requires browser authorization; run `cortana authorize-discord work-discord-renamed` first'
  )
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  fireEvent.click(screen.getByRole('button', { name: /Discover servers/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain('requires browser authorization')
  )
})

test('discord server chooser warns when discovery is truncated at 100 servers', async () => {
  state.serversResult = { ...servers, truncated: true }
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover servers/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(
      'Discord returned more than 100 servers; select from the first 100.'
    )
  )
})

test('discord channels outside assigned servers are marked when servers are assigned', async () => {
  state.settings = settingsWith({
    ...discordSource,
    servers: ['175928847299117063'],
  })
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover channels/ }))
  await waitFor(() => expect(screen.getByText('release · text')).toBeTruthy())
  // The unassigned guild is labeled; the assigned guild is not.
  expect(screen.getByText(/not assigned to this workspace/)).toBeTruthy()
  const engineering = screen.getByText('Engineering')
  expect(engineering.closest('.discord-guild')?.className ?? '').not.toContain(
    'discord-guild-unassigned'
  )
  const community = screen.getAllByText('Community')[0]
  expect(community.closest('.discord-guild')?.className ?? '').toContain('discord-guild-unassigned')
})

test('discord authorize action names Discord and starts browser authorization', async () => {
  state.settings = settingsWith({
    ...discordSource,
    token_path: '/Users/you/.config/cortana/discord-user-token.json',
    oauth_client_path: '/Users/you/.config/cortana/discord-oauth-client.json',
  })
  renderDiscordSettings()

  const confirm = mock((message?: string) => {
    confirmMessage = message ?? ''
    return true
  })
  let confirmMessage = ''
  const originalConfirm = window.confirm
  window.confirm = confirm

  fireEvent.click(screen.getByRole('button', { name: 'Authorize' }))
  await waitFor(() => expect(state.authorizationCalls).toEqual(['work-discord']))
  expect(confirmMessage).toContain('Authorize work-discord with Discord')

  window.confirm = originalConfirm
})

test('discord authorize action stays disabled until OAuth paths are saved', async () => {
  renderDiscordSettings()

  const authorize = screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement
  expect(authorize.disabled).toBe(true)
  expect(state.authorizationCalls).toEqual([])

  // A token destination without a client JSON is still incomplete, and the
  // native runtime must not be invoked with unsaved edits anyway.
  fireEvent.change(
    screen.getByPlaceholderText('/Users/you/.config/cortana/discord-user-token.json'),
    { target: { value: '/Users/you/.config/cortana/discord-user-token.json' } }
  )
  fireEvent.change(
    screen.getByPlaceholderText('/Users/you/.config/cortana/discord-oauth-client.json'),
    { target: { value: '/Users/you/.config/cortana/discord-oauth-client.json' } }
  )
  expect((screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement).disabled).toBe(
    true
  )

  // Once the paths are saved, the same source card offers browser
  // authorization for Discord.
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  await waitFor(() =>
    expect((screen.getByRole('button', { name: 'Authorize' }) as HTMLButtonElement).disabled).toBe(
      false
    )
  )
  expect(state.saved?.sources[0].token_path).toBe(
    '/Users/you/.config/cortana/discord-user-token.json'
  )
  expect(state.saved?.sources[0].oauth_client_path).toBe(
    '/Users/you/.config/cortana/discord-oauth-client.json'
  )
})
