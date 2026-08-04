import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopSettings } from './test/fixtures'
import type {
  DesktopSettings,
  DesktopSettingsUpdate,
  DiscordChannelList,
  SourceSettings,
} from './types'

afterEach(cleanup)

const realApi = await import('./api')

const discovery: DiscordChannelList = {
  truncated: false,
  guilds: [
    {
      id: '175928847299117063',
      name: 'Engineering',
      truncated: false,
      channels: [
        { id: '175928847299117064', name: 'release', kind: 'text' },
        { id: '175928847299117065', name: 'standup', kind: 'text' },
        { id: '175928847299117066', name: 'Town Hall', kind: 'voice' },
      ],
    },
    {
      id: '175928847299117067',
      name: 'Community',
      truncated: false,
      channels: [{ id: '175928847299117068', name: 'announcements', kind: 'announcement' }],
    },
  ],
}

const discordSource: SourceSettings = {
  name: 'work-discord',
  kind: 'discord',
  enabled: true,
  project: 'work',
  root: null,
  source: null,
  channels: [],
  repositories: [],
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
  discoveryResult: discovery as DiscordChannelList | null,
  discoveryError: null as Error | null,
  savedUpdates: [] as DesktopSettingsUpdate[],
  saved: null as DesktopSettings | null,
}

beforeEach(() => {
  state.settings = settingsWith(discordSource)
  state.discoverCalls = []
  state.discoveryResult = discovery
  state.discoveryError = null
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
  listDesktopDiscordChannels: (source: string) => {
    state.discoverCalls.push(source)
    if (state.discoveryError) return Promise.reject(state.discoveryError)
    return Promise.resolve(state.discoveryResult)
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

function channelIdsTextarea(): HTMLTextAreaElement {
  return screen.getByLabelText('Channel IDs') as HTMLTextAreaElement
}

test('discord chooser discovers guilds and channels and persists selected snowflake ids', async () => {
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover channels/ }))
  await waitFor(() => expect(screen.getByText('Engineering')).toBeTruthy())
  expect(state.discoverCalls).toEqual(['work-discord'])
  expect(screen.getByText('Community')).toBeTruthy()
  expect(screen.getByText('release · text')).toBeTruthy()
  expect(screen.getByText('standup · text')).toBeTruthy()
  expect(screen.getByText('Town Hall · voice')).toBeTruthy()
  expect(screen.getByText('announcements · announcement')).toBeTruthy()

  // Selections land in the same `channels` field the native runtime reads,
  // as exact snowflake strings (renderer numbers cannot hold 64-bit ids).
  fireEvent.click(screen.getByRole('checkbox', { name: /release · text/ }))
  expect(channelIdsTextarea().value).toBe('175928847299117064')

  fireEvent.click(screen.getByRole('checkbox', { name: /announcements · announcement/ }))
  expect(channelIdsTextarea().value).toBe('175928847299117064, 175928847299117068')

  fireEvent.click(screen.getByRole('checkbox', { name: /release · text/ }))
  expect(channelIdsTextarea().value).toBe('175928847299117068')

  // Saving persists the selected channel ids through the settings bridge.
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  expect(state.savedUpdates[0].sources[0].channels).toEqual(['175928847299117068'])
  expect(state.saved?.sources[0].channels).toEqual(['175928847299117068'])
})

test('discord chooser refuses to discover unsaved changes and surfaces failures', async () => {
  // A selected channel keeps the required Channel IDs field satisfied so the
  // rename can be saved and the failure path reached.
  state.settings = settingsWith({ ...discordSource, channels: ['175928847299117064'] })
  renderDiscordSettings()

  // Editing the source makes the native command unsafe until it is saved, so
  // the discovery button is disabled and no IPC call can start.
  fireEvent.change(screen.getByLabelText(/^Source name/), {
    target: { value: 'work-discord-renamed' },
  })
  const discoverButton = screen.getByRole('button', {
    name: /Discover channels/,
  }) as HTMLButtonElement
  expect(discoverButton.disabled).toBe(true)
  fireEvent.click(discoverButton)
  expect(state.discoverCalls).toEqual([])

  // After saving the edit, a native failure surfaces the CLI's bounded,
  // token-free diagnostic.
  state.discoveryError = new Error(
    'Discord channel discovery failed; check the configured bot token: Discord bot token environment variable DISCORD_BOT_TOKEN is not configured'
  )
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
  await waitFor(() => expect(state.savedUpdates).toHaveLength(1))
  expect(state.savedUpdates[0].sources[0].name).toBe('work-discord-renamed')
  fireEvent.click(screen.getByRole('button', { name: /Discover channels/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(
      'Discord bot token environment variable DISCORD_BOT_TOKEN is not configured'
    )
  )
  expect(state.saved?.sources[0].channels).toEqual(['175928847299117064'])
})

test('discord chooser warns when discovery is truncated at 100 servers', async () => {
  state.discoveryResult = { ...discovery, truncated: true }
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover channels/ }))
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(
      'Discord returned more than 100 servers; select from the first 100.'
    )
  )
})

test('discord chooser marks a server with more than 100 channels as truncated', async () => {
  const manyChannels = Array.from({ length: 100 }, (_, index) => ({
    id: `1759288472991${String(index).padStart(6, '0')}`,
    name: `channel-${index}`,
    kind: 'text',
  }))
  state.discoveryResult = {
    truncated: false,
    guilds: [
      {
        id: '175928847299117063',
        name: 'Engineering',
        truncated: true,
        channels: manyChannels,
      },
    ],
  }
  renderDiscordSettings()

  fireEvent.click(screen.getByRole('button', { name: /Discover channels/ }))
  await waitFor(() => expect(screen.getByText('channel-99 · text')).toBeTruthy())
  // The per-server truncation marker is always rendered so the user knows
  // the persisted selection is limited to the returned channels.
  expect(screen.getByText(/first 100 channels/)).toBeTruthy()
  fireEvent.click(screen.getByRole('checkbox', { name: /channel-0 · text/ }))
  expect(channelIdsTextarea().value).toBe('1759288472991000000')
})
