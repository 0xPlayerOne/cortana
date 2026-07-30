import { AlertTriangle, Check, Plus, Save, Settings2, Trash2 } from 'lucide-react'
import { type FormEvent, type ReactNode, useEffect, useState } from 'react'

import { getDesktopSettings, isDesktopApp, saveDesktopSettings } from '../api'
import type { DesktopSettings, DesktopSettingsUpdate, WorkspaceSettings } from '../types'

type Section = 'workspaces' | 'embedding' | 'query' | 'ingestion' | 'advanced'

export function SettingsView({ onSaved }: { onSaved: (settings: DesktopSettings) => void }) {
  const [settings, setSettings] = useState<DesktopSettings | null>(null)
  const [section, setSection] = useState<Section>('workspaces')
  const [secretValues, setSecretValues] = useState<Record<string, string>>({})
  const [clearedSecrets, setClearedSecrets] = useState<Set<string>>(new Set())
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    if (!isDesktopApp) return
    void getDesktopSettings()
      .then(setSettings)
      .catch((caught: unknown) =>
        setError(caught instanceof Error ? caught.message : 'Unable to load settings')
      )
  }, [])

  const update = (change: (draft: DesktopSettings) => DesktopSettings) => {
    setSettings((current) => (current ? change(current) : current))
    setSaved(false)
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!settings) return
    setSaving(true)
    setError('')
    try {
      const payload: DesktopSettingsUpdate = {
        workspaces: settings.workspaces,
        embedding: settings.embedding,
        query: settings.query,
        ingestion: settings.ingestion,
        runtime: settings.runtime,
        secrets: [
          ...Object.entries(secretValues)
            .filter(([name, value]) => value.length > 0 && !clearedSecrets.has(name))
            .map(([name, value]) => ({ name, value })),
          ...Array.from(clearedSecrets, (name) => ({ name, clear: true })),
        ],
      }
      const next = await saveDesktopSettings(payload)
      setSettings(next)
      setSecretValues({})
      setClearedSecrets(new Set())
      setSaved(true)
      onSaved(next)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Unable to save settings')
    } finally {
      setSaving(false)
    }
  }

  if (!isDesktopApp) {
    return (
      <main className="settings-view settings-unavailable">
        <Settings2 size={34} />
        <h1>Desktop settings</h1>
        <p>Install Cortana Desktop to manage local models, secrets, workspaces, and services.</p>
      </main>
    )
  }

  if (!settings) {
    return (
      <main className="settings-view settings-unavailable">
        <Settings2 size={34} />
        <h1>{error || 'Loading local settings…'}</h1>
      </main>
    )
  }

  return (
    <main className="settings-view">
      <header className="settings-header">
        <div>
          <span className="eyebrow">{settings.needs_setup ? 'Guided setup' : 'Control plane'}</span>
          <h1>Settings</h1>
          <p>Changes are written locally and audited. Secret values never return to this window.</p>
        </div>
        <button className="primary-button" form="settings-form" disabled={saving}>
          <Save size={16} /> {saving ? 'Saving…' : 'Save changes'}
        </button>
      </header>
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {(['workspaces', 'embedding', 'query', 'ingestion', 'advanced'] as Section[]).map(
            (item) => (
              <button
                key={item}
                className={section === item ? 'active' : ''}
                onClick={() => setSection(item)}
              >
                {item[0].toUpperCase() + item.slice(1)}
              </button>
            )
          )}
          <div className="settings-paths">
            <span>Config</span>
            <code title={settings.config_path}>{settings.config_path}</code>
          </div>
        </nav>
        <form id="settings-form" className="settings-form" onSubmit={submit}>
          {section === 'workspaces' && <WorkspaceSection settings={settings} update={update} />}
          {section === 'embedding' && (
            <EmbeddingSection
              settings={settings}
              secretValues={secretValues}
              onSecret={setSecretValues}
              clearedSecrets={clearedSecrets}
              onClearSecret={(name) => {
                setClearedSecrets((current) => new Set(current).add(name))
                setSecretValues((current) => ({ ...current, [name]: '' }))
              }}
              update={update}
            />
          )}
          {section === 'query' && (
            <QuerySection
              settings={settings}
              secrets={settings.secrets}
              secretValues={secretValues}
              onSecret={setSecretValues}
              clearedSecrets={clearedSecrets}
              onClearSecret={(name) => {
                setClearedSecrets((current) => new Set(current).add(name))
                setSecretValues((current) => ({ ...current, [name]: '' }))
              }}
              update={update}
            />
          )}
          {section === 'ingestion' && <IngestionSection settings={settings} update={update} />}
          {section === 'advanced' && <AdvancedSection settings={settings} update={update} />}
        </form>
      </div>
      {(error || saved || settings.restart_required) && (
        <div className={`settings-banner ${error ? 'error' : ''}`} role="status">
          {error ? <AlertTriangle size={16} /> : <Check size={16} />}
          {error ||
            (saved
              ? 'Settings saved. Restart affected services to apply them.'
              : 'A service restart is required.')}
        </div>
      )}
    </main>
  )
}

