import {
  AlertTriangle,
  Check,
  CircleStop,
  ExternalLink,
  FolderOpen,
  KeyRound,
  LoaderCircle,
  Play,
  Plus,
  RefreshCw,
  Save,
  Settings2,
  Trash2,
  X,
} from 'lucide-react'
import { type FormEvent, type ReactNode, useEffect, useState } from 'react'

import {
  cancelDesktopInstaller,
  cancelDesktopSourceValidation,
  getDesktopInstaller,
  getDesktopSourceValidation,
  getDesktopSettings,
  isDesktopApp,
  openDesktopSourceSetup,
  pickDesktopPath,
  saveDesktopSettings,
  scanDesktopReadiness,
  startDesktopInstaller,
  startDesktopSourceAuthorization,
  startDesktopSourceValidation,
} from '../api'
import type {
  DesktopInstallJob,
  DesktopReadiness,
  DesktopSettings,
  DesktopSettingsUpdate,
  DesktopSourceJob,
  SourceKind,
  SourceSettings,
  WorkspaceSettings,
} from '../types'

type Section =
  'readiness' | 'workspaces' | 'sources' | 'embedding' | 'query' | 'ingestion' | 'advanced'

export function SettingsView({ onSaved }: { onSaved: (settings: DesktopSettings) => void }) {
  const [settings, setSettings] = useState<DesktopSettings | null>(null)
  const [section, setSection] = useState<Section>('readiness')
  const [secretValues, setSecretValues] = useState<Record<string, string>>({})
  const [clearedSecrets, setClearedSecrets] = useState<Set<string>>(new Set())
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [dirty, setDirty] = useState(false)

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
    setDirty(true)
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!settings) return
    setSaving(true)
    setError('')
    try {
      const payload: DesktopSettingsUpdate = {
        workspaces: settings.workspaces,
        sources: settings.sources,
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
      setDirty(false)
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
          {(
            [
              'readiness',
              'workspaces',
              'sources',
              'embedding',
              'query',
              'ingestion',
              'advanced',
            ] as Section[]
          ).map((item) => (
            <button
              key={item}
              className={section === item ? 'active' : ''}
              onClick={() => setSection(item)}
            >
              {item[0].toUpperCase() + item.slice(1)}
            </button>
          ))}
          <div className="settings-paths">
            <span>Config</span>
            <code title={settings.config_path}>{settings.config_path}</code>
          </div>
        </nav>
        <form id="settings-form" className="settings-form" onSubmit={submit}>
          {section === 'readiness' && <ReadinessSection />}
          {section === 'workspaces' && <WorkspaceSection settings={settings} update={update} />}
          {section === 'sources' && (
            <SourcesSection
              settings={settings}
              update={update}
              canValidate={!dirty && !saving}
              secretValues={secretValues}
              onSecret={(values) => {
                setSecretValues(values)
                setDirty(true)
                setSaved(false)
              }}
              clearedSecrets={clearedSecrets}
              onClearSecret={(name) => {
                setClearedSecrets((current) => new Set(current).add(name))
                setSecretValues((current) => ({ ...current, [name]: '' }))
                setDirty(true)
                setSaved(false)
              }}
            />
          )}
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

function ReadinessSection() {
  const [readiness, setReadiness] = useState<DesktopReadiness | null>(null)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState('')
  const [job, setJob] = useState<DesktopInstallJob | null>(null)

  useEffect(() => {
    if (!job || !['running', 'cancelling'].includes(job.status)) return
    let active = true
    const timer = window.setTimeout(() => {
      void getDesktopInstaller(job.id)
        .then((next) => {
          if (!active) return
          setJob(next)
          if (next.status === 'succeeded') setReadiness(null)
        })
        .catch((caught: unknown) => {
          if (active) {
            setError(caught instanceof Error ? caught.message : 'Installer status failed')
          }
        })
    }, 700)
    return () => {
      active = false
      window.clearTimeout(timer)
    }
  }, [job])

  const scan = async () => {
    setScanning(true)
    setError('')
    try {
      setReadiness(await scanDesktopReadiness())
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Readiness scan failed')
    } finally {
      setScanning(false)
    }
  }

  const install = async (tool: string, label: string) => {
    if (
      !window.confirm(
        `Install ${label} on this computer?\n\nCortana will run its fixed, platform-specific installer. No ingestion or sync will start.`
      )
    ) {
      return
    }
    setError('')
    try {
      setJob(await startDesktopInstaller(tool))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Installer failed to start')
    }
  }

  const cancel = async () => {
    if (!job) return
    try {
      setJob(await cancelDesktopInstaller(job.id))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Installer could not be cancelled')
    }
  }

  return (
    <SettingsSection
      title="System readiness"
      description="A read-only scan checks local tools and Cortana's production gates. It never starts a connector, installs a schedule, or writes indexed data."
    >
      <div className="readiness-actions">
        <button type="button" className="secondary-button" disabled={scanning} onClick={scan}>
          {scanning ? <LoaderCircle className="spin" size={15} /> : <RefreshCw size={15} />}
          {scanning ? 'Checking system…' : readiness ? 'Run again' : 'Run readiness scan'}
        </button>
        {readiness && (
          <span>
            Last checked {new Date(readiness.scanned_at_unix_seconds * 1000).toLocaleTimeString()}
          </span>
        )}
      </div>
      {error && (
        <div className="safety-note">
          <AlertTriangle size={16} /> <span>{error}</span>
        </div>
      )}
      {readiness && (
        <>
          <div className="readiness-summary">
            <StatusGlyph passed={readiness.tools_ready} />
            <div>
              <strong>{readiness.tools_ready ? 'Local tools ready' : 'Setup required'}</strong>
              <span>
                {readiness.tools.filter((tool) => tool.required && !tool.available).length} required
                components missing
              </span>
            </div>
          </div>
          <div className="readiness-list">
            {readiness.tools.map((tool) => (
              <article key={tool.id}>
                <StatusGlyph passed={tool.available} optional={!tool.required} />
                <div>
                  <strong>
                    {tool.label} {!tool.required && <small>optional</small>}
                  </strong>
                  <span>{tool.version || tool.detail}</span>
                  {tool.path && <code>{tool.path}</code>}
                </div>
                {!tool.available && tool.install_supported && (
                  <button
                    type="button"
                    disabled={job?.status === 'running' || job?.status === 'cancelling'}
                    onClick={() => void install(tool.id, tool.label)}
                  >
                    Install
                  </button>
                )}
              </article>
            ))}
          </div>
          {job && (
            <div className={`installer-job ${job.status}`} role="status">
              <div>
                {['running', 'cancelling'].includes(job.status) ? (
                  <LoaderCircle className="spin" size={16} />
                ) : (
                  <StatusGlyph passed={job.status === 'succeeded'} />
                )}
                <span>
                  <strong>{job.summary}</strong>
                  <small>Status: {job.status}</small>
                </span>
                {job.status === 'running' && (
                  <button type="button" onClick={() => void cancel()}>
                    Cancel
                  </button>
                )}
                {job.retryable && (
                  <button type="button" onClick={() => void install(job.tool, job.tool)}>
                    Retry
                  </button>
                )}
              </div>
              {job.log && <pre>{job.log}</pre>}
            </div>
          )}
          <div className="core-readiness">
            <h3>Production gates</h3>
            {readiness.core_error && <p>{readiness.core_error}</p>}
            {readiness.core?.checks.map((check) => (
              <article key={check.name}>
                <StatusGlyph passed={check.passed} />
                <div>
                  <strong>{check.name.replaceAll('-', ' ')}</strong>
                  <span>{check.detail}</span>
                </div>
              </article>
            ))}
          </div>
        </>
      )}
    </SettingsSection>
  )
}

function StatusGlyph({ passed, optional = false }: { passed: boolean; optional?: boolean }) {
  return (
    <i className={`status-glyph ${passed ? 'passed' : optional ? 'optional' : 'failed'}`}>
      {passed ? <Check size={13} /> : <X size={13} />}
    </i>
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

const SOURCE_KINDS: Array<{ value: SourceKind; label: string }> = [
  { value: 'filesystem', label: 'Files and code' },
  { value: 'apple-notes', label: 'Apple Notes' },
  { value: 'buzz', label: 'Buzz' },
  { value: 'google-drive', label: 'Google Drive' },
  { value: 'gmail', label: 'Gmail' },
  { value: 'google-calendar', label: 'Google Calendar' },
  { value: 'slack', label: 'Slack' },
  { value: 'discord', label: 'Discord' },
]

function SourcesSection({
  settings,
  update,
  canValidate,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
}: SettingsSectionProps & {
  canValidate: boolean
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
}) {
  const [job, setJob] = useState<DesktopSourceJob | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!job || !['running', 'cancelling'].includes(job.status)) return
    let active = true
    const timer = window.setTimeout(() => {
      void getDesktopSourceValidation(job.id)
        .then((next) => {
          if (active) setJob(next)
        })
        .catch((caught: unknown) => {
          if (active) {
            setError(caught instanceof Error ? caught.message : 'Source validation status failed')
          }
        })
    }, 700)
    return () => {
      active = false
      window.clearTimeout(timer)
    }
  }, [job])

  const changeSource = (index: number, patch: Partial<SourceSettings>) =>
    update((current) => ({
      ...current,
      sources: current.sources.map((source, position) =>
        position === index ? { ...source, ...patch } : source
      ),
    }))

  const addSource = () =>
    update((current) => ({
      ...current,
      sources: [...current.sources, newSource(current)],
    }))

  const validateSource = async (source: SourceSettings) => {
    if (!canValidate) {
      setError(
        'Save source changes before validating so the native runtime uses this exact config.'
      )
      return
    }
    if (
      !window.confirm(
        `Validate ${source.name} now?\n\nCortana may read up to 25 documents or 5 MiB for at most 60 seconds. It will not embed, index, reconcile, or start a sync.`
      )
    ) {
      return
    }
    setError('')
    try {
      setJob(await startDesktopSourceValidation(source.name))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Source validation failed to start')
    }
  }

  const authorizeSource = async (source: SourceSettings) => {
    if (!canValidate) {
      setError(
        'Save source changes before authorizing so the native runtime uses this exact config.'
      )
      return
    }
    if (
      !window.confirm(
        `Authorize ${source.name} with Google?\n\nCortana will open the system browser, listen only on a random 127.0.0.1 callback port, request read-only scopes for Google sources sharing this token, and store the resulting token in the configured private file.`
      )
    ) {
      return
    }
    setError('')
    try {
      setJob(await startDesktopSourceAuthorization(source.name))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Google authorization failed to start')
    }
  }

  const openSetup = async (source: SourceSettings) => {
    if (!canValidate) {
      setError('Save this source before opening its account setup page.')
      return
    }
    setError('')
    try {
      await openDesktopSourceSetup(source.name)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Account setup page could not be opened')
    }
  }

  const choosePath = async (
    index: number,
    kind: 'directory' | 'oauth-client' | 'google-token',
    field: 'root' | 'token_path' | 'oauth_client_path'
  ) => {
    setError('')
    try {
      const path = await pickDesktopPath(kind)
      if (path) changeSource(index, { [field]: path })
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Path selection failed')
    }
  }

  const cancel = async () => {
    if (!job) return
    try {
      setJob(await cancelDesktopSourceValidation(job.id))
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : 'Source validation could not be cancelled'
      )
    }
  }

  return (
    <SettingsSection
      title="Ingestion sources"
      description="Configure local and account-backed sources per workspace. Saving only updates configuration; validation is a separate, read-only bounded action and full sync remains disabled."
    >
      <div className="source-settings-toolbar">
        <span>
          {settings.sources.filter((source) => source.enabled).length} enabled ·{' '}
          {settings.sources.length} configured
        </span>
        <button
          type="button"
          className="secondary-button"
          disabled={settings.sources.length >= 128}
          onClick={addSource}
        >
          <Plus size={15} /> Add source
        </button>
      </div>

      <div className="source-settings-list">
        {settings.sources.length === 0 && (
          <div className="empty-source-settings">
            <strong>No sources configured</strong>
            <span>Add a source, assign its workspace, then save and run bounded validation.</span>
          </div>
        )}
        {settings.sources.map((source, index) => {
          const secret = source.token_env
            ? settings.secrets.find((item) => item.name === source.token_env)
            : undefined
          const runningThis =
            job?.source === source.name && ['running', 'cancelling'].includes(job.status)
          return (
            <article className="source-settings-card" key={`${source.name}:${index}`}>
              <header>
                <label className="source-enable">
                  <input
                    type="checkbox"
                    checked={source.enabled}
                    onChange={(event) => changeSource(index, { enabled: event.target.checked })}
                  />
                  <span>
                    <strong>{source.name || 'New source'}</strong>
                    <small>
                      {SOURCE_KINDS.find((kind) => kind.value === source.kind)?.label ||
                        'External connector'}
                    </small>
                  </span>
                </label>
                <div className="source-card-actions">
                  {hasBrowserSetup(source.kind) && (
                    <button
                      type="button"
                      disabled={!canValidate}
                      title="Open the official provider setup page"
                      onClick={() => void openSetup(source)}
                    >
                      <ExternalLink size={14} /> Setup
                    </button>
                  )}
                  {isGoogleSource(source.kind) && (
                    <button
                      type="button"
                      disabled={
                        !canValidate ||
                        !source.token_path ||
                        !source.oauth_client_path ||
                        Boolean(job && ['running', 'cancelling'].includes(job.status))
                      }
                      title="Authorize read-only Google access with PKCE"
                      onClick={() => void authorizeSource(source)}
                    >
                      {runningThis && job?.operation === 'authorization' ? (
                        <LoaderCircle className="spin" size={14} />
                      ) : (
                        <KeyRound size={14} />
                      )}
                      Authorize
                    </button>
                  )}
                  <button
                    type="button"
                    disabled={
                      !canValidate ||
                      Boolean(
                        job &&
                        runningThis === false &&
                        ['running', 'cancelling'].includes(job.status)
                      )
                    }
                    title={canValidate ? 'Read-only bounded validation' : 'Save changes first'}
                    onClick={() => void validateSource(source)}
                  >
                    {runningThis && job?.operation === 'validation' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Play size={14} />
                    )}
                    Validate
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove ${source.name}`}
                    onClick={() => {
                      if (
                        window.confirm(
                          `Remove ${source.name} from configuration? Existing indexed data is not deleted.`
                        )
                      ) {
                        update((current) => ({
                          ...current,
                          sources: current.sources.filter((_, position) => position !== index),
                        }))
                      }
                    }}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </header>

              {!source.editable && (
                <div className="source-managed-note">
                  This external command is managed in the TOML file. Desktop can retain, disable, or
                  remove it, but cannot edit or create shell commands.
                </div>
              )}

              <div className="form-grid source-form-grid">
                <Field label="Source name" hint="stable lowercase identifier">
                  <input
                    value={source.name}
                    disabled={!source.editable}
                    required
                    maxLength={64}
                    pattern="[a-z0-9][a-z0-9_-]*"
                    onChange={(event) => changeSource(index, { name: event.target.value })}
                  />
                </Field>
                <Field label="Connector">
                  <select
                    value={source.kind}
                    disabled={!source.editable}
                    onChange={(event) =>
                      changeSource(index, {
                        kind: event.target.value as SourceKind,
                        token_env: defaultTokenEnv(event.target.value as SourceKind),
                      })
                    }
                  >
                    {source.kind === 'external' && (
                      <option value="external">External command</option>
                    )}
                    {SOURCE_KINDS.map((kind) => (
                      <option key={kind.value} value={kind.value}>
                        {kind.label}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Workspace">
                  <select
                    value={source.project}
                    onChange={(event) => changeSource(index, { project: event.target.value })}
                  >
                    {settings.workspaces.map((workspace) => (
                      <option key={workspace.id} value={workspace.id}>
                        {workspace.name}
                      </option>
                    ))}
                  </select>
                </Field>
                {(source.kind === 'filesystem' || source.kind === 'buzz') && (
                  <Field
                    label={source.kind === 'buzz' ? 'Buzz data directory' : 'Root directory'}
                    hint="absolute, non-root path"
                    wide
                  >
                    <div className="path-input">
                      <input
                        value={source.root || ''}
                        disabled={!source.editable}
                        required={source.enabled}
                        placeholder="/Users/you/Documents"
                        onChange={(event) =>
                          changeSource(index, { root: event.target.value || null })
                        }
                      />
                      <button
                        type="button"
                        disabled={!source.editable}
                        aria-label="Choose source directory"
                        onClick={() => void choosePath(index, 'directory', 'root')}
                      >
                        <FolderOpen size={14} />
                      </button>
                    </div>
                  </Field>
                )}
                {source.kind === 'filesystem' && (
                  <>
                    <Field label="Source label" hint="identifier stored on indexed documents">
                      <input
                        value={source.source || ''}
                        disabled={!source.editable}
                        maxLength={128}
                        placeholder={source.name}
                        onChange={(event) =>
                          changeSource(index, { source: event.target.value || null })
                        }
                      />
                    </Field>
                    <Field label="Excluded paths" hint="comma or line separated, relative paths">
                      <input
                        value={source.exclude.join(', ')}
                        disabled={!source.editable}
                        onChange={(event) =>
                          changeSource(index, { exclude: splitList(event.target.value) })
                        }
                      />
                    </Field>
                  </>
                )}
                {isGoogleSource(source.kind) && (
                  <>
                    <Field
                      label="Google OAuth token file"
                      hint="private token created by Cortana after authorization"
                      wide
                    >
                      <div className="path-input">
                        <input
                          value={source.token_path || ''}
                          disabled={!source.editable}
                          required={source.enabled && !source.token_env}
                          placeholder="/Users/you/.config/cortana/google-token.json"
                          onChange={(event) =>
                            changeSource(index, { token_path: event.target.value || null })
                          }
                        />
                        <button
                          type="button"
                          disabled={!source.editable}
                          aria-label="Choose Google token destination"
                          onClick={() => void choosePath(index, 'google-token', 'token_path')}
                        >
                          <FolderOpen size={14} />
                        </button>
                      </div>
                    </Field>
                    <Field
                      label="Google Desktop OAuth client JSON"
                      hint="downloaded from Google Cloud Console; required to authorize"
                      wide
                    >
                      <div className="path-input">
                        <input
                          value={source.oauth_client_path || ''}
                          disabled={!source.editable}
                          placeholder="/Users/you/Downloads/google-oauth-client.json"
                          onChange={(event) =>
                            changeSource(index, {
                              oauth_client_path: event.target.value || null,
                            })
                          }
                        />
                        <button
                          type="button"
                          disabled={!source.editable}
                          aria-label="Choose Google OAuth client JSON"
                          onClick={() =>
                            void choosePath(index, 'oauth-client', 'oauth_client_path')
                          }
                        >
                          <FolderOpen size={14} />
                        </button>
                      </div>
                    </Field>
                    <Field label="Google query" hint="optional provider-native filter" wide>
                      <input
                        value={source.query || ''}
                        disabled={!source.editable}
                        maxLength={2048}
                        placeholder={source.kind === 'gmail' ? 'newer_than:1y' : ''}
                        onChange={(event) =>
                          changeSource(index, { query: event.target.value || null })
                        }
                      />
                    </Field>
                  </>
                )}
                {(source.kind === 'slack' || source.kind === 'discord') && (
                  <>
                    <Field label="Channel IDs" hint="comma or line separated" wide>
                      <input
                        value={source.channels.join(', ')}
                        disabled={!source.editable}
                        required={source.enabled}
                        onChange={(event) =>
                          changeSource(index, { channels: splitList(event.target.value) })
                        }
                      />
                    </Field>
                    <Field
                      label="Token variable"
                      hint={
                        secret?.configured && !clearedSecrets.has(secret.name)
                          ? `Configured via ${secret.source}`
                          : 'stored in Cortana owner-only secret file'
                      }
                    >
                      <input
                        value={source.token_env || ''}
                        disabled={!source.editable}
                        required={source.enabled}
                        pattern="[A-Z_][A-Z0-9_]*"
                        onChange={(event) =>
                          changeSource(index, { token_env: event.target.value || null })
                        }
                      />
                    </Field>
                    <Field label="New token" hint="write-only; leave blank to keep existing">
                      <div className="secret-input">
                        <input
                          type="password"
                          autoComplete="new-password"
                          disabled={!source.editable || !source.token_env}
                          value={source.token_env ? secretValues[source.token_env] || '' : ''}
                          onChange={(event) => {
                            if (source.token_env) {
                              onSecret({ ...secretValues, [source.token_env]: event.target.value })
                            }
                          }}
                        />
                        {source.token_env &&
                          secret?.configured &&
                          !clearedSecrets.has(secret.name) && (
                            <button type="button" onClick={() => onClearSecret(source.token_env!)}>
                              Clear
                            </button>
                          )}
                      </div>
                    </Field>
                  </>
                )}
                {source.editable && (
                  <>
                    <Field label="Document limit" hint="blank uses global budget">
                      <input
                        type="number"
                        min={1}
                        max={1000000}
                        value={source.max_documents ?? ''}
                        onChange={(event) =>
                          changeSource(index, {
                            max_documents: optionalNumber(event.target.value),
                          })
                        }
                      />
                    </Field>
                    <Field label="Content limit (bytes)" hint="blank uses global budget">
                      <input
                        type="number"
                        min={1024}
                        max={1099511627776}
                        value={source.max_bytes ?? ''}
                        onChange={(event) =>
                          changeSource(index, { max_bytes: optionalNumber(event.target.value) })
                        }
                      />
                    </Field>
                  </>
                )}
              </div>
            </article>
          )
        })}
      </div>

      {error && <div className="safety-note">{error}</div>}
      {job && (
        <div className={`source-validation-job ${job.status}`}>
          <div>
            <StatusGlyph
              passed={job.status === 'succeeded'}
              optional={job.status === 'cancelled'}
            />
            <span>
              <strong>
                {job.source} · {job.operation} · {job.status}
              </strong>
              <small>{job.summary}</small>
            </span>
            {['running', 'cancelling'].includes(job.status) && (
              <button
                type="button"
                disabled={job.status === 'cancelling'}
                onClick={() => void cancel()}
              >
                <CircleStop size={14} /> Cancel
              </button>
            )}
            {job.retryable && (
              <button
                type="button"
                disabled={!canValidate}
                onClick={() => {
                  const source = settings.sources.find((item) => item.name === job.source)
                  if (source) {
                    if (job.operation === 'authorization') void authorizeSource(source)
                    else void validateSource(source)
                  }
                }}
              >
                <RefreshCw size={14} /> Retry
              </button>
            )}
          </div>
          {job.log && <pre>{job.log}</pre>}
        </div>
      )}

      <div className="safety-note">
        <AlertTriangle size={16} />
        <span>
          Source validation fetches a deliberately small sample but writes only metadata about the
          outcome. Full ingestion requires a separate reviewed sync action, which is not enabled
          here.
        </span>
      </div>
    </SettingsSection>
  )
}

function newSource(settings: DesktopSettings): SourceSettings {
  const index = settings.sources.length + 1
  return {
    name: `source-${index}`,
    kind: 'filesystem',
    enabled: false,
    project: settings.workspaces[0]?.id || 'personal',
    root: null,
    source: null,
    channels: [],
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
}

function defaultTokenEnv(kind: SourceKind): string | null {
  if (kind === 'slack') return 'SLACK_BOT_TOKEN'
  if (kind === 'discord') return 'DISCORD_BOT_TOKEN'
  return null
}

function isGoogleSource(kind: SourceKind) {
  return ['google-drive', 'gmail', 'google-calendar'].includes(kind)
}

function hasBrowserSetup(kind: SourceKind) {
  return isGoogleSource(kind) || kind === 'slack' || kind === 'discord'
}

function splitList(value: string) {
  return value
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean)
}

function optionalNumber(value: string): number | null {
  return value === '' ? null : Number(value)
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
