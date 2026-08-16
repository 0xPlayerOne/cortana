import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

import { desktopInfo, desktopSettings } from './test/fixtures'
import type {
  DesktopSettings,
  DesktopSettingsUpdate,
  ProviderModelKind,
  ProviderModelList,
} from './types'

afterEach(cleanup)

const realApi = await import('./api')

const advertised: ProviderModelList = {
  kind: 'embedding',
  provider: 'https://api.openai.com/v1',
  truncated: false,
  models: [
    {
      id: 'text-embedding-3-small',
      object: 'model',
      owned_by: 'openai',
      created: 1677610602,
      capabilities: null,
    },
    {
      id: 'text-embedding-3-large',
      object: 'model',
      owned_by: 'openai',
      created: 1677610602,
      capabilities: ['embedding'],
    },
  ],
}

function cloudSettings(): DesktopSettings {
  return {
    ...desktopSettings,
    embedding: {
      ...desktopSettings.embedding,
      provider: 'cloud',
      base_url: 'https://api.openai.com/v1',
      model: 'provider-custom-embedding',
      api_key_env: 'CORTANA_OPENAI_API_KEY',
    },
    query: {
      ...desktopSettings.query,
      provider: 'cloud',
      base_url: 'https://api.openai.com/v1',
      model: 'provider-custom-embedding',
      api_key_env: 'CORTANA_OPENAI_API_KEY',
    },
  }
}

const state = {
  settings: cloudSettings(),
  refreshCalls: [] as ProviderModelKind[],
  refreshResult: advertised as ProviderModelList | null,
  refreshError: null as Error | null,
  savedUpdates: [] as DesktopSettingsUpdate[],
  saved: null as DesktopSettings | null,
}

beforeEach(() => {
  state.settings = cloudSettings()
  state.refreshCalls = []
  state.refreshResult = advertised
  state.refreshError = null
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
  listDesktopProviderModels: (kind: ProviderModelKind) => {
    state.refreshCalls.push(kind)
    if (state.refreshError) return Promise.reject(state.refreshError)
    return Promise.resolve(state.refreshResult)
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

function renderEmbeddingSettings() {
  render(
    <SettingsView
      initialSection="embedding"
      desktopSettings={state.settings}
      onSaved={(next) => {
        state.saved = next
      }}
    />
  )
}

function modelSelect(): HTMLSelectElement {
  return screen.getByLabelText('Model catalog') as HTMLSelectElement
}

function modelInput(): HTMLInputElement {
  return screen.getByLabelText('Model') as HTMLInputElement
}

test('refresh exposes provider-advertised models without stale cloud presets', async () => {
  // The current model is one the provider advertises, so the select stays.
  state.settings.embedding.model = 'text-embedding-3-small'
  renderEmbeddingSettings()

  // Cloud models are not maintained as a stale static list; discovery is the
  // source of truth and the current value is preserved as custom until then.
  expect(screen.queryByLabelText('Model catalog')).toBeNull()
  expect(modelInput().value).toBe('text-embedding-3-small')

  fireEvent.click(screen.getByRole('button', { name: /Refresh Embedding model models/ }))

  await waitFor(() => {
    const values = Array.from(modelSelect().options).map((option) => option.value)
    expect(values).toContain('text-embedding-3-small')
    expect(values).toContain('text-embedding-3-large')
  })
  expect(state.refreshCalls).toEqual(['embedding'])
  expect(screen.getByText(/2 models advertised by the provider/)).toBeTruthy()
})

test('a current model that is not advertised falls back to the custom field unchanged', async () => {
  state.settings.embedding.model = 'provider-custom-embedding'
  renderEmbeddingSettings()

  fireEvent.click(screen.getByRole('button', { name: /Refresh Embedding model models/ }))

  // Flush the refresh continuation (scheduled outside `fireEvent`'s act scope)
  // so the derived custom fallback commits deterministically.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  expect(screen.queryByLabelText('Model catalog')).toBeNull()
  expect(modelInput().value).toBe('provider-custom-embedding')
})

test('selecting an advertised model updates the provider settings', async () => {
  state.settings.embedding.model = 'text-embedding-3-small'
  renderEmbeddingSettings()
  fireEvent.click(screen.getByRole('button', { name: /Refresh Embedding model models/ }))
  await waitFor(() => {
    expect(Array.from(modelSelect().options).map((option) => option.value)).toContain(
      'text-embedding-3-large'
    )
  })

  fireEvent.change(modelSelect(), { target: { value: 'text-embedding-3-large' } })
  expect(modelSelect().value).toBe('text-embedding-3-large')

  fireEvent.click(screen.getByRole('button', { name: /Save/ }))
  await waitFor(() => {
    expect(state.saved?.embedding.model).toBe('text-embedding-3-large')
  })
})

test('failed discovery keeps the explicit model and reports the error', async () => {
  state.refreshError = new Error('provider /models request failed with status 404')
  renderEmbeddingSettings()

  fireEvent.click(screen.getByRole('button', { name: /Refresh Embedding model models/ }))

  await waitFor(() => {
    expect(screen.getByText(/provider \/models request failed with status 404/)).toBeTruthy()
  })
  expect(screen.queryByLabelText('Model catalog')).toBeNull()
  expect(modelInput().value).toBe('provider-custom-embedding')
})

test('changing the endpoint invalidates the advertised catalog', async () => {
  state.settings.embedding.model = 'text-embedding-3-small'
  renderEmbeddingSettings()
  fireEvent.click(screen.getByRole('button', { name: /Refresh Embedding model models/ }))
  await waitFor(() => {
    expect(Array.from(modelSelect().options).map((option) => option.value)).toContain(
      'text-embedding-3-large'
    )
  })

  // The user edits the endpoint; the stale advertised list must not apply to
  // the new provider.
  const endpoint = screen.getByLabelText('OpenAI-compatible endpoint') as HTMLInputElement
  fireEvent.change(endpoint, { target: { value: 'https://other.example.test/v1' } })

  await waitFor(() => {
    expect(screen.queryByLabelText('Model catalog')).toBeNull()
    expect(modelInput().value).toBe('text-embedding-3-small')
  })
})

test('query section refreshes the query provider separately', async () => {
  state.refreshResult = {
    kind: 'query',
    provider: 'https://api.openai.com/v1',
    truncated: true,
    models: [
      {
        id: 'provider-chat-large',
        object: 'model',
        owned_by: 'openai',
        created: null,
        capabilities: null,
      },
      {
        id: 'provider-custom-embedding',
        object: 'model',
        owned_by: 'openai',
        created: null,
        capabilities: null,
      },
      { id: 'o3-mini', object: 'model', owned_by: 'openai', created: null, capabilities: null },
    ],
  }
  render(
    <SettingsView
      initialSection="query"
      desktopSettings={state.settings}
      onSaved={(next) => {
        state.saved = next
      }}
    />
  )

  fireEvent.click(screen.getByRole('button', { name: /Refresh Query and answer model models/ }))

  await waitFor(() => {
    const values = Array.from(modelSelect().options).map((option) => option.value)
    expect(values).toContain('o3-mini')
    expect(values).toContain('provider-chat-large')
  })
  expect(state.refreshCalls).toEqual(['query'])
  expect(screen.getByText(/first 512 shown/)).toBeTruthy()
})