function WorkspaceSection({
  settings,
  update,
}: {
  settings: DesktopSettings
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
}) {
  const addWorkspace = () =>
    update((current) => ({
      ...current,
      workspaces: [
        ...current.workspaces,
        {
          id: `workspace-${current.workspaces.length + 1}`,
          name: 'New workspace',
          account_label: null,
          color: '#A875D6',
        },
      ],
    }))
  const changeWorkspace = (index: number, patch: Partial<WorkspaceSettings>) =>
    update((current) => ({
      ...current,
      workspaces: current.workspaces.map((workspace, position) =>
        position === index ? { ...workspace, ...patch } : workspace
      ),
    }))
  return (
    <SettingsSection
      title="Workspaces"
      description="Create up to three isolated query scopes. Sources and accounts can be assigned to one scope."
    >
      <div className="workspace-settings-grid">
        {settings.workspaces.map((workspace, index) => (
          <article className="workspace-card" key={`${workspace.id}:${index}`}>
            <div className="workspace-card-heading">
              <i style={{ background: workspace.color || '#E8A83B' }} />
              <strong>Workspace {index + 1}</strong>
              {settings.workspaces.length > 1 && (
                <button
                  type="button"
                  aria-label={`Remove ${workspace.name}`}
                  onClick={() =>
                    update((current) => ({
                      ...current,
                      workspaces: current.workspaces.filter((_, position) => position !== index),
                    }))
                  }
                >
                  <Trash2 size={15} />
                </button>
              )}
            </div>
            <Field label="Display name">
              <input
                value={workspace.name}
                onChange={(event) => changeWorkspace(index, { name: event.target.value })}
                required
                maxLength={80}
              />
            </Field>
            <Field label="Scope ID" hint="lowercase letters, numbers, dashes">
              <input
                value={workspace.id}
                onChange={(event) => changeWorkspace(index, { id: event.target.value })}
                required
                maxLength={32}
                pattern="[a-z0-9][a-z0-9_-]*"
              />
            </Field>
            <Field label="Account label" hint="shown only as a local reminder">
              <input
                value={workspace.account_label || ''}
                onChange={(event) =>
                  changeWorkspace(index, { account_label: event.target.value || null })
                }
                maxLength={128}
                placeholder="team@example.com"
              />
            </Field>
            <Field label="Color">
              <input
                type="color"
                value={workspace.color || '#E8A83B'}
                onChange={(event) => changeWorkspace(index, { color: event.target.value })}
              />
            </Field>
          </article>
        ))}
      </div>
      <button
        type="button"
        className="secondary-button"
        disabled={settings.workspaces.length >= 3}
        onClick={addWorkspace}
      >
        <Plus size={15} /> Add workspace ({settings.workspaces.length}/3)
      </button>
    </SettingsSection>
  )
}

type ProviderValue = DesktopSettings['embedding'] | DesktopSettings['query']

function EmbeddingSection({
  settings,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  update,
}: {
  settings: DesktopSettings
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
}) {
  const setEmbedding = (embedding: DesktopSettings['embedding']) =>
    update((current) => ({ ...current, embedding }))
  return (
    <ProviderSection
      title="Embedding model"
      description="Local Qwen maximizes privacy and cache reuse. Cloud endpoints must use HTTPS."
      provider={settings.embedding}
      secrets={settings.secrets}
      secretValues={secretValues}
      onSecret={onSecret}
      clearedSecrets={clearedSecrets}
      onClearSecret={onClearSecret}
      update={setEmbedding}
    >
      <div className="form-grid compact">
        <NumberField
          label="Vector dimension"
          value={settings.embedding.dimension}
          min={1}
          max={65536}
          onChange={(dimension) => setEmbedding({ ...settings.embedding, dimension })}
        />
        <NumberField
          label="Cache entries"
          value={settings.embedding.cache_max_entries}
          min={100}
          max={5000000}
          onChange={(cache_max_entries) =>
            setEmbedding({ ...settings.embedding, cache_max_entries })
          }
        />
        <NumberField
          label="Request timeout"
          value={settings.embedding.request_timeout_seconds}
          min={1}
          max={3600}
          onChange={(request_timeout_seconds) =>
            setEmbedding({ ...settings.embedding, request_timeout_seconds })
          }
        />
        <NumberField
          label="Request concurrency"
          value={settings.embedding.request_concurrency}
          min={1}
          max={64}
          onChange={(request_concurrency) =>
            setEmbedding({ ...settings.embedding, request_concurrency })
          }
        />
        <NumberField
          label="Startup timeout"
          value={settings.embedding.startup_timeout_seconds}
          min={1}
          max={3600}
          onChange={(startup_timeout_seconds) =>
            setEmbedding({ ...settings.embedding, startup_timeout_seconds })
          }
        />
        <NumberField
          label="Memory limit (MB)"
          value={settings.embedding.memory_limit_mb}
          min={256}
          max={262144}
          onChange={(memory_limit_mb) => setEmbedding({ ...settings.embedding, memory_limit_mb })}
        />
      </div>
    </ProviderSection>
  )
}

function ProviderSection<T extends ProviderValue>({
  title,
  description,
  provider,
  secrets,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  update,
  children,
}: {
  title: string
  description: string
  provider: T
  secrets: DesktopSettings['secrets']
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  update: (provider: T) => void
  children?: ReactNode
}) {
  const secret = provider.api_key_env
    ? secrets.find((item) => item.name === provider.api_key_env)
    : undefined
  return (
    <SettingsSection title={title} description={description}>
      <div className="form-grid">
        <Field label="Provider">
          <select
            value={provider.provider}
            onChange={(event) => {
              const nextProvider = event.target.value as 'local' | 'cloud'
              const loopback = /localhost|127\.0\.0\.1|\[::1\]/.test(provider.base_url)
              const base_url =
                nextProvider === 'cloud' && loopback
                  ? 'https://api.openai.com/v1'
                  : nextProvider === 'local' && !loopback
                    ? title.startsWith('Embedding')
                      ? 'http://127.0.0.1:6999/v1'
                      : 'http://127.0.0.1:8008/v1'
                    : provider.base_url
              update({ ...provider, provider: nextProvider, base_url })
            }}
          >
            <option value="local">Local</option>
            <option value="cloud">Cloud</option>
          </select>
        </Field>
        <Field label="Model">
          <input
            value={provider.model}
            onChange={(event) => update({ ...provider, model: event.target.value })}
            required
            maxLength={256}
          />
        </Field>
        <Field label="OpenAI-compatible endpoint" wide>
          <input
            type="url"
            value={provider.base_url}
            onChange={(event) => update({ ...provider, base_url: event.target.value })}
            required
          />
        </Field>
        <Field
          label="API key variable"
          hint={
            secret?.configured && !clearedSecrets.has(secret.name)
              ? `Configured via ${secret.source}`
              : 'Optional for local providers'
          }
        >
          <input
            value={provider.api_key_env || ''}
            onChange={(event) => update({ ...provider, api_key_env: event.target.value || null })}
            pattern="[A-Z_][A-Z0-9_]*"
            placeholder="CORTANA_PROVIDER_API_KEY"
          />
        </Field>
        <Field label="New API key" hint="write-only; leave blank to keep existing">
          <div className="secret-input">
            <input
              type="password"
              autoComplete="new-password"
              value={provider.api_key_env ? secretValues[provider.api_key_env] || '' : ''}
              disabled={!provider.api_key_env}
              onChange={(event) => {
                if (!provider.api_key_env) return
                onSecret({ ...secretValues, [provider.api_key_env]: event.target.value })
              }}
            />
            {provider.api_key_env && secret?.configured && !clearedSecrets.has(secret.name) && (
              <button type="button" onClick={() => onClearSecret(provider.api_key_env!)}>
                Clear
              </button>
            )}
          </div>
        </Field>
      </div>
      {children}
    </SettingsSection>
  )
}

function QuerySection({
  settings,
  secrets,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  update,
}: {
  settings: DesktopSettings
  secrets: DesktopSettings['secrets']
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
}) {
  const setQuery = (query: DesktopSettings['query']) => update((current) => ({ ...current, query }))
  return (
    <ProviderSection
      title="Query and answer model"
      description="Retrieval always works locally. Enable synthesis to create grounded answers with citations."
      provider={settings.query}
      secrets={secrets}
      secretValues={secretValues}
      onSecret={onSecret}
      clearedSecrets={clearedSecrets}
      onClearSecret={onClearSecret}
      update={setQuery}
    >
      <label className="toggle-row">
        <input
          type="checkbox"
          checked={settings.query.synthesis_enabled}
          onChange={(event) =>
            setQuery({ ...settings.query, synthesis_enabled: event.target.checked })
          }
        />
        <span>
          <strong>Grounded answer synthesis</strong>
          <small>
            Uses retrieved evidence and validates citation indices before returning an answer.
          </small>
        </span>
      </label>
      <div className="form-grid compact">
        <NumberField
          label="Planned queries"
          value={settings.query.max_planned_queries}
          min={1}
          max={8}
          onChange={(max_planned_queries) => setQuery({ ...settings.query, max_planned_queries })}
        />
        <NumberField
          label="Retrieval candidates"
          value={settings.query.retrieval_limit}
          min={1}
          max={100}
          onChange={(retrieval_limit) => setQuery({ ...settings.query, retrieval_limit })}
        />
        <NumberField
          label="Evidence results"
          value={settings.query.result_limit}
          min={1}
          max={50}
          onChange={(result_limit) => setQuery({ ...settings.query, result_limit })}
        />
        <NumberField
          label="Context tokens"
          value={settings.query.context_tokens}
          min={256}
          max={131072}
          onChange={(context_tokens) => setQuery({ ...settings.query, context_tokens })}
        />
        <NumberField
          label="Output tokens"
          value={settings.query.output_tokens}
          min={64}
          max={32768}
          onChange={(output_tokens) => setQuery({ ...settings.query, output_tokens })}
        />
        <NumberField
          label="Request timeout"
          value={settings.query.request_timeout_seconds}
          min={1}
          max={600}
          onChange={(request_timeout_seconds) =>
            setQuery({ ...settings.query, request_timeout_seconds })
          }
        />
        <NumberField
          label="Answer timeout"
          value={settings.query.answer_timeout_seconds}
          min={1}
          max={600}
          onChange={(answer_timeout_seconds) =>
            setQuery({ ...settings.query, answer_timeout_seconds })
          }
        />
        <NumberField
          label="Request concurrency"
          value={settings.query.request_concurrency}
          min={1}
          max={32}
          onChange={(request_concurrency) => setQuery({ ...settings.query, request_concurrency })}
        />
        <NumberField
          label="Cache entries"
          value={settings.query.cache_max_entries}
          min={100}
          max={1000000}
          onChange={(cache_max_entries) => setQuery({ ...settings.query, cache_max_entries })}
        />
        <NumberField
          label="Cache lifetime (seconds)"
          value={settings.query.cache_ttl_seconds}
          min={1}
          max={604800}
          onChange={(cache_ttl_seconds) => setQuery({ ...settings.query, cache_ttl_seconds })}
        />
      </div>
    </ProviderSection>
  )
}

function IngestionSection({ settings, update }: SettingsSectionProps) {
  const setIngestion = (patch: Partial<DesktopSettings['ingestion']>) =>
    update((current) => ({ ...current, ingestion: { ...current.ingestion, ...patch } }))
  return (
    <SettingsSection
      title="Ingestion safety budgets"
      description="These hard limits protect the machine even when a connector returns more data than expected. Scheduled sync remains opt-in."
    >
      <div className="form-grid compact">
        <NumberField
          label="Documents per source"
          value={settings.ingestion.max_documents_per_source}
          min={1}
          max={1000000}
          onChange={(max_documents_per_source) => setIngestion({ max_documents_per_source })}
        />
        <NumberField
          label="Bytes per source"
          value={settings.ingestion.max_bytes_per_source}
          min={1024}
          max={1099511627776}
          onChange={(max_bytes_per_source) => setIngestion({ max_bytes_per_source })}
        />
        <NumberField
          label="Duration seconds"
          value={settings.ingestion.max_duration_seconds}
          min={1}
          max={86400}
          onChange={(max_duration_seconds) => setIngestion({ max_duration_seconds })}
        />
        <NumberField
          label="Document batch size"
          value={settings.ingestion.document_batch_size}
          min={1}
          max={2048}
          onChange={(document_batch_size) => setIngestion({ document_batch_size })}
        />
        <NumberField
          label="Request concurrency"
          value={settings.ingestion.request_concurrency}
          min={1}
          max={32}
          onChange={(request_concurrency) => setIngestion({ request_concurrency })}
        />
      </div>
      <div className="safety-note">
        <AlertTriangle size={16} />
        <span>
          Saving these values does not start a sync. Source authorization and bounded sync controls
          are managed separately.
        </span>
      </div>
    </SettingsSection>
  )
}

function AdvancedSection({ settings, update }: SettingsSectionProps) {
  const setRuntime = (patch: Partial<DesktopSettings['runtime']>) =>
    update((current) => ({ ...current, runtime: { ...current.runtime, ...patch } }))
  return (
    <SettingsSection
      title="Local runtime"
      description="Storage and audit configuration for this machine. Moving the data directory requires a restart and does not copy existing data."
    >
      <div className="form-grid">
        <Field label="Data directory" wide>
          <input
            value={settings.runtime.data_dir}
            onChange={(event) => setRuntime({ data_dir: event.target.value })}
            required
          />
        </Field>
        <NumberField
          label="Connector timeout"
          value={settings.runtime.connector_timeout_seconds}
          min={1}
          max={86400}
          onChange={(connector_timeout_seconds) => setRuntime({ connector_timeout_seconds })}
        />
        <NumberField
          label="Audit event limit"
          value={settings.runtime.audit_max_events}
          min={100}
          max={1000000}
          onChange={(audit_max_events) => setRuntime({ audit_max_events })}
        />
      </div>
    </SettingsSection>
  )
}

type SettingsSectionProps = {
  settings: DesktopSettings
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
}

function SettingsSection({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section className="settings-section">
      <h2>{title}</h2>
      <p>{description}</p>
      {children}
    </section>
  )
}

function Field({
  label,
  hint,
  wide = false,
  children,
}: {
  label: string
  hint?: string
  wide?: boolean
  children: ReactNode
}) {
  return (
    <label className={`form-field ${wide ? 'wide' : ''}`}>
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  )
}

function NumberField({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
}) {
  return (
    <Field label={label}>
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        onChange={(event) => onChange(Number(event.target.value))}
        required
      />
    </Field>
  )
}
