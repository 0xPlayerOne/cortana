import {
  AlertTriangle,
  Check,
  CircleStop,
  Download,
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
  Upload,
  X,
  Zap,
} from 'lucide-react'
import { type FormEvent, type ReactNode, useEffect, useRef, useState } from 'react'
import { applyTheme, readThemePreference, SUPPORTED_THEMES, type ThemeMode } from '../theme'
import { WorkspaceLogo } from '../workspaceLogos'
import { readWorkspaceLogoFile, writeWorkspaceLogo } from '../workspaceLogoStore'
import { SourceIcon } from './sourceIcons'
import { sourceDisplayName } from './sourceIconData'

import {
  cancelDesktopInstaller,
  cancelDesktopSourceValidation,
  checkDesktopUpdate,
  exportDesktopSettings,
  getDesktopAudit,
  getDesktopInstaller,
  getDesktopInfo,
  listDesktopDiscordChannels,
  listDesktopDiscordServers,
  listDesktopGithubRepositories,
  listDesktopProviderModels,
  listDesktopSlackWorkspaces,
  listDesktopBuzzCommunities,
  getDesktopHindsightStatus,
  getDesktopHonchoStatus,
  getDesktopSchedule,
  getDesktopServices,
  getDesktopSourceValidation,
  getDesktopSettings,
  getDesktopUpdate,
  getRuntimeAudit,
  installDesktopUpdate,
  installDesktopServices,
  installDesktopSyncService,
  importDesktopSettings,
  migrateDesktopEmbeddingGeneration,
  isDesktopApp,
  openDesktopSourceSetup,
  openDesktopProject,
  openDesktopSecretFile,
  pickDesktopPath,
  planDesktopInitialSync,
  saveDesktopSettings,
  saveDesktopSchedule,
  scanDesktopReadiness,
  setDesktopAutostart,
  startDesktopInitialSync,
  startDesktopInstaller,
  startDesktopSourceAuthorization,
  startDesktopSourceTrialSync,
  startDesktopSourceValidation,
  runDesktopServiceAction,
  runDesktopServicesActionAll,
} from '../api'
import { buildSetupSteps } from '../setup'
import { INITIAL_SYNC_BUDGETS, type ProviderModelKind, type ProviderModelEntry } from '../types'
import { isLoopbackUrl } from '../operations'
import type {
  DesktopInitialSyncPlan,
  DesktopInstallJob,
  DesktopInfo,
  DesktopHindsightStatus,
  DesktopHonchoStatus,
  DesktopReadiness,
  DesktopReadinessActivity,
  DesktopServiceActivity,
  DesktopServiceReport,
  DesktopSchedule,
  DesktopSettings,
  DesktopSettingsUpdate,
  DesktopSourceJob,
  DesktopUpdate,
  AuditEvent,
  AuthPrincipalSettings,
  DiscordGuildChannels,
  DiscordServerSummary,
  GithubRepositorySummary,
  InitialSyncBudget,
  SlackWorkspaceSummary,
  BuzzCommunitySummary,
  SourceKind,
  SourceSettings,
  WorkspaceSettings,
} from '../types'

type Section =
  | 'readiness'
  | 'services'
  | 'updates'
  | 'access'
  | 'audit'
  | 'workspaces'
  | 'sources'
  | 'embedding'
  | 'query'
  | 'hindsight'
  | 'honcho'
  | 'ingestion'
  | 'advanced'

const PLUGIN_SECTIONS: Array<{ key: 'hindsight' | 'honcho'; label: string }> = [
  { key: 'hindsight', label: 'Hindsight' },
  { key: 'honcho', label: 'Honcho' },
]

const SETTINGS_NAV_PRIMARY_SECTIONS: Section[] = ['services', 'workspaces', 'sources', 'readiness']
const SETTINGS_NAV_SECONDARY_SECTIONS: Section[] = [
  'updates',
  'access',
  'audit',
  'embedding',
  'query',
  'ingestion',
  'advanced',
]

function useDesktopForeground(): boolean {
  const [foreground, setForeground] = useState(
    () => typeof document === 'undefined' || document.visibilityState !== 'hidden'
  )

  useEffect(() => {
    const visibility = { current: document.visibilityState !== 'hidden' }
    const focused = { current: true }
    const syncForeground = () => setForeground(visibility.current && focused.current)
    const markVisible = () => {
      visibility.current = document.visibilityState !== 'hidden'
      syncForeground()
    }
    const markFocused = () => {
      focused.current = true
      syncForeground()
    }
    const markBlurred = () => {
      focused.current = false
      syncForeground()
    }

    window.addEventListener('focus', markFocused)
    window.addEventListener('blur', markBlurred)
    document.addEventListener('visibilitychange', markVisible)
    let disposed = false
    let unlistenFocus: (() => void) | undefined
    if (isDesktopApp && '__TAURI_INTERNALS__' in window) {
      void import('@tauri-apps/api/window')
        .then(({ getCurrentWindow }) => {
          const currentWindow = getCurrentWindow()
          void currentWindow
            .isFocused()
            .then((payload) => {
              if (!disposed) {
                focused.current = payload
                syncForeground()
              }
            })
            .catch(() => {
              // Browser focus events remain the fallback when the native
              // startup snapshot is unavailable.
            })
          return currentWindow.onFocusChanged(({ payload }) => {
            if (!disposed) {
              focused.current = payload
              syncForeground()
            }
          })
        })
        .then((unlisten) => {
          if (disposed) unlisten()
          else unlistenFocus = unlisten
        })
        .catch(() => {
          // Browser focus events remain the fallback when the native focus
          // listener cannot be registered during startup.
        })
    }
    return () => {
      disposed = true
      window.removeEventListener('focus', markFocused)
      window.removeEventListener('blur', markBlurred)
      document.removeEventListener('visibilitychange', markVisible)
      unlistenFocus?.()
    }
  }, [])

  return foreground
}

export function SettingsView({
  desktopSettings: externalSettings,
  onLoaded,
  onSaved,
  onDirtyChange,
  initialSection = 'readiness',
  onJob,
  sourceJobs,
  installerJob: externalInstallerJob,
  onInstallerJob,
  readiness: externalReadiness,
  onReadiness,
  readinessActivity,
  onReadinessScan,
  desktopUpdate: externalDesktopUpdate,
  onDesktopUpdate,
  services: externalServices,
  onServices,
  servicesError: externalServicesError,
  onServicesError,
  desktopInfo: externalDesktopInfo,
  onDesktopInfo,
  serviceActivity,
  onServiceActivity,
  hindsightStatus: externalHindsightStatus,
  onHindsightStatus,
  honchoStatus: externalHonchoStatus,
  onHonchoStatus,
}: {
  /** Shell-owned settings snapshot. Standalone renders fetch their own copy. */
  desktopSettings?: DesktopSettings
  /** Report a standalone settings load back to the Desktop shell. */
  onLoaded?: (settings: DesktopSettings) => void
  onSaved: (settings: DesktopSettings) => void
  onDirtyChange?: (dirty: boolean) => void
  initialSection?: Section
  onJob?: (job: DesktopSourceJob) => void
  /**
   * Shared snapshots from the shell-level source-job poller. Standalone
   * renders (for example the web fallback and focused tests) omit this prop
   * and keep the local observer below.
   */
  sourceJobs?: DesktopSourceJob[]
  /**
   * Optional shell-owned installer state. The app shell supplies this so an
   * install remains observable while SettingsView is unmounted. Standalone
   * renders keep the local state below.
   */
  installerJob?: DesktopInstallJob | null
  onInstallerJob?: (job: DesktopInstallJob | null) => void
  /** Optional shell-owned readiness snapshot shared across Settings mounts. */
  readiness?: DesktopReadiness | null
  onReadiness?: (readiness: DesktopReadiness | null) => void
  readinessActivity?: DesktopReadinessActivity | null
  onReadinessScan?: () => Promise<DesktopReadiness>
  /** Optional shell-owned updater snapshot shared across Settings mounts. */
  desktopUpdate?: DesktopUpdate | null
  onDesktopUpdate?: (update: DesktopUpdate) => void
  /** Shell-owned service status shared with the tray/health indicator. */
  services?: DesktopServiceReport | null
  onServices?: (report: DesktopServiceReport) => void
  servicesError?: string
  onServicesError?: (error: string) => void
  desktopInfo?: DesktopInfo | null
  onDesktopInfo?: (info: DesktopInfo) => void
  /** Shell-owned service action status shared across Settings mounts. */
  serviceActivity?: DesktopServiceActivity | null
  onServiceActivity?: (activity: DesktopServiceActivity | null) => void
  /** Shell-owned Hindsight health snapshot shared across Settings mounts. */
  hindsightStatus?: DesktopHindsightStatus | null
  onHindsightStatus?: (status: DesktopHindsightStatus | null) => void
  /** Shell-owned Honcho health snapshot shared across Settings mounts. */
  honchoStatus?: DesktopHonchoStatus | null
  onHonchoStatus?: (status: DesktopHonchoStatus | null) => void
}) {
  const [settings, setSettings] = useState<DesktopSettings | null>(externalSettings ?? null)
  const [section, setSection] = useState<Section>(initialSection)
  const [secretValues, setSecretValues] = useState<Record<string, string>>({})
  const [clearedSecrets, setClearedSecrets] = useState<Set<string>>(new Set())
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [dirty, setDirty] = useState(false)
  const [theme, setTheme] = useState<ThemeMode>(readThemePreference)
  const [localReadiness, setLocalReadiness] = useState<DesktopReadiness | null>(null)
  const setupReadiness = externalReadiness === undefined ? localReadiness : externalReadiness
  const setSetupReadiness = onReadiness ?? setLocalReadiness
  const [localInstallerJob, setLocalInstallerJob] = useState<DesktopInstallJob | null>(null)
  const installerJob = externalInstallerJob === undefined ? localInstallerJob : externalInstallerJob
  const setInstallerJob = onInstallerJob ?? setLocalInstallerJob
  const componentMounted = useRef(true)
  const settingsRef = useRef(settings)

  const [providerModels, setProviderModels] = useState<ProviderModelsState[]>([])
  const [modelsLoading, setModelsLoading] = useState<ProviderModelKind | null>(null)
  const [modelsError, setModelsError] = useState<Record<ProviderModelKind, string>>({
    embedding: '',
    query: '',
  })

  const applyLoadedSettings = (next: DesktopSettings) => {
    setSettings(next)
    onLoaded?.(next)
  }

  /**
   * Fetch the models advertised by the configured provider through the
   * bundled CLI. The catalog is applied only when the provider endpoint,
   * mode, and API key variable are unchanged since the fetch started, so a
   * stale list can never be attached to a different provider.
   */
  const refreshProviderModels = async (kind: ProviderModelKind) => {
    const current = settingsRef.current
    if (!current) return
    const provider = kind === 'embedding' ? current.embedding : current.query
    const captured = {
      provider: normalizeProviderUrl(provider.base_url),
      mode: provider.provider,
      key_env: provider.api_key_env,
    }
    setModelsLoading(kind)
    setModelsError({ embedding: '', query: '' })
    try {
      const list = await listDesktopProviderModels(kind)
      if (!componentMounted.current) return
      const live = settingsRef.current
      if (!live) return
      const liveProvider = kind === 'embedding' ? live.embedding : live.query
      if (
        normalizeProviderUrl(liveProvider.base_url) !== captured.provider ||
        liveProvider.provider !== captured.mode ||
        liveProvider.api_key_env !== captured.key_env
      ) {
        setModelsError((current) => ({
          ...current,
          [kind]: 'Provider endpoint changed while refreshing; run refresh again.',
        }))
        return
      }
      setProviderModels((existing) => [
        ...existing.filter((entry) => entry.kind !== kind),
        {
          kind,
          provider: list.provider,
          mode: captured.mode,
          key_env: captured.key_env,
          models: list.models,
          truncated: list.truncated,
        },
      ])
    } catch (caught) {
      if (!componentMounted.current) return
      setModelsError((current) => ({
        ...current,
        [kind]: caught instanceof Error ? caught.message : 'Unable to refresh provider models',
      }))
    } finally {
      if (componentMounted.current) setModelsLoading(null)
    }
  }

  /** Advertised catalog for a kind, or null when unavailable or stale. */
  const advertisedModelsFor = (kind: ProviderModelKind) => {
    const entry = providerModels.find((candidate) => candidate.kind === kind)
    if (!entry || entry.models.length === 0 || !settings) return null
    const provider = kind === 'embedding' ? settings.embedding : settings.query
    if (
      normalizeProviderUrl(provider.base_url) !== entry.provider ||
      provider.provider !== entry.mode ||
      provider.api_key_env !== entry.key_env
    ) {
      return null
    }
    return entry
  }

  useEffect(() => {
    settingsRef.current = settings
  }, [settings])

  useEffect(() => {
    componentMounted.current = true
    return () => {
      componentMounted.current = false
    }
  }, [])

  useEffect(() => {
    if (!isDesktopApp) return
    if (externalSettings) {
      // The shell owns the saved snapshot. Do not replace an in-progress local
      // draft when a parent status update re-renders this view.
      if (!dirty) setSettings(externalSettings)
      return
    }
    void getDesktopSettings()
      .then(applyLoadedSettings)
      .catch((caught: unknown) =>
        setError(caught instanceof Error ? caught.message : 'Unable to load settings')
      )
  }, [externalSettings, dirty, onLoaded])

  useEffect(() => setSection(initialSection), [initialSection])

  useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])

  // Restarts the affected core services in the background after a save that
  // reports restart_required. Success clears the pending-restart notice;
  // failure keeps it visible with an explicit recovery action instead of
  // leaving the operator guessing which service needs attention.
  const restartServices = (next: DesktopSettings) => {
    onServiceActivity?.({
      target: 'core services',
      action: 'restart',
      status: 'running',
      detail: null,
    })

    void runDesktopServicesActionAll('restart')
      .then(() => {
        if (!componentMounted.current) return
        onServiceActivity?.({
          target: 'core services',
          action: 'restart',
          status: 'succeeded',
          detail: null,
        })
        const cleared = { ...next, restart_required: false }
        setSettings(cleared)
        onSaved(cleared)
        setSaved(false)
      })
      .catch((caught: unknown) => {
        if (!componentMounted.current) return
        onServiceActivity?.({
          target: 'core services',
          action: 'restart',
          status: 'failed',
          detail: caught instanceof Error ? caught.message : 'Core services restart failed',
        })
      })
  }

  const restartAfterSaveIfNeeded = (next: DesktopSettings) => {
    if (!next.restart_required) return
    restartServices(next)
  }

  const update = (change: (draft: DesktopSettings) => DesktopSettings) => {
    setSettings((current) => (current ? change(current) : current))
    setSaved(false)
    setDirty(true)
  }

  const onThemeChange = (next: ThemeMode) => {
    setTheme(next)
    applyTheme(next)
  }

  const retrySettingsLoad = () => {
    setError('')
    void getDesktopSettings()
      .then(applyLoadedSettings)
      .catch((caught: unknown) =>
        setError(caught instanceof Error ? caught.message : 'Unable to load settings')
      )
  }

  const discard = async () => {
    if (!dirty || saving) return
    if (!window.confirm('Discard unsaved Cortana settings changes?')) return
    setSaving(true)
    setError('')
    try {
      const next = await getDesktopSettings()
      applyLoadedSettings(next)
      setSecretValues({})
      setClearedSecrets(new Set())
      setSaved(false)
      setDirty(false)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Unable to discard settings changes')
    } finally {
      setSaving(false)
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    // Enter can submit a form even when the visible Save button is disabled.
    // Avoid creating a no-op settings audit event or touching secrets when
    // there is no draft to persist.
    if (!settings || !dirty || saving) return
    const sourceIdentityError = validateSourceIdentityScopes(settings.sources)
    if (sourceIdentityError) {
      setError(sourceIdentityError)
      return
    }
    setSaving(true)
    setError('')
    try {
      const referencedSecrets = referencedSecretNames(settings)
      const payload: DesktopSettingsUpdate = {
        workspaces: settings.workspaces,
        sources: settings.sources,
        auth_principals: settings.auth_principals,
        embedding: settings.embedding,
        query: settings.query,
        hindsight: settings.hindsight,
        honcho: settings.honcho,
        ingestion: settings.ingestion,
        runtime: settings.runtime,
        secrets: [
          ...Object.entries(secretValues)
            .filter(
              ([name, value]) =>
                referencedSecrets.has(name) && value.length > 0 && !clearedSecrets.has(name)
            )
            .map(([name, value]) => ({ name, value })),
          ...Array.from(clearedSecrets)
            .filter((name) => referencedSecrets.has(name))
            .map((name) => ({ name, clear: true })),
        ],
      }
      const next = await saveDesktopSettings(payload)
      setSettings(next)
      setSecretValues({})
      setClearedSecrets(new Set())
      setSaved(true)
      setDirty(false)
      onSaved(next)
      restartAfterSaveIfNeeded(next)
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
        <h1 role={error ? 'alert' : 'status'}>{error || 'Loading local settings…'}</h1>
        {error && (
          <button type="button" className="secondary-button" onClick={retrySettingsLoad}>
            <RefreshCw size={15} /> Retry settings
          </button>
        )}
      </main>
    )
  }

  const restartFailed = settings.restart_required && serviceActivity?.status === 'failed'

  return (
    <main className="settings-view">
      <header className="settings-header">
        <div>
          <span className="eyebrow">{settings.needs_setup ? 'Guided setup' : 'Control plane'}</span>
          <h1>Settings</h1>
          <p>Changes are written locally and audited. Secret values never return to this window.</p>
        </div>
        <div className="settings-header-actions">
          <label className="settings-theme-control" htmlFor="theme-select">
            <span>Theme</span>
            <select
              id="theme-select"
              value={theme}
              onChange={(event) => onThemeChange(event.target.value as ThemeMode)}
            >
              {SUPPORTED_THEMES.map((item) => (
                <option value={item.id} key={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
          {dirty && (
            <button
              type="button"
              className="secondary-button"
              disabled={saving}
              onClick={() => void discard()}
            >
              <X size={15} /> Discard
            </button>
          )}
          <button
            type="submit"
            className="primary-button"
            form="settings-form"
            disabled={saving || !dirty}
            title={dirty ? undefined : 'Make a change before saving'}
          >
            <Save size={16} /> {saving ? 'Saving…' : 'Save changes'}
          </button>
        </div>
      </header>
      {settings.needs_setup && (
        <SetupGuide
          settings={settings}
          readiness={setupReadiness}
          dirty={dirty}
          onOpen={setSection}
        />
      )}
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {SETTINGS_NAV_PRIMARY_SECTIONS.map((item) => (
            <button
              type="button"
              key={item}
              className={`settings-nav-item ${section === item ? 'active' : ''}`}
              onClick={() => setSection(item)}
            >
              {item[0].toUpperCase() + item.slice(1)}
            </button>
          ))}
          <div className="settings-nav-divider" aria-hidden="true" />
          {SETTINGS_NAV_SECONDARY_SECTIONS.map((item) => (
            <button
              type="button"
              key={item}
              className={`settings-nav-item ${section === item ? 'active' : ''}`}
              onClick={() => setSection(item)}
            >
              {item[0].toUpperCase() + item.slice(1)}
            </button>
          ))}
          <div className="settings-nav-group">
            <button
              type="button"
              className={`settings-nav-item ${
                section === 'hindsight' || section === 'honcho' ? 'active' : ''
              }`}
              onClick={() => setSection('hindsight')}
            >
              Plugins
            </button>
            <div className="settings-nav-subgroup" role="group" aria-label="Plugins">
              {PLUGIN_SECTIONS.map((plugin) => (
                <button
                  type="button"
                  key={plugin.key}
                  className={`settings-nav-item settings-nav-item--sub ${
                    section === plugin.key ? 'active' : ''
                  }`}
                  onClick={() => setSection(plugin.key)}
                >
                  {plugin.label}
                </button>
              ))}
            </div>
          </div>
          <div className="settings-paths">
            <span>Config</span>
            <code title={settings.config_path}>{settings.config_path}</code>
          </div>
        </nav>
        <form id="settings-form" className="settings-form" onSubmit={submit}>
          {section === 'readiness' && (
            <ReadinessSection
              autoScan={settings.needs_setup}
              readiness={setupReadiness}
              onResult={setSetupReadiness}
              onOpenServices={() => setSection('services')}
              job={installerJob}
              onJob={setInstallerJob}
              readinessActivity={readinessActivity}
              onReadinessScan={onReadinessScan}
              pollInstaller={externalInstallerJob === undefined}
            />
          )}
          {section === 'services' && (
            <ServicesSection
              settings={settings}
              dirty={dirty}
              services={externalServices}
              onServices={onServices}
              servicesError={externalServicesError}
              onServicesError={onServicesError}
              desktopInfo={externalDesktopInfo}
              onDesktopInfo={onDesktopInfo}
              serviceActivity={serviceActivity}
              onServiceActivity={onServiceActivity}
              onRestarted={() =>
                setSettings((current) =>
                  current ? { ...current, restart_required: false } : current
                )
              }
            />
          )}
          {section === 'updates' && (
            <UpdatesSection
              desktopUpdate={externalDesktopUpdate}
              onDesktopUpdate={onDesktopUpdate}
            />
          )}
          {section === 'access' && (
            <AccessSection
              settings={settings}
              update={update}
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
          {section === 'audit' && <AuditSection />}
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
              onJob={onJob}
              sourceJobs={sourceJobs}
            />
          )}
          {section === 'embedding' && (
            <EmbeddingSection
              settings={settings}
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
              update={update}
              advertisedModels={
                advertisedModelsFor('embedding')?.models.map((model) => ({
                  value: model.id,
                  label: model.id,
                })) ?? null
              }
              modelsLoading={modelsLoading === 'embedding'}
              modelsError={modelsError.embedding}
              modelsTruncated={advertisedModelsFor('embedding')?.truncated ?? false}
              onRefreshModels={() => void refreshProviderModels('embedding')}
            />
          )}
          {section === 'query' && (
            <QuerySection
              settings={settings}
              secrets={settings.secrets}
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
              update={update}
              advertisedModels={
                advertisedModelsFor('query')?.models.map((model) => ({
                  value: model.id,
                  label: model.id,
                })) ?? null
              }
              modelsLoading={modelsLoading === 'query'}
              modelsError={modelsError.query}
              modelsTruncated={advertisedModelsFor('query')?.truncated ?? false}
              onRefreshModels={() => void refreshProviderModels('query')}
            />
          )}
          {section === 'hindsight' && (
            <HindsightSection
              settings={settings}
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
              update={update}
              hindsightStatus={externalHindsightStatus}
              onHindsightStatus={onHindsightStatus}
            />
          )}
          {section === 'honcho' && (
            <HonchoSection
              settings={settings}
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
              update={update}
              honchoStatus={externalHonchoStatus}
              onHonchoStatus={onHonchoStatus}
            />
          )}
          {section === 'ingestion' && <IngestionSection settings={settings} update={update} />}
          {section === 'advanced' && (
            <AdvancedSection settings={settings} update={update} dirty={dirty} />
          )}
        </form>
      </div>
      {(error || saved || settings.restart_required) && (
        <div
          className={`settings-banner ${error || restartFailed ? 'error' : ''}`}
          role={error || restartFailed ? 'alert' : 'status'}
        >
          {error || restartFailed ? <AlertTriangle size={16} /> : <Check size={16} />}
          {error ||
            (restartFailed
              ? `Settings saved, but the service restart failed${
                  serviceActivity?.detail ? `: ${serviceActivity.detail}` : '.'
                }`
              : settings.restart_required && serviceActivity?.status === 'running'
                ? 'Settings saved. Restarting affected services in the background…'
                : saved && settings.restart_required
                  ? 'Settings saved. A service restart is still required.'
                  : saved
                    ? 'Settings saved.'
                    : 'A service restart is still required.')}
          {!error && settings.restart_required && serviceActivity?.status !== 'running' && (
            <>
              {restartFailed && (
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => restartServices(settings)}
                >
                  <RefreshCw size={14} /> Retry restart
                </button>
              )}
              <button
                type="button"
                className="secondary-button"
                onClick={() => setSection('services')}
              >
                Open services
              </button>
            </>
          )}
        </div>
      )}
    </main>
  )
}

function SetupGuide({
  settings,
  readiness,
  dirty,
  onOpen,
}: {
  settings: DesktopSettings
  readiness: DesktopReadiness | null
  dirty: boolean
  onOpen: (section: Section) => void
}) {
  const steps = buildSetupSteps(settings, readiness)
  const complete = steps.filter((step) => step.complete).length
  return (
    <section className="setup-guide" aria-label="Guided setup progress">
      <div className="setup-guide-heading">
        <div>
          <span className="eyebrow">First launch</span>
          <strong>Set up Cortana safely</strong>
          <p>
            Review each step, then save. The guide itself never starts ingestion; recurring sync is
            a separate validation-gated action in Services.
          </p>
        </div>
        <span>
          {complete} of {steps.length} ready
        </span>
      </div>
      <div className="setup-steps">
        {steps.map((step, index) => (
          <button
            type="button"
            key={step.section}
            className={step.complete ? 'complete' : ''}
            onClick={() => onOpen(step.section)}
          >
            <i>{step.complete ? <Check size={13} /> : index + 1}</i>
            <span>
              <strong>{step.label}</strong>
              <small>{step.detail}</small>
            </span>
          </button>
        ))}
      </div>
      <p className="setup-save-state">
        {dirty
          ? 'Unsaved setup changes are ready for review.'
          : 'The Save changes button creates an owner-only configuration with a rollback copy.'}
      </p>
    </section>
  )
}

function ServicesSection({
  settings,
  dirty,
  services: externalServices,
  onServices,
  servicesError: externalServicesError,
  onServicesError,
  desktopInfo: externalDesktopInfo,
  onDesktopInfo,
  serviceActivity,
  onServiceActivity,
  onRestarted,
}: {
  settings: DesktopSettings
  dirty: boolean
  services?: DesktopServiceReport | null
  onServices?: (report: DesktopServiceReport) => void
  servicesError?: string
  onServicesError?: (error: string) => void
  desktopInfo?: DesktopInfo | null
  onDesktopInfo?: (info: DesktopInfo) => void
  serviceActivity?: DesktopServiceActivity | null
  onServiceActivity?: (activity: DesktopServiceActivity | null) => void
  onRestarted?: () => void
}) {
  const foreground = useDesktopForeground()
  const [localReport, setLocalReport] = useState<DesktopServiceReport | null>(null)
  const report = externalServices === undefined ? localReport : externalServices
  const setReport = onServices ?? setLocalReport
  const [localInfo, setLocalInfo] = useState<DesktopInfo | null>(null)
  const info = externalDesktopInfo === undefined ? localInfo : externalDesktopInfo
  const setInfo = onDesktopInfo ?? setLocalInfo
  const [busy, setBusy] = useState('')
  const [localError, setLocalError] = useState('')
  const [schedule, setSchedule] = useState<DesktopSchedule | null>(null)
  const [scheduleDraft, setScheduleDraft] = useState<DesktopSchedule | null>(null)
  const [scheduleError, setScheduleError] = useState('')
  const [scheduleSaving, setScheduleSaving] = useState(false)
  const [scheduleApplyPending, setScheduleApplyPending] = useState(false)
  const error = localError || externalServicesError || ''
  const refreshInFlightRef = useRef(false)
  const actionInFlightRef = useRef(false)
  const mountedRef = useRef(true)
  const servicesRequestRef = useRef(0)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const isFreshServicesRequest = (requestId: number) => {
    return mountedRef.current && requestId === servicesRequestRef.current
  }

  // Desktop shells own service status errors. When a parent shell refresh
  // succeeds after a previous section-local failure, clear stale local messages
  // so the user-visible banner is driven by the latest snapshot.
  useEffect(() => {
    if (externalServicesError !== undefined && externalServicesError.length === 0) {
      setLocalError('')
    }
  }, [externalServicesError])

  const refresh = async () => {
    if (refreshInFlightRef.current || actionInFlightRef.current) return
    refreshInFlightRef.current = true
    const requestId = ++servicesRequestRef.current
    setLocalError('')
    try {
      const [nextReport, nextInfo] = await Promise.all([getDesktopServices(), getDesktopInfo()])
      if (!isFreshServicesRequest(requestId)) return
      setReport(nextReport)
      setInfo(nextInfo)
      onServicesError?.('')
    } catch (caught) {
      if (!isFreshServicesRequest(requestId)) return
      const message =
        caught instanceof Error ? caught.message : 'Service status could not be loaded'
      setLocalError(message)
      onServicesError?.(message)
    } finally {
      if (servicesRequestRef.current === requestId) refreshInFlightRef.current = false
    }
  }

  useEffect(() => {
    if (externalServices !== undefined || !foreground) return
    void refresh()
    const timer = window.setInterval(() => void refresh(), 15_000)
    return () => {
      window.clearInterval(timer)
      servicesRequestRef.current += 1
      refreshInFlightRef.current = false
    }
  }, [externalServices, foreground])

  useEffect(() => {
    let active = true
    void getDesktopSchedule()
      .then((next) => {
        if (!active) return
        setSchedule(next)
        setScheduleDraft(next)
        setScheduleError('')
      })
      .catch((caught) => {
        if (!active) return
        setScheduleError(caught instanceof Error ? caught.message : 'Schedule could not be loaded')
      })
    return () => {
      active = false
    }
  }, [])

  const saveSchedule = async () => {
    if (!scheduleDraft || scheduleSaving) return
    setScheduleSaving(true)
    setScheduleError('')
    try {
      const next = await saveDesktopSchedule(scheduleDraft)
      if (!mountedRef.current) return
      setSchedule(next)
      setScheduleDraft(next)
      if (report?.services.some((service) => service.name === 'sync' && service.installed)) {
        setScheduleApplyPending(true)
      }
    } catch (caught) {
      setScheduleError(caught instanceof Error ? caught.message : 'Schedule could not be saved')
    } finally {
      setScheduleSaving(false)
    }
  }

  const serviceAction = async (
    service: DesktopServiceReport['services'][number],
    action: 'start' | 'stop' | 'restart'
  ) => {
    const warning =
      service.name === 'sync'
        ? '\n\nThis controls only an already installed recurring sync job. Cortana will not install one automatically.'
        : ''
    if (!window.confirm(`${action} ${service.label}?${warning}`)) return
    setBusy(`${service.name}:${action}`)
    actionInFlightRef.current = true
    refreshInFlightRef.current = false
    servicesRequestRef.current += 1
    setLocalError('')
    onServiceActivity?.({
      target: service.name,
      action,
      status: 'running',
      detail: null,
    })
    try {
      const next = await runDesktopServiceAction(service.name, action)
      if (mountedRef.current || onServices) {
        setReport(next)
        onServicesError?.('')
      }
      onServiceActivity?.({
        target: service.name,
        action,
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : 'Service action failed'
      if (mountedRef.current) setLocalError(message)
      onServiceActivity?.({
        target: service.name,
        action,
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      if (mountedRef.current) setBusy('')
    }
  }

  const toggleAutostart = async (enabled: boolean) => {
    setBusy('autostart')
    actionInFlightRef.current = true
    refreshInFlightRef.current = false
    servicesRequestRef.current += 1
    setLocalError('')
    try {
      const next = await setDesktopAutostart(enabled)
      if (mountedRef.current) setInfo(next)
    } catch (caught) {
      setLocalError(
        caught instanceof Error ? caught.message : 'Desktop autostart could not be changed'
      )
    } finally {
      actionInFlightRef.current = false
      setBusy('')
    }
  }

  const groupAction = async (action: 'start' | 'stop' | 'restart') => {
    const coreServices =
      settings.embedding.provider === 'local' ? 'the server and embedding services' : 'the server'
    if (
      !window.confirm(
        `${action} ${coreServices}?\n\nRecurring sync and backup are explicitly excluded.`
      )
    ) {
      return
    }
    setBusy(`all:${action}`)
    actionInFlightRef.current = true
    refreshInFlightRef.current = false
    servicesRequestRef.current += 1
    setLocalError('')
    onServiceActivity?.({ target: 'core services', action, status: 'running', detail: null })
    try {
      const next = await runDesktopServicesActionAll(action)
      if (mountedRef.current || onServices) {
        setReport(next)
        onServicesError?.('')
      }
      onServiceActivity?.({ target: 'core services', action, status: 'succeeded', detail: null })
      if (mountedRef.current && action === 'restart') onRestarted?.()
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : 'Whole-app service action failed'
      if (mountedRef.current) setLocalError(message)
      onServiceActivity?.({
        target: 'core services',
        action,
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      if (mountedRef.current) setBusy('')
    }
  }

  const install = async () => {
    if (
      !window.confirm(
        'Install Cortana background services for this user?\n\nThis installs the API, local embedding (when configured), and verified backup jobs. It does not install or enable recurring ingestion.'
      )
    ) {
      return
    }
    setBusy('install')
    actionInFlightRef.current = true
    refreshInFlightRef.current = false
    servicesRequestRef.current += 1
    setLocalError('')
    onServiceActivity?.({
      target: 'core services',
      action: 'install',
      status: 'running',
      detail: null,
    })
    try {
      const next = await installDesktopServices()
      if (mountedRef.current || onServices) {
        setReport(next)
        onServicesError?.('')
      }
      onServiceActivity?.({
        target: 'core services',
        action: 'install',
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message =
        caught instanceof Error ? caught.message : 'Cortana services could not be installed'
      if (mountedRef.current) setLocalError(message)
      onServiceActivity?.({
        target: 'core services',
        action: 'install',
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      if (mountedRef.current) setBusy('')
    }
  }

  const installSync = async () => {
    if (dirty) {
      setLocalError(
        'Save changes before enabling recurring sync so the validated configuration is current.'
      )
      return
    }
    if (!schedule || !scheduleDraft) {
      setLocalError('Load the service schedule before enabling recurring sync.')
      return
    }
    if (
      scheduleDraft.sync_interval_seconds !== schedule.sync_interval_seconds ||
      scheduleDraft.backup_interval_seconds !== schedule.backup_interval_seconds
    ) {
      setLocalError('Save the service schedule before enabling recurring sync.')
      return
    }
    const applyingExistingSchedule =
      scheduleApplyPending &&
      !(report?.services.some((service) => service.name === 'sync' && !service.installed) ?? false)
    const actionLabel = applyingExistingSchedule
      ? 'Apply the updated recurring sync schedule'
      : 'Enable recurring source sync'
    if (
      !window.confirm(
        `${actionLabel} for this user?\n\nCortana will re-check that every enabled source has a current successful validation covering its configured safety budgets before installing the schedule. The first run is delayed by the platform scheduler; existing indexed data is not deleted.`
      )
    ) {
      return
    }
    setBusy('sync-install')
    actionInFlightRef.current = true
    refreshInFlightRef.current = false
    servicesRequestRef.current += 1
    setLocalError('')
    onServiceActivity?.({
      target: 'recurring sync',
      action: 'install',
      status: 'running',
      detail: null,
    })
    try {
      const next = await installDesktopSyncService()
      if (mountedRef.current || onServices) {
        setReport(next)
        if (mountedRef.current) setScheduleApplyPending(false)
        onServicesError?.('')
      }
      onServiceActivity?.({
        target: 'recurring sync',
        action: 'install',
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message =
        caught instanceof Error ? caught.message : 'Recurring sync could not be installed'
      if (mountedRef.current) setLocalError(message)
      onServiceActivity?.({
        target: 'recurring sync',
        action: 'install',
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      if (mountedRef.current) setBusy('')
    }
  }

  const needsCoreInstall =
    report?.supported === true &&
    report.services.some(
      (service) =>
        service.name !== 'sync' &&
        (service.name !== 'embedding' || settings.embedding.provider === 'local') &&
        !service.installed
    )
  const needsSyncInstall =
    report?.supported === true &&
    report.services.some((service) => service.name === 'sync' && !service.installed)
  const syncScheduleNeedsApply = needsSyncInstall || scheduleApplyPending
  const actionInFlight = Boolean(busy) || serviceActivity?.status === 'running'
  const actionMessage = serviceActivity
    ? `${serviceActivity.action === 'install' ? 'Install' : serviceActivity.action[0].toUpperCase() + serviceActivity.action.slice(1)} ${serviceActivity.target}${serviceActivity.status === 'running' ? ' in progress…' : serviceActivity.status === 'succeeded' ? ' completed.' : ` failed: ${serviceActivity.detail || 'unknown error'}`}`
    : ''

  return (
    <SettingsSection
      title="Services"
      description="Inspect and control Cortana runtime services. Recurring ingestion stays absent until its dedicated, validation-gated action is confirmed."
    >
      <div className="service-autostart">
        <label className="source-enable">
          <input
            type="checkbox"
            checked={info?.autostart_enabled || false}
            disabled={!info || busy === 'autostart' || actionInFlight}
            onChange={(event) => void toggleAutostart(event.target.checked)}
          />
          <span>
            <strong>Open Cortana Desktop at login</strong>
            <small>
              The window may be closed while the tray and runtime continue independently.
            </small>
          </span>
        </label>
      </div>
      <div className="source-settings-toolbar">
        <span>
          {report?.supported
            ? `${report.services.filter((service) => service.loaded).length} loaded`
            : report
              ? `Runtime service control is not supported on ${report.platform}`
              : 'Checking services…'}
        </span>
        <div className="service-actions">
          <button
            type="button"
            disabled={actionInFlight || report?.supported !== true}
            onClick={() => void groupAction('start')}
          >
            <Play size={14} /> Start all
          </button>
          <button
            type="button"
            disabled={actionInFlight || report?.supported !== true}
            onClick={() => void groupAction('stop')}
          >
            <CircleStop size={14} /> Stop all
          </button>
          <button
            type="button"
            disabled={actionInFlight || report?.supported !== true}
            onClick={() => void groupAction('restart')}
          >
            <RefreshCw size={14} /> Restart all
          </button>
          <button
            type="button"
            className="secondary-button"
            disabled={actionInFlight}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} /> Refresh
          </button>
          {needsCoreInstall && (
            <button
              type="button"
              className="primary-button"
              disabled={actionInFlight}
              onClick={() => void install()}
            >
              {busy === 'install' ? (
                <LoaderCircle className="spin" size={14} />
              ) : (
                <Download size={14} />
              )}{' '}
              Install core services
            </button>
          )}
          {syncScheduleNeedsApply && (
            <button
              type="button"
              className="secondary-button"
              disabled={actionInFlight || report?.supported !== true}
              onClick={() => void installSync()}
            >
              {busy === 'sync-install' ? (
                <LoaderCircle className="spin" size={14} />
              ) : (
                <Download size={14} />
              )}{' '}
              {scheduleApplyPending && !needsSyncInstall
                ? 'Apply recurring sync schedule'
                : 'Enable recurring sync'}
            </button>
          )}
        </div>
      </div>
      {(error || scheduleError || actionMessage) && (
        <div className={`safety-note ${serviceActivity?.status === 'failed' ? 'error' : ''}`}>
          {error || scheduleError || actionMessage}
        </div>
      )}
      {scheduleDraft && (
        <div className="service-schedule">
          <div>
            <strong>Background schedule</strong>
            <p>
              These intervals apply only when you explicitly install recurring sync. Saving them
              never starts a service.
            </p>
          </div>
          <div className="form-grid compact">
            <NumberField
              label="Sync interval (seconds)"
              hint="1 minute to 7 days"
              value={scheduleDraft.sync_interval_seconds}
              min={60}
              max={604800}
              onChange={(sync_interval_seconds) =>
                setScheduleDraft((current) =>
                  current ? { ...current, sync_interval_seconds } : current
                )
              }
            />
            <NumberField
              label="Backup interval (seconds)"
              hint="5 minutes to 30 days"
              value={scheduleDraft.backup_interval_seconds}
              min={300}
              max={2592000}
              onChange={(backup_interval_seconds) =>
                setScheduleDraft((current) =>
                  current ? { ...current, backup_interval_seconds } : current
                )
              }
            />
          </div>
          <div className="service-actions">
            <button
              type="button"
              className="secondary-button"
              disabled={
                actionInFlight ||
                scheduleSaving ||
                !schedule ||
                (scheduleDraft.sync_interval_seconds === schedule.sync_interval_seconds &&
                  scheduleDraft.backup_interval_seconds === schedule.backup_interval_seconds)
              }
              onClick={() => void saveSchedule()}
            >
              {scheduleSaving ? <LoaderCircle className="spin" size={14} /> : <Save size={14} />}{' '}
              Save schedule
            </button>
          </div>
        </div>
      )}
      <div className="service-grid">
        {report?.services.map((service) => {
          const running = service.loaded && service.state === 'running'
          const failed = service.last_exit_status !== null && service.last_exit_status !== 0
          return (
            <article className="service-card" key={service.name}>
              <header>
                <i className={`service-state ${running ? 'ready' : failed ? 'failed' : ''}`} />
                <div>
                  <strong>{service.name[0].toUpperCase() + service.name.slice(1)}</strong>
                  <small>{service.label}</small>
                </div>
              </header>
              <p>
                {!service.installed
                  ? 'Not installed'
                  : service.loaded
                    ? service.state || 'Loaded'
                    : 'Installed, not loaded'}
                {service.pid ? ` · PID ${service.pid}` : ''}
                {failed ? ` · last exit ${service.last_exit_status}` : ''}
              </p>
              <div className="service-actions">
                <button
                  type="button"
                  disabled={!report.supported || !service.installed || running || actionInFlight}
                  onClick={() => void serviceAction(service, 'start')}
                >
                  <Play size={14} /> Start
                </button>
                <button
                  type="button"
                  disabled={!report.supported || !service.loaded || actionInFlight}
                  onClick={() => void serviceAction(service, 'stop')}
                >
                  <CircleStop size={14} /> Stop
                </button>
                <button
                  type="button"
                  disabled={!report.supported || !service.installed || actionInFlight}
                  onClick={() => void serviceAction(service, 'restart')}
                >
                  <RefreshCw size={14} /> Restart
                </button>
              </div>
            </article>
          )
        })}
      </div>
      <p className="settings-note">
        Recurring sync is opt-in and requires current source validation before installation.
        Starting the server, embedding, or backup service does not run ingestion.
      </p>
    </SettingsSection>
  )
}

function UpdatesSection({
  desktopUpdate: externalDesktopUpdate,
  onDesktopUpdate,
}: {
  desktopUpdate?: DesktopUpdate | null
  onDesktopUpdate?: (update: DesktopUpdate) => void
}) {
  const foreground = useDesktopForeground()
  const [localUpdate, setLocalUpdate] = useState<DesktopUpdate | null>(null)
  const update = externalDesktopUpdate === undefined ? localUpdate : externalDesktopUpdate
  const setUpdate = onDesktopUpdate ?? setLocalUpdate
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    if ((externalDesktopUpdate !== undefined && externalDesktopUpdate !== null) || !foreground) {
      return
    }
    void getDesktopUpdate()
      .then((next) => {
        setUpdate(next)
        if (!next.error) setError('')
      })
      .catch((caught: unknown) => {
        setError(caught instanceof Error ? caught.message : 'Updater status unavailable')
      })
  }, [externalDesktopUpdate, foreground, setUpdate])

  useEffect(() => {
    if (externalDesktopUpdate !== undefined || busy !== 'install' || !foreground) return
    let requestInFlight = false
    const poll = () => {
      if (requestInFlight) return
      requestInFlight = true
      void getDesktopUpdate()
        .then((next) => {
          setUpdate(next)
          if (!next.error) setError('')
        })
        .catch((caught: unknown) => {
          setError(caught instanceof Error ? caught.message : 'Updater status unavailable')
        })
        .finally(() => {
          requestInFlight = false
        })
    }
    const timer = window.setInterval(poll, 400)
    return () => window.clearInterval(timer)
  }, [busy, externalDesktopUpdate, foreground])

  const check = async () => {
    setBusy('check')
    setError('')
    try {
      setUpdate(await checkDesktopUpdate())
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Update check failed')
      try {
        setUpdate(await getDesktopUpdate())
      } catch {
        // Keep the existing snapshot when both the check and status fallback
        // are unavailable; the visible error already explains the failure.
      }
    } finally {
      setBusy('')
    }
  }

  const install = async () => {
    if (!update?.available_version) return
    if (
      !window.confirm(
        `Install signed Cortana ${update.available_version} and restart the Desktop app?\n\nThe native updater will verify the release signature before installation.`
      )
    ) {
      return
    }
    setBusy('install')
    setError('')
    setUpdate({
      ...update,
      phase: 'downloading',
      downloaded_bytes: 0,
      total_bytes: null,
      error: null,
    })
    try {
      setUpdate(await installDesktopUpdate(update.available_version, true))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Update installation failed')
      try {
        setUpdate(await getDesktopUpdate())
      } catch {
        // Keep the last known update state when the updater is unreachable.
      }
    } finally {
      setBusy('')
    }
  }

  const openProject = async () => {
    setError('')
    try {
      await openDesktopProject()
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Unable to open the Cortana project page')
    }
  }

  const percent =
    update?.total_bytes && update.total_bytes > 0
      ? Math.min(100, Math.round((update.downloaded_bytes / update.total_bytes) * 100))
      : null
  const updateInFlight =
    busy === 'install' || update?.phase === 'downloading' || update?.phase === 'installing'
  const canInstall =
    Boolean(update?.available_version) && !update?.restart_required && update?.phase !== 'installed'

  return (
    <SettingsSection
      title="Updates"
      description="Cortana checks the fixed GitHub release feed and verifies signed Tauri artifacts in the native process before installation."
    >
      <div className="update-card">
        <div>
          <span className="eyebrow">Installed version</span>
          <strong>{update?.current_version || 'Checking…'}</strong>
          <small>
            {update?.available_version
              ? `Version ${update.available_version} is available`
              : update?.phase === 'current'
                ? 'You are up to date'
                : `Updater status: ${update?.phase || 'idle'}`}
          </small>
        </div>
        <div className="service-actions">
          <button
            type="button"
            disabled={Boolean(busy) || updateInFlight}
            onClick={() => void check()}
          >
            {busy === 'check' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <RefreshCw size={14} />
            )}
            Check now
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!canInstall || Boolean(busy) || updateInFlight}
            onClick={() => void install()}
          >
            {updateInFlight ? <LoaderCircle className="spin" size={14} /> : <Play size={14} />}
            {update?.restart_required || update?.phase === 'installed'
              ? 'Restart required'
              : 'Install and restart'}
          </button>
        </div>
      </div>
      {percent !== null && (
        <div className="update-progress" role="progressbar" aria-valuenow={percent}>
          <i style={{ width: `${percent}%` }} />
          <span>{percent}% downloaded</span>
        </div>
      )}
      {(error || update?.error) && (
        <div className="safety-note" role="alert">
          <AlertTriangle size={16} /> <span>{error || update?.error}</span>
        </div>
      )}
      {update?.release_notes && (
        <div className="release-notes">
          <h3>Version {update.available_version}</h3>
          <SafeMarkdown text={update.release_notes} />
        </div>
      )}
      <div className="release-notes">
        <h3>Installed changelog</h3>
        <SafeMarkdown text={update?.changelog || 'Loading changelog…'} />
      </div>
      {update && (
        <button type="button" className="link-button" onClick={() => void openProject()}>
          View Cortana source on GitHub <ExternalLink size={13} />
        </button>
      )}
    </SettingsSection>
  )
}

function HindsightSection({
  settings,
  update,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  hindsightStatus: externalStatus,
  onHindsightStatus,
}: SettingsSectionProps & {
  settings: DesktopSettings
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  hindsightStatus?: DesktopHindsightStatus | null
  onHindsightStatus?: (status: DesktopHindsightStatus | null) => void
}) {
  const [localStatus, setLocalStatus] = useState<DesktopHindsightStatus | null>(null)
  const status = externalStatus === undefined ? localStatus : externalStatus
  const setStatus = onHindsightStatus ?? setLocalStatus
  const [checking, setChecking] = useState(false)
  const setHindsight = (hindsight: DesktopSettings['hindsight']) => {
    setStatus(null)
    update((current) => ({ ...current, hindsight }))
  }
  const statusSource = settings.hindsight.token_env
    ? settings.secrets.find((item) => item.name === settings.hindsight.token_env)
    : undefined

  const checkStatus = async () => {
    setChecking(true)
    try {
      setStatus(await getDesktopHindsightStatus())
    } catch (caught) {
      setStatus({
        enabled: settings.hindsight.enabled,
        configured: false,
        reachable: false,
        state: 'unreachable',
        endpoint: settings.hindsight.base_url,
        bank: settings.hindsight.bank,
        token_configured: false,
        detail: caught instanceof Error ? caught.message : 'Hindsight status check failed',
      })
    } finally {
      setChecking(false)
    }
  }

  return (
    <SettingsSection
      title="Hindsight memory sidecar"
      description="Optional connector for outbox-based memory export. It is intentionally not wired into normal ingestion by default."
    >
      <div className="form-grid">
        <label className="form-field">
          <span>Adapter status</span>
          <input type="text" value="Optional sidecar" disabled />
          <small>This adapter is opt-in only.</small>
        </label>
        <label className="form-field">
          <span>Ingestion integration</span>
          <input
            type="text"
            value={settings.hindsight.wired_to_ingestion ? 'Enabled' : 'Disabled'}
            disabled
          />
          <small>Normal source sync flow remains unchanged.</small>
        </label>
        <label className="form-field">
          <span>Enabled</span>
          <input
            type="checkbox"
            checked={settings.hindsight.enabled}
            onChange={(event) =>
              setHindsight({ ...settings.hindsight, enabled: event.target.checked })
            }
          />
        </label>
      </div>
      <div className="safety-note" role="status">
        <span>
          Health: {status?.state.replace('_', ' ') || 'not checked'}
          {status?.detail ? ` — ${status.detail}` : ''}
        </span>
        <button
          type="button"
          className="secondary-button"
          onClick={() => void checkStatus()}
          disabled={checking}
        >
          <RefreshCw size={14} /> {checking ? 'Checking…' : 'Check connection'}
        </button>
      </div>
      {externalStatus && (
        <p className="settings-note">
          This health snapshot is retained while you move between Desktop settings sections. It
          reads the last saved Hindsight configuration; save changes before checking again.
        </p>
      )}
      <Field label="Provider" hint="Hindsight currently supports only this provider.">
        <select
          value={settings.hindsight.provider}
          onChange={(event) =>
            setHindsight({
              ...settings.hindsight,
              provider: event.target.value as 'hindsight',
            })
          }
        >
          <option value="hindsight">hindsight</option>
        </select>
      </Field>
      <div className="form-grid">
        <Field label="Endpoint" wide>
          <input
            type="url"
            value={settings.hindsight.base_url}
            onChange={(event) =>
              setHindsight({ ...settings.hindsight, base_url: event.target.value })
            }
            required
          />
        </Field>
        <Field label="Bank">
          <input
            value={settings.hindsight.bank}
            onChange={(event) => setHindsight({ ...settings.hindsight, bank: event.target.value })}
            required
            maxLength={64}
          />
        </Field>
        <Field label="Token environment variable">
          <input
            value={settings.hindsight.token_env || ''}
            onChange={(event) =>
              setHindsight({ ...settings.hindsight, token_env: event.target.value || null })
            }
            pattern="[A-Z_][A-Z0-9_]*"
            placeholder="CORTANA_HINDSIGHT_TOKEN"
          />
        </Field>
        <Field label="New token" hint="write-only; leave blank to retain">
          <div className="secret-input">
            <input
              type="password"
              autoComplete="new-password"
              value={
                settings.hindsight.token_env ? secretValues[settings.hindsight.token_env] || '' : ''
              }
              disabled={!settings.hindsight.token_env}
              onChange={(event) => {
                if (!settings.hindsight.token_env) return
                setStatus(null)
                onSecret({ ...secretValues, [settings.hindsight.token_env]: event.target.value })
              }}
            />
            {settings.hindsight.token_env &&
              statusSource?.configured &&
              !clearedSecrets.has(statusSource.name) && (
                <button
                  type="button"
                  onClick={() => {
                    setStatus(null)
                    onClearSecret(settings.hindsight.token_env!)
                  }}
                >
                  Clear
                </button>
              )}
          </div>
        </Field>
      </div>
    </SettingsSection>
  )
}

function HonchoSection({
  settings,
  update,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  honchoStatus: externalStatus,
  onHonchoStatus,
}: SettingsSectionProps & {
  settings: DesktopSettings
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  honchoStatus?: DesktopHonchoStatus | null
  onHonchoStatus?: (status: DesktopHonchoStatus | null) => void
}) {
  const [localStatus, setLocalStatus] = useState<DesktopHonchoStatus | null>(null)
  const status = externalStatus === undefined ? localStatus : externalStatus
  const setStatus = onHonchoStatus ?? setLocalStatus
  const [checking, setChecking] = useState(false)
  const setHoncho = (honcho: DesktopSettings['honcho']) => {
    setStatus(null)
    update((current) => ({ ...current, honcho }))
  }
  const statusSource = settings.honcho.token_env
    ? settings.secrets.find((item) => item.name === settings.honcho.token_env)
    : undefined

  const checkStatus = async () => {
    setChecking(true)
    try {
      setStatus(await getDesktopHonchoStatus())
    } catch (caught) {
      setStatus({
        enabled: settings.honcho.enabled,
        configured: false,
        reachable: false,
        state: 'unreachable',
        endpoint: settings.honcho.base_url,
        workspace_id: settings.honcho.workspace_id,
        peer_id: settings.honcho.peer_id,
        token_configured: false,
        detail: caught instanceof Error ? caught.message : 'Honcho status check failed',
      })
    } finally {
      setChecking(false)
    }
  }

  return (
    <SettingsSection
      title="Honcho memory sidecar"
      description="Optional session memory for deliberately selected agent episodes. It is not the source of truth and is never wired into normal ingestion by default."
    >
      <div className="form-grid">
        <label className="form-field">
          <span>Adapter status</span>
          <input type="text" value="Optional sidecar" disabled />
          <small>
            Saving records configuration only; it does not copy the corpus or start a worker.
          </small>
        </label>
        <label className="form-field">
          <span>Ingestion integration</span>
          <input
            type="text"
            value={settings.honcho.wired_to_ingestion ? 'Enabled' : 'Disabled'}
            disabled
          />
          <small>Normal source sync remains unchanged.</small>
        </label>
        <label className="form-field">
          <span>Enabled</span>
          <input
            type="checkbox"
            checked={settings.honcho.enabled}
            onChange={(event) => setHoncho({ ...settings.honcho, enabled: event.target.checked })}
          />
        </label>
      </div>
      <div className="safety-note" role="status">
        Honcho uses one deterministic session per retained Cortana document so deletion can remove
        only that document. Keep it disabled until the evaluation, ACL, deletion, and export gates
        pass.
      </div>
      <div className="safety-note" role="status">
        <span>
          Health: {status?.state.replace('_', ' ') || 'not checked'}
          {status?.detail ? ` — ${status.detail}` : ''}
        </span>
        <button
          type="button"
          className="secondary-button"
          onClick={() => void checkStatus()}
          disabled={checking}
        >
          <RefreshCw size={14} /> {checking ? 'Checking…' : 'Check connection'}
        </button>
      </div>
      {externalStatus && (
        <p className="settings-note">
          This health snapshot is retained while you move between Desktop settings sections. It
          reads the last saved Honcho configuration; save changes before checking again.
        </p>
      )}
      <Field label="Provider" hint="Honcho currently supports only its v3 HTTP API.">
        <select value={settings.honcho.provider} disabled>
          <option value="honcho">honcho</option>
        </select>
      </Field>
      <div className="form-grid">
        <Field label="Endpoint" wide>
          <input
            type="url"
            value={settings.honcho.base_url}
            onChange={(event) => setHoncho({ ...settings.honcho, base_url: event.target.value })}
            required
          />
        </Field>
        <Field label="Workspace ID" hint="letters, numbers, dots, dashes, or underscores">
          <input
            value={settings.honcho.workspace_id}
            onChange={(event) =>
              setHoncho({ ...settings.honcho, workspace_id: event.target.value })
            }
            pattern="[A-Za-z0-9._-]{1,128}"
            maxLength={128}
            required
          />
        </Field>
        <Field label="Peer ID" hint="the Honcho agent peer identity">
          <input
            value={settings.honcho.peer_id}
            onChange={(event) => setHoncho({ ...settings.honcho, peer_id: event.target.value })}
            pattern="[A-Za-z0-9._-]{1,128}"
            maxLength={128}
            required
          />
        </Field>
        <Field label="Session prefix" hint="stable namespace prefix for per-document sessions">
          <input
            value={settings.honcho.session_prefix}
            onChange={(event) =>
              setHoncho({ ...settings.honcho, session_prefix: event.target.value })
            }
            pattern="[A-Za-z0-9._-]{1,128}"
            maxLength={128}
            required
          />
        </Field>
        <Field label="Token environment variable">
          <input
            value={settings.honcho.token_env || ''}
            onChange={(event) =>
              setHoncho({ ...settings.honcho, token_env: event.target.value || null })
            }
            pattern="[A-Z_][A-Z0-9_]*"
            placeholder="CORTANA_HONCHO_TOKEN"
          />
        </Field>
        <Field label="New token" hint="write-only; leave blank to retain">
          <div className="secret-input">
            <input
              type="password"
              autoComplete="new-password"
              value={settings.honcho.token_env ? secretValues[settings.honcho.token_env] || '' : ''}
              disabled={!settings.honcho.token_env}
              onChange={(event) => {
                if (!settings.honcho.token_env) return
                setStatus(null)
                onSecret({ ...secretValues, [settings.honcho.token_env]: event.target.value })
              }}
            />
            {settings.honcho.token_env &&
              statusSource?.configured &&
              !clearedSecrets.has(statusSource.name) && (
                <button
                  type="button"
                  onClick={() => {
                    setStatus(null)
                    onClearSecret(settings.honcho.token_env!)
                  }}
                >
                  Clear
                </button>
              )}
          </div>
        </Field>
      </div>
    </SettingsSection>
  )
}

function AccessSection({
  settings,
  update,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
}: SettingsSectionProps & {
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
}) {
  const change = (index: number, patch: Partial<AuthPrincipalSettings>) =>
    update((current) => ({
      ...current,
      auth_principals: current.auth_principals.map((principal, position) =>
        position === index ? { ...principal, ...patch } : principal
      ),
    }))
  const add = () =>
    update((current) => {
      const usedPrincipals = new Set(
        current.auth_principals.map((principal) => principal.principal)
      )
      const usedTokens = new Set(current.auth_principals.map((principal) => principal.token_env))
      let number = 1
      while (
        usedPrincipals.has(`agent-${number}`) ||
        usedTokens.has(`CORTANA_AGENT_${number}_TOKEN`)
      ) {
        number += 1
      }
      return {
        ...current,
        auth_principals: [
          ...current.auth_principals,
          {
            principal: `agent-${number}`,
            token_env: `CORTANA_AGENT_${number}_TOKEN`,
            scopes: ['query', 'status'],
            acl: current.workspaces.map((workspace) => workspace.id),
          },
        ],
      }
    })

  return (
    <SettingsSection
      title="Agent access"
      description="Create named bearer principals with least-privilege scopes and workspace ACL labels. Token values are write-only and never return to the renderer."
    >
      <div className="principal-list">
        {settings.auth_principals.map((principal, index) => {
          const secret = settings.secrets.find((item) => item.name === principal.token_env)
          return (
            <article className="principal-card" key={`${principal.principal}:${index}`}>
              <header>
                <KeyRound size={16} />
                <strong>{principal.principal || `Principal ${index + 1}`}</strong>
                <button
                  type="button"
                  className="quick-tooltip"
                  aria-label={`Remove ${principal.principal}`}
                  title={`Remove ${principal.principal}`}
                  data-tooltip={`Remove ${principal.principal}`}
                  onClick={() =>
                    update((current) => ({
                      ...current,
                      auth_principals: current.auth_principals.filter(
                        (_, position) => position !== index
                      ),
                    }))
                  }
                >
                  <Trash2 size={15} />
                </button>
              </header>
              <div className="form-grid">
                <Field label="Principal name">
                  <input
                    value={principal.principal}
                    maxLength={128}
                    required
                    onChange={(event) => change(index, { principal: event.target.value })}
                  />
                </Field>
                <Field label="Token environment name">
                  <input
                    value={principal.token_env}
                    maxLength={128}
                    pattern="[A-Za-z_][A-Za-z0-9_]*"
                    required
                    onChange={(event) => change(index, { token_env: event.target.value })}
                  />
                </Field>
                <Field label="New bearer token" hint="write-only; leave blank to retain">
                  <input
                    type="password"
                    autoComplete="new-password"
                    value={secretValues[principal.token_env] || ''}
                    onChange={(event) =>
                      onSecret({ ...secretValues, [principal.token_env]: event.target.value })
                    }
                  />
                  {secret?.configured && !clearedSecrets.has(principal.token_env) && (
                    <button type="button" onClick={() => onClearSecret(principal.token_env)}>
                      Clear stored token
                    </button>
                  )}
                </Field>
                <Field label="ACL labels" hint="comma-separated workspace IDs; * grants all">
                  <input
                    value={principal.acl.join(', ')}
                    onChange={(event) =>
                      change(index, {
                        acl: event.target.value
                          .split(',')
                          .map((value) => value.trim())
                          .filter(Boolean),
                      })
                    }
                  />
                </Field>
              </div>
              <div className="scope-options">
                {(['query', 'status', 'admin'] as const).map((scope) => (
                  <label key={scope}>
                    <input
                      type="checkbox"
                      checked={principal.scopes.includes(scope)}
                      onChange={(event) =>
                        change(index, {
                          scopes: event.target.checked
                            ? [...principal.scopes, scope]
                            : principal.scopes.filter((value) => value !== scope),
                        })
                      }
                    />
                    {scope}
                  </label>
                ))}
              </div>
            </article>
          )
        })}
      </div>
      <button type="button" className="secondary-button" onClick={add}>
        <Plus size={15} /> Add principal
      </button>
      <p className="settings-note">
        Settings take effect after the server restarts. Desktop requests select a matching private
        native credential by scope without exposing it to web content.
      </p>
    </SettingsSection>
  )
}

function AuditSection() {
  const [runtime, setRuntime] = useState<AuditEvent[]>([])
  const [desktop, setDesktop] = useState<AuditEvent[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const refreshRequestRef = useRef(0)

  const refresh = async () => {
    const requestId = ++refreshRequestRef.current
    setLoading(true)
    setError('')
    const [runtimeResult, desktopResult] = await Promise.allSettled([
      getRuntimeAudit(100),
      getDesktopAudit(100),
    ])
    // A manual refresh can overlap the initial request. Never let a slower
    // response replace a newer audit snapshot or clear its error state.
    if (refreshRequestRef.current !== requestId) return
    if (runtimeResult.status === 'fulfilled') setRuntime(runtimeResult.value)
    if (desktopResult.status === 'fulfilled') setDesktop(desktopResult.value)
    const errors = [runtimeResult, desktopResult]
      .filter((result): result is PromiseRejectedResult => result.status === 'rejected')
      .map((result) =>
        result.reason instanceof Error ? result.reason.message : 'Audit source unavailable'
      )
    setError(errors.join(' · '))
    setLoading(false)
  }

  useEffect(() => {
    void refresh()
    return () => {
      refreshRequestRef.current += 1
    }
  }, [])

  // The events already shown here are the redacted, bounded metadata snapshots
  // returned by the runtime and Desktop audit endpoints; this export writes
  // exactly those loaded events to a JSON file and adds nothing else.
  const exportAudit = () => {
    const payload = {
      exported_at: new Date().toISOString(),
      runtime,
      desktop,
    }
    const blob = new Blob([JSON.stringify(payload, null, 2)], {
      type: 'application/json',
    })
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `cortana-audit-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(anchor)
    anchor.click()
    anchor.remove()
    // Defer the revoke one tick so the browser can initiate the download
    // before the object URL is torn down.
    window.setTimeout(() => URL.revokeObjectURL(url), 0)
  }

  return (
    <SettingsSection
      title="Audit trail"
      description="Bounded metadata-only runtime and Desktop events. Queries, document contents, bearer tokens, and secret values are excluded."
    >
      <div className="source-settings-toolbar">
        <span>
          {runtime.length} runtime · {desktop.length} Desktop events
        </span>
        <div className="service-actions">
          <button type="button" disabled={loading} onClick={() => void refresh()}>
            {loading ? <LoaderCircle className="spin" size={14} /> : <RefreshCw size={14} />}
            Refresh
          </button>
          <button
            type="button"
            className="secondary-button"
            disabled={loading}
            onClick={exportAudit}
          >
            <Download size={14} /> Export
          </button>
        </div>
      </div>
      {error && (
        <div className="safety-note" role="alert">
          {error}
        </div>
      )}
      <AuditList title="Runtime retrieval" events={runtime} />
      <AuditList title="Desktop actions" events={desktop} />
    </SettingsSection>
  )
}

function AuditList({ title, events }: { title: string; events: AuditEvent[] }) {
  return (
    <div className="audit-list">
      <h3>{title}</h3>
      {events.length === 0 ? (
        <p>No events available.</p>
      ) : (
        events.map((event, index) => (
          <article key={`${String(event.id || event.at_unix_seconds || 'event')}:${index}`}>
            <strong>{String(event.event || event.action || 'event')}</strong>
            <time>
              {event.timestamp
                ? new Date(String(event.timestamp)).toLocaleString()
                : event.at_unix_seconds
                  ? new Date(Number(event.at_unix_seconds) * 1000).toLocaleString()
                  : ''}
            </time>
            <pre>{JSON.stringify(event, null, 2)}</pre>
          </article>
        ))
      )}
    </div>
  )
}

function ReadinessSection({
  autoScan = false,
  readiness,
  onResult,
  onOpenServices,
  job,
  onJob,
  readinessActivity,
  onReadinessScan,
  pollInstaller = true,
}: {
  autoScan?: boolean
  readiness: DesktopReadiness | null
  onResult: (readiness: DesktopReadiness | null) => void
  onOpenServices?: () => void
  job: DesktopInstallJob | null
  onJob: (job: DesktopInstallJob | null) => void
  readinessActivity?: DesktopReadinessActivity | null
  onReadinessScan?: () => Promise<DesktopReadiness>
  pollInstaller?: boolean
}) {
  const foreground = useDesktopForeground()
  const [scanning, setScanning] = useState(false)
  const [migratingGeneration, setMigratingGeneration] = useState(false)
  const [error, setError] = useState('')
  const [migrationNotice, setMigrationNotice] = useState('')
  const autoScanAttemptedRef = useRef(false)

  useEffect(() => {
    if (!pollInstaller || !foreground || !job || !['running', 'cancelling'].includes(job.status)) {
      return
    }
    let active = true
    const timer = window.setTimeout(() => {
      void getDesktopInstaller(job.id)
        .then((next) => {
          if (!active) return
          onJob(next)
          if (next.status === 'succeeded') {
            onResult(null)
            setScanning(true)
            void (onReadinessScan ? onReadinessScan() : scanDesktopReadiness())
              .then((scan) => {
                if (!active) return
                onResult(scan)
              })
              .catch((caught: unknown) => {
                if (active) {
                  setError(
                    caught instanceof Error ? caught.message : 'Post-install readiness scan failed'
                  )
                }
              })
              .finally(() => {
                if (active) setScanning(false)
              })
          }
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
  }, [foreground, job, onJob, onReadinessScan, onResult, pollInstaller])

  const scan = async () => {
    setScanning(true)
    setError('')
    setMigrationNotice('')
    try {
      const next = await (onReadinessScan ? onReadinessScan() : scanDesktopReadiness())
      onResult(next)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Readiness scan failed')
    } finally {
      setScanning(false)
    }
  }

  const migrateGeneration = async () => {
    const generation = readiness?.core?.embedding_generation
    if (!generation?.stored || generation.stored === generation.configured) return
    const from = generation.stored
    if (
      !window.confirm(
        `Adopt the stored embedding generation?\n\n${from}\n\nUse this only when the configured model, dimension, and vector space are unchanged and only the provider fingerprint changed. Cortana will create a verified backup, update generation metadata, and clear derived caches. Indexed documents will not be rebuilt. Continue?`
      )
    ) {
      return
    }
    setMigratingGeneration(true)
    setError('')
    setMigrationNotice('')
    try {
      await migrateDesktopEmbeddingGeneration(from)
      try {
        const next = await (onReadinessScan ? onReadinessScan() : scanDesktopReadiness())
        onResult(next)
        const nextGeneration = next.core?.embedding_generation
        if (!nextGeneration || nextGeneration.stored !== nextGeneration.configured) {
          setError(
            'Embedding generation was adopted, but the follow-up readiness scan still reports a mismatch.'
          )
        } else {
          setMigrationNotice('Embedding generation adopted and readiness was rescanned.')
        }
      } catch (caught) {
        setError(
          caught instanceof Error
            ? `Embedding generation was adopted, but readiness could not be rescanned: ${caught.message}`
            : 'Embedding generation was adopted, but readiness could not be rescanned'
        )
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Embedding generation migration failed')
    } finally {
      setMigratingGeneration(false)
    }
  }

  useEffect(() => {
    if (!autoScan || readiness || autoScanAttemptedRef.current) return
    // First-launch readiness is intentionally one-shot. A failed scan is
    // surfaced for the operator to retry explicitly; it must not loop every
    // time the shell-owned activity status changes to failed.
    autoScanAttemptedRef.current = true
    let active = true
    setScanning(true)
    setError('')
    void (onReadinessScan ? onReadinessScan() : scanDesktopReadiness())
      .then((next) => {
        if (!active) return
        onResult(next)
      })
      .catch((caught: unknown) => {
        if (active) {
          setError(caught instanceof Error ? caught.message : 'Readiness scan failed')
        }
      })
      .finally(() => {
        if (active) setScanning(false)
      })
    return () => {
      active = false
    }
  }, [autoScan, onReadinessScan, onResult, readiness])

  const readinessInFlight =
    scanning || migratingGeneration || readinessActivity?.status === 'running'
  const readinessActivityError =
    readinessActivity?.status === 'failed' ? readinessActivity.detail : null
  const embeddingGeneration = readiness?.core?.embedding_generation
  const embeddingGenerationMismatch = Boolean(
    embeddingGeneration?.stored && embeddingGeneration.stored !== embeddingGeneration.configured
  )

  const install = async (tool: string, label: string) => {
    const action =
      tool === 'connectors'
        ? 'Cortana will create the per-user connector environment from the signed Desktop bundle and install its bounded ingestion dependencies with uv.'
        : tool === 'embedding-runtime'
          ? 'Cortana will install the text-embeddings-inference runtime with Homebrew. The model itself is downloaded by the runtime on first start and no ingestion will begin.'
          : 'Cortana will run its fixed, platform-specific installer.'
    if (
      !window.confirm(
        `Install ${label} on this computer?\n\n${action} No ingestion or sync will start.`
      )
    ) {
      return
    }
    setError('')
    try {
      onJob(await startDesktopInstaller(tool))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Installer failed to start')
    }
  }

  const cancel = async () => {
    if (!job) return
    try {
      onJob(await cancelDesktopInstaller(job.id))
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
        <button
          type="button"
          className="secondary-button"
          disabled={readinessInFlight}
          onClick={scan}
        >
          {readinessInFlight ? (
            <LoaderCircle className="spin" size={15} />
          ) : (
            <RefreshCw size={15} />
          )}
          {readinessInFlight ? 'Checking system…' : readiness ? 'Run again' : 'Run readiness scan'}
        </button>
        {readiness && (
          <span>
            Last checked {new Date(readiness.scanned_at_unix_seconds * 1000).toLocaleTimeString()}
          </span>
        )}
      </div>
      {(error || readinessActivityError) && (
        <div className="safety-note" role="alert">
          <AlertTriangle size={16} /> <span>{error || readinessActivityError}</span>
        </div>
      )}
      {migrationNotice && (
        <div className="safety-note" role="status">
          <Check size={16} /> <span>{migrationNotice}</span>
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
            {embeddingGenerationMismatch && (
              <div className="safety-note" role="status">
                <span>
                  The index uses a different embedding generation. Adopt it only after confirming
                  that the vectors are interchangeable; otherwise rebuild or import a new
                  generation.
                </span>
                <button
                  type="button"
                  className="secondary-button"
                  disabled={readinessInFlight}
                  onClick={() => void migrateGeneration()}
                >
                  {migratingGeneration ? 'Adopting generation…' : 'Adopt stored generation'}
                </button>
              </div>
            )}
          </div>
          {readiness.core && !readiness.core.passed && onOpenServices && (
            <div className="safety-note" role="status">
              <span>
                Runtime checks are not passing. Confirm the API and embedding services are installed
                and running before retrying readiness.
              </span>
              <button type="button" className="secondary-button" onClick={onOpenServices}>
                Check Services
              </button>
            </div>
          )}
        </>
      )}
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
  const [logoError, setLogoError] = useState('')
  const hasWorkspaceSources = (workspaceId: string) =>
    settings.sources.some((source) => source.project === workspaceId)

  const updateLogo = async (workspaceId: string, file: File | undefined) => {
    if (!file) return
    try {
      writeWorkspaceLogo(workspaceId, await readWorkspaceLogoFile(file))
      setLogoError('')
    } catch (caught) {
      setLogoError(caught instanceof Error ? caught.message : 'Workspace logo could not be saved.')
    }
  }

  const addWorkspace = () =>
    update((current) => {
      const nextName = 'New workspace'
      const nextId = ensureWorkspaceIdentifierUnique(
        deriveWorkspaceIdentifier(nextName),
        current.workspaces.map((workspace) => workspace.id)
      )
      return {
        ...current,
        workspaces: [
          ...current.workspaces,
          {
            id: nextId,
            name: nextName,
            account_label: null,
            color: '#A875D6',
          },
        ],
      }
    })

  const changeWorkspace = (index: number, patch: Partial<WorkspaceSettings>) => {
    update((current) => {
      const currentWorkspace = current.workspaces[index]
      if (!currentWorkspace) return current

      const remainingIds = current.workspaces
        .map((workspace) => workspace.id)
        .filter((candidate) => candidate !== currentWorkspace.id)
      const nextName = patch.name ?? currentWorkspace.name
      const shouldDeriveId =
        patch.id === undefined &&
        patch.name !== undefined &&
        !hasWorkspaceSources(currentWorkspace.id) &&
        isWorkspaceIdDerivedFromName(currentWorkspace)
      const nextId = patch.id
        ? patch.id
        : shouldDeriveId
          ? ensureWorkspaceIdentifierUnique(deriveWorkspaceIdentifier(nextName), remainingIds)
          : currentWorkspace.id

      return {
        ...current,
        workspaces: current.workspaces.map((workspace, position) =>
          position === index
            ? {
                ...workspace,
                ...patch,
                id: nextId,
              }
            : workspace
        ),
      }
    })
  }

  return (
    <SettingsSection
      title="Workspaces"
      description="Create up to three isolated query scopes. Sources and accounts can be assigned to one scope. Workspace logos stay local to this Desktop profile and never enter the index or portable settings export."
    >
      <div className="workspace-settings-grid">
        {settings.workspaces.map((workspace, index) => (
          <article className="workspace-card" key={`${workspace.id}:${index}`}>
            <div className="workspace-card-heading">
              <WorkspaceLogo workspace={workspace} size="large" />
              <div className="workspace-card-title">
                <strong>{workspace.name || 'New workspace'}</strong>
                <small>Workspace identity</small>
              </div>
              <label
                className="workspace-logo-upload quick-tooltip"
                title="Upload workspace logo"
                data-tooltip="Upload workspace logo"
              >
                <Upload size={14} />
                <span className="visually-hidden">Upload logo for {workspace.name}</span>
                <input
                  type="file"
                  accept="image/png,image/jpeg,image/webp,image/gif"
                  onChange={(event) => {
                    void updateLogo(workspace.id, event.target.files?.[0])
                    event.currentTarget.value = ''
                  }}
                />
              </label>
              {settings.workspaces.length > 1 && (
                <button
                  type="button"
                  className="quick-tooltip"
                  aria-label={`Remove ${workspace.name}`}
                  disabled={hasWorkspaceSources(workspace.id)}
                  title={
                    hasWorkspaceSources(workspace.id)
                      ? 'Move assigned sources before removing this workspace'
                      : 'Remove workspace'
                  }
                  data-tooltip={
                    hasWorkspaceSources(workspace.id)
                      ? 'Move assigned sources before removing this workspace'
                      : 'Remove workspace'
                  }
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
            <details className="workspace-advanced-details">
              <summary>Advanced workspace details</summary>
              <div className="workspace-advanced-fields">
                <small className="workspace-advanced-note">
                  ID is internal; account labels are optional metadata.
                </small>
                <Field label="Scope ID" hint="generated from the display name; used internally">
                  <input
                    value={workspace.id}
                    readOnly
                    disabled={hasWorkspaceSources(workspace.id)}
                    aria-disabled={hasWorkspaceSources(workspace.id)}
                    title="Generated from the display name and used internally"
                    required
                    maxLength={32}
                    pattern="[a-z0-9][a-z0-9_-]*"
                  />
                </Field>
                <Field
                  label="Account label"
                  hint="optional display note; OAuth credentials belong to each source"
                >
                  <input
                    value={workspace.account_label || ''}
                    onChange={(event) =>
                      changeWorkspace(index, { account_label: event.target.value || null })
                    }
                    maxLength={128}
                    placeholder="e.g. Nifty League"
                  />
                </Field>
              </div>
            </details>
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
      {logoError && (
        <p className="settings-inline-error" role="alert">
          {logoError}
        </p>
      )}
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
  { value: 'github', label: 'GitHub code' },
  { value: 'slack', label: 'Slack' },
  { value: 'discord', label: 'Discord' },
]

const UNASSIGNED_WORKSPACE = '__unassigned__'

function initialSourceWorkspace(settings: DesktopSettings): string {
  const workspaceIds = new Set(settings.workspaces.map((workspace) => workspace.id))
  if (settings.sources.some((source) => !workspaceIds.has(source.project))) {
    return UNASSIGNED_WORKSPACE
  }
  return (
    settings.workspaces.find((workspace) =>
      settings.sources.some((source) => source.project === workspace.id)
    )?.id ||
    settings.workspaces[0]?.id ||
    UNASSIGNED_WORKSPACE
  )
}

function SourcesSection({
  settings,
  update,
  canValidate,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  onJob,
  sourceJobs,
}: SettingsSectionProps & {
  canValidate: boolean
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  onJob?: (job: DesktopSourceJob) => void
  sourceJobs?: DesktopSourceJob[]
}) {
  const [job, setJob] = useState<DesktopSourceJob | null>(null)
  const applyJob = (next: DesktopSourceJob) => {
    setJob(next)
    onJob?.(next)
  }
  const [error, setError] = useState('')
  const [githubRepositories, setGithubRepositories] = useState<
    Record<string, { items: GithubRepositorySummary[]; truncated: boolean }>
  >({})
  const [githubRepositoriesLoading, setGithubRepositoriesLoading] = useState<string | null>(null)
  const [discordChannels, setDiscordChannels] = useState<
    Record<string, { guilds: DiscordGuildChannels[]; truncated: boolean }>
  >({})
  const [discordChannelsLoading, setDiscordChannelsLoading] = useState<string | null>(null)
  const [discordServers, setDiscordServers] = useState<
    Record<string, { guilds: DiscordServerSummary[]; truncated: boolean }>
  >({})
  const [discordServersLoading, setDiscordServersLoading] = useState<string | null>(null)
  const [slackWorkspaces, setSlackWorkspaces] = useState<
    Record<string, { teams: SlackWorkspaceSummary[]; truncated: boolean }>
  >({})
  const [slackWorkspacesLoading, setSlackWorkspacesLoading] = useState<string | null>(null)
  const [buzzCommunities, setBuzzCommunities] = useState<
    Record<string, { communities: BuzzCommunitySummary[]; truncated: boolean }>
  >({})
  const [buzzCommunitiesLoading, setBuzzCommunitiesLoading] = useState<string | null>(null)
  const [sourceWorkspace, setSourceWorkspace] = useState(() => initialSourceWorkspace(settings))
  const [initialSync, setInitialSync] = useState<{
    source: string
    budget: InitialSyncBudget
    plan: DesktopInitialSyncPlan | null
    planning: boolean
    flowError: string
  } | null>(null)
  const validationPlanKey = useRef('')
  const sharedJobIds = useRef(new Set<string>())
  const cancelInFlight = useRef(new Set<string>())
  const foreground = useDesktopForeground()

  const workspaceIds = settings.workspaces.map((workspace) => workspace.id)
  const unassignedSourceCount = settings.sources.filter(
    (source) => !workspaceIds.includes(source.project)
  ).length
  const sourceWorkspaceIsAssigned = workspaceIds.includes(sourceWorkspace)
  const visibleSources = settings.sources
    .map((source, index) => ({ source, index }))
    .filter(({ source }) =>
      sourceWorkspace === UNASSIGNED_WORKSPACE
        ? !workspaceIds.includes(source.project)
        : source.project === sourceWorkspace
    )
  const selectedWorkspace = settings.workspaces.find(({ id }) => id === sourceWorkspace)

  useEffect(() => {
    if (
      sourceWorkspaceIsAssigned ||
      (sourceWorkspace === UNASSIGNED_WORKSPACE && unassignedSourceCount)
    ) {
      return
    }
    setSourceWorkspace(initialSourceWorkspace(settings))
  }, [sourceWorkspace, sourceWorkspaceIsAssigned, unassignedSourceCount, settings.workspaces])

  // In the full Desktop shell, App owns one poller for the source-job list so
  // SourcePanel, the tray/status bar, and Settings all observe the same
  // snapshots. A standalone SettingsView still uses its local observer.
  useEffect(() => {
    if (!sourceJobs) return
    const currentIds = new Set(sourceJobs.map((candidate) => candidate.id))
    sourceJobs.forEach((candidate) => sharedJobIds.current.add(candidate.id))
    for (const id of sharedJobIds.current) {
      if (!currentIds.has(id) && id !== job?.id) sharedJobIds.current.delete(id)
    }
    // A job may have started while Settings was unmounted. Adopt the newest
    // recovered snapshot so this section can show and cancel it immediately,
    // instead of only locking the editor in the background.
    if (!job) {
      if (sourceJobs[0]) setJob(sourceJobs[0])
      return
    }
    const next = sourceJobs.find((candidate) => candidate.id === job.id)
    if (next && next !== job) setJob(next)
    else if (!next && sharedJobIds.current.has(job.id)) {
      sharedJobIds.current.delete(job.id)
      setJob(null)
    }
  }, [job, sourceJobs])

  useEffect(() => {
    if (sourceJobs || !foreground) return
    if (!job || !['running', 'cancelling'].includes(job.status)) return
    let active = true
    const timer = window.setTimeout(() => {
      void getDesktopSourceValidation(job.id)
        .then((next) => {
          if (!active) return
          applyJob(next)
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
  }, [foreground, job, initialSync, onJob])
  const activeJob =
    (job && ['running', 'cancelling'].includes(job.status) ? job : undefined) ??
    sourceJobs?.find((candidate) => ['running', 'cancelling'].includes(candidate.status))
  const observedJob = activeJob ?? job ?? sourceJobs?.[0]
  const initialSyncSource = initialSync
    ? settings.sources.find((item) => item.name === initialSync.source)
    : undefined
  const requestPlan = async (source: string, budget: InitialSyncBudget) => {
    setInitialSync((current) =>
      current && current.source === source
        ? { ...current, budget, plan: null, planning: true, flowError: '' }
        : current
    )
    try {
      const plan = await planDesktopInitialSync(source, budget)
      setInitialSync((current) =>
        current && current.source === source && current.budget === budget
          ? { ...current, plan, planning: false, flowError: '' }
          : current
      )
    } catch (caught) {
      setInitialSync((current) =>
        current && current.source === source
          ? {
              ...current,
              planning: false,
              flowError:
                caught instanceof Error ? caught.message : 'Initial sync plan request failed',
            }
          : current
      )
    }
  }

  // Whether polling is owned by this section or by App, a successful
  // validation must unlock a fresh plan for the selected initial-sync budget.
  // Keeping this transition here avoids coupling the flow to one polling
  // implementation and prevents duplicate plan requests on rerenders.
  useEffect(() => {
    if (
      !observedJob ||
      observedJob.status !== 'succeeded' ||
      observedJob.operation !== 'validation' ||
      !initialSync ||
      initialSync.source !== observedJob.source
    ) {
      return
    }
    const key = `${observedJob.id}:${initialSync.budget}`
    if (validationPlanKey.current === key) return
    validationPlanKey.current = key
    void requestPlan(observedJob.source, initialSync.budget)
  }, [initialSync, observedJob])

  const openInitialSync = (source: SourceSettings, budget: InitialSyncBudget = 'small') => {
    setInitialSync({
      source: source.name,
      budget,
      plan: null,
      planning: false,
      flowError: '',
    })
    void requestPlan(source.name, budget)
  }

  const validateInitialSyncBudget = async (source: SourceSettings) => {
    if (!initialSync) return
    if (!canValidate) {
      setError(
        'Save source changes before validating so the native runtime uses this exact config.'
      )
      return
    }
    const budget = initialSync.budget
    if (
      !window.confirm(
        `Validate ${source.name} for an initial sync budget?\n\nCortana may read up to ${budgetLabel(budget)} without embedding, indexing, reconciling, or starting a sync.`
      )
    ) {
      return
    }
    setError('')
    try {
      applyJob(await startDesktopSourceValidation(source.name, budget))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Budget validation failed to start')
    }
  }

  const startInitialSync = async (source: SourceSettings) => {
    if (!initialSync?.plan) return
    const plan = initialSync.plan
    if (
      !window.confirm(
        `Start a guarded initial sync for ${source.name}?\n\n` +
          `It may embed and index at most ${plan.budget_documents} documents or ${mebibytes(plan.budget_bytes)} MiB for up to ${minutes(plan.budget_seconds)} minutes. ` +
          `It requires a matching successful validation, and it will not delete or reconcile existing records. ` +
          `Committed batches remain indexed if you cancel.`
      )
    ) {
      return
    }
    setError('')
    try {
      const next = await startDesktopInitialSync(source.name, plan.budget, plan.plan_id)
      applyJob(next)
      setInitialSync(null)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Initial sync failed to start')
    }
  }

  const changeSource = (index: number, patch: Partial<SourceSettings>) => {
    const current = settings.sources[index]
    if (activeJob && current?.name === activeJob.source) return
    // A native initial-sync plan is tied to the exact saved source config.
    // Editing or renaming that source invalidates the plan; force a fresh
    // plan/validation rather than leaving an old plan ID in the UI.
    if (initialSync && current?.name === initialSync.source) setInitialSync(null)
    update((current) => ({
      ...current,
      sources: current.sources.map((source, position) =>
        position === index ? { ...source, ...patch } : source
      ),
    }))
  }

  const addSource = () => {
    const targetWorkspace = sourceWorkspaceIsAssigned
      ? sourceWorkspace
      : settings.workspaces[0]?.id || 'personal'
    if (!sourceWorkspaceIsAssigned) setSourceWorkspace(targetWorkspace)
    update((current) => ({
      ...current,
      sources: [...current.sources, newSource(current, targetWorkspace)],
    }))
  }

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
      applyJob(await startDesktopSourceValidation(source.name))
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
    const provider =
      source.kind === 'github'
        ? 'GitHub'
        : source.kind === 'discord'
          ? 'Discord'
          : source.kind === 'slack'
            ? 'Slack'
            : 'Google'
    if (
      !window.confirm(
        `Authorize ${source.name} with ${provider}?\n\nCortana will open the system browser and store the resulting token in the configured private file. No source data is read during authorization.`
      )
    ) {
      return
    }
    setError('')
    try {
      applyJob(await startDesktopSourceAuthorization(source.name))
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : `${provider} authorization failed to start`
      )
    }
  }

  const discoverGithubRepositories = async (index: number, source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before discovering repositories.')
      return
    }
    setError('')
    setGithubRepositoriesLoading(source.name)
    try {
      const result = await listDesktopGithubRepositories(source.name)
      setGithubRepositories((current) => ({
        ...current,
        [source.name]: { items: result.repositories, truncated: result.truncated },
      }))
      if (result.truncated) {
        setError(
          'GitHub returned more than 1,000 repositories; select from the most recently updated 1,000.'
        )
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'GitHub repository discovery failed')
    } finally {
      setGithubRepositoriesLoading(null)
    }
  }

  const toggleGithubRepository = (index: number, source: SourceSettings, fullName: string) => {
    const repositories = source.repositories.includes(fullName)
      ? source.repositories.filter((repository) => repository !== fullName)
      : [...source.repositories, fullName]
    changeSource(index, { repositories })
  }

  const discoverDiscordChannels = async (index: number, source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before discovering channels.')
      return
    }
    setError('')
    setDiscordChannelsLoading(source.name)
    try {
      const result = await listDesktopDiscordChannels(source.name)
      setDiscordChannels((current) => ({
        ...current,
        [source.name]: { guilds: result.guilds, truncated: result.truncated },
      }))
      if (result.truncated) {
        setError('Discord returned more than 100 servers; select from the first 100.')
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Discord channel discovery failed')
    } finally {
      setDiscordChannelsLoading(null)
    }
  }

  const toggleDiscordChannel = (index: number, source: SourceSettings, channelId: string) => {
    const channels = source.channels.includes(channelId)
      ? source.channels.filter((channel) => channel !== channelId)
      : [...source.channels, channelId]
    changeSource(index, { channels })
  }

  const discoverDiscordServers = async (index: number, source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before discovering servers.')
      return
    }
    setError('')
    setDiscordServersLoading(source.name)
    try {
      const result = await listDesktopDiscordServers(source.name)
      setDiscordServers((current) => ({
        ...current,
        [source.name]: { guilds: result.guilds, truncated: result.truncated },
      }))
      if (result.truncated) {
        setError('Discord returned more than 100 servers; select from the first 100.')
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Discord server discovery failed')
    } finally {
      setDiscordServersLoading(null)
    }
  }

  const toggleDiscordServer = (index: number, source: SourceSettings, guildId: string) => {
    const servers = source.servers.includes(guildId)
      ? source.servers.filter((server) => server !== guildId)
      : [...source.servers, guildId]
    changeSource(index, { servers })
  }

  const discoverSlackWorkspaces = async (index: number, source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before discovering workspaces.')
      return
    }
    setError('')
    setSlackWorkspacesLoading(source.name)
    try {
      const result = await listDesktopSlackWorkspaces(source.name)
      setSlackWorkspaces((current) => ({
        ...current,
        [source.name]: { teams: result.teams, truncated: result.truncated },
      }))
      if (result.truncated) {
        setError('Slack returned more than 100 teams; select from the first 100.')
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Slack workspace discovery failed')
    } finally {
      setSlackWorkspacesLoading(null)
    }
  }

  const toggleSlackTeam = (index: number, source: SourceSettings, team: SlackWorkspaceSummary) => {
    // A Slack user token is scoped to exactly one workspace, so assigning a
    // team replaces the previous assignment instead of accumulating.
    const assigned = source.teams.includes(team.id)
    changeSource(index, {
      teams: assigned ? [] : [team.id],
      team_names: assigned ? [] : [team.name],
    })
  }

  const discoverBuzzCommunities = async (index: number, source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before discovering communities.')
      return
    }
    setError('')
    setBuzzCommunitiesLoading(source.name)
    try {
      const result = await listDesktopBuzzCommunities(source.name)
      setBuzzCommunities((current) => ({
        ...current,
        [source.name]: {
          communities: result.communities,
          truncated: result.truncated,
        },
      }))
      if (result.truncated) {
        setError('Buzz returned more than 100 communities; select from the first 100.')
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Buzz community discovery failed')
    } finally {
      setBuzzCommunitiesLoading(null)
    }
  }

  const toggleBuzzCommunity = (
    index: number,
    source: SourceSettings,
    community: BuzzCommunitySummary
  ) => {
    // Community assignment is per-workspace: the checked community ids land
    // in the source's `communities` field with display names kept
    // index-aligned in `community_names`. This source belongs to exactly one
    // workspace, so the chooser is scoped to the selected workspace.
    const assigned = source.communities.includes(community.id)
    changeSource(index, {
      communities: assigned
        ? source.communities.filter((id) => id !== community.id)
        : [...source.communities, community.id],
      community_names: assigned
        ? source.community_names.filter(
            (_, position) => source.communities[position] !== community.id
          )
        : [...source.community_names, community.name],
    })
  }

  const trialSyncSource = async (source: SourceSettings) => {
    if (!canValidate) {
      setError('Save source changes before syncing so the native runtime uses this exact config.')
      return
    }
    if (
      !window.confirm(
        `Run a guarded trial sync for ${source.name}?\n\nThis requires a matching successful validation. It may embed and index at most 25 documents or 5 MiB for up to 5 minutes. It will not delete or reconcile existing records. Committed batches remain indexed if you cancel.`
      )
    ) {
      return
    }
    setError('')
    try {
      applyJob(await startDesktopSourceTrialSync(source.name))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Trial sync failed to start')
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
    kind:
      | 'directory'
      | 'oauth-client'
      | 'google-token'
      | 'github-token'
      | 'discord-token'
      | 'slack-token',
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
    const current = observedJob
    if (!current || current.status !== 'running' || cancelInFlight.current.has(current.id)) return
    cancelInFlight.current.add(current.id)
    const previous = current
    applyJob({
      ...current,
      status: 'cancelling',
      summary: `Cancelling source ${current.operation}…`,
    })
    setError('')
    try {
      applyJob(await cancelDesktopSourceValidation(current.id))
    } catch (caught) {
      applyJob(previous)
      setError(caught instanceof Error ? caught.message : 'Source job cancellation failed')
    } finally {
      cancelInFlight.current.delete(current.id)
    }
  }

  return (
    <SettingsSection
      title="Ingestion sources"
      description="Configure local and account-backed sources per workspace. Saving never ingests data; validation, a deliberately small no-reconcile trial sync, and a fixed-budget guided initial sync are separate confirmed actions."
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

      <div className="source-workspace-tabs" role="tablist" aria-label="Source workspace">
        {settings.workspaces.map((workspace) => {
          const count = settings.sources.filter((source) => source.project === workspace.id).length
          return (
            <button
              type="button"
              role="tab"
              key={workspace.id}
              aria-selected={sourceWorkspace === workspace.id}
              className={sourceWorkspace === workspace.id ? 'active' : ''}
              onClick={() => setSourceWorkspace(workspace.id)}
            >
              <WorkspaceLogo workspace={workspace} size="small" />
              <span>{workspace.name}</span>
              <small>{count}</small>
            </button>
          )
        })}
        {unassignedSourceCount > 0 && (
          <button
            type="button"
            role="tab"
            aria-selected={sourceWorkspace === UNASSIGNED_WORKSPACE}
            className={sourceWorkspace === UNASSIGNED_WORKSPACE ? 'active warning' : 'warning'}
            onClick={() => setSourceWorkspace(UNASSIGNED_WORKSPACE)}
          >
            <AlertTriangle size={15} />
            <span>Needs assignment</span>
            <small>{unassignedSourceCount}</small>
          </button>
        )}
      </div>

      <p className="source-workspace-caption">
        {selectedWorkspace
          ? `Showing sources assigned to ${selectedWorkspace.name}.`
          : 'Assign legacy sources to a workspace before enabling or syncing them.'}
      </p>

      {activeJob && (
        <div className="safety-note" role="status">
          Settings for {activeJob.source} are locked while its operation is running. Other sources
          remain configurable, but source actions still wait until this operation finishes.
        </div>
      )}

      <div className="source-settings-list">
        {settings.sources.length === 0 && (
          <div className="empty-source-settings">
            <strong>No sources configured</strong>
            <span>Add a source, assign its workspace, then save and run bounded validation.</span>
          </div>
        )}
        {settings.sources.length > 0 && visibleSources.length === 0 && (
          <div className="empty-source-settings">
            <strong>
              {sourceWorkspace === UNASSIGNED_WORKSPACE
                ? 'No sources need assignment'
                : `No sources in ${selectedWorkspace?.name || 'this workspace'}`}
            </strong>
            <span>
              {sourceWorkspace === UNASSIGNED_WORKSPACE
                ? 'All configured sources are assigned to a workspace.'
                : 'Add a source to this workspace or choose another workspace above.'}
            </span>
          </div>
        )}
        {visibleSources.map(({ source, index }) => {
          const secret = source.token_env
            ? settings.secrets.find((item) => item.name === source.token_env)
            : undefined
          const runningThis = activeJob?.source === source.name
          const sourceLocked = runningThis
          const sourceLabel =
            SOURCE_KINDS.find((kind) => kind.value === source.kind)?.label || 'External connector'
          const workspaceLabel =
            settings.workspaces.find((workspace) => workspace.id === source.project)?.name ||
            `Unassigned (${source.project || 'legacy scope'})`
          const assignedWorkspace = settings.workspaces.find(
            (workspace) => workspace.id === source.project
          )
          const workspaceAssigned = settings.workspaces.some(
            (workspace) => workspace.id === source.project
          )
          return (
            <article className="source-settings-card" key={`${source.name}:${index}`}>
              <header>
                <label className="source-enable">
                  <input
                    type="checkbox"
                    checked={source.enabled}
                    disabled={sourceLocked || (!workspaceAssigned && !source.enabled)}
                    title={
                      !workspaceAssigned
                        ? 'Assign this source to a workspace before enabling it'
                        : undefined
                    }
                    onChange={(event) => changeSource(index, { enabled: event.target.checked })}
                  />
                  <span
                    className={`source-service-icon source-service-icon--${source.kind}`}
                    title={sourceLabel}
                    aria-label={`${sourceLabel} connector`}
                    role="img"
                  >
                    <SourceIcon kind={source.kind} size={17} />
                  </span>
                  <span>
                    <strong>{sourceDisplayName(source.kind, source.name || 'New source')}</strong>
                    <small>
                      {workspaceLabel} · {source.enabled ? 'Enabled' : 'Disabled'}
                    </small>
                  </span>
                </label>
                <label className="source-workspace-picker">
                  {assignedWorkspace && (
                    <WorkspaceLogo workspace={assignedWorkspace} size="small" />
                  )}
                  <span className="visually-hidden">Assign source workspace</span>
                  <select
                    aria-label={`Workspace for ${source.name}`}
                    value={source.project}
                    disabled={sourceLocked}
                    onChange={(event) => changeSource(index, { project: event.target.value })}
                  >
                    {!workspaceAssigned && source.project && (
                      <option value={source.project}>Unassigned: {source.project}</option>
                    )}
                    {settings.workspaces.map((workspace) => (
                      <option key={workspace.id} value={workspace.id}>
                        {workspace.name}
                      </option>
                    ))}
                  </select>
                </label>
                <div className="source-card-actions">
                  {hasBrowserSetup(source.kind) && (
                    <button
                      type="button"
                      className="source-icon-button quick-tooltip"
                      aria-label="Setup"
                      data-tooltip="Setup"
                      disabled={!canValidate || sourceLocked}
                      title="Open the official provider setup page"
                      onClick={() => void openSetup(source)}
                    >
                      <ExternalLink size={14} />
                    </button>
                  )}
                  {(isGoogleSource(source.kind) ||
                    source.kind === 'github' ||
                    source.kind === 'discord' ||
                    source.kind === 'slack') && (
                    <button
                      type="button"
                      className="source-icon-button quick-tooltip"
                      aria-label="Authorize"
                      data-tooltip="Authorize"
                      disabled={
                        !canValidate ||
                        (source.kind === 'discord' || source.kind === 'slack'
                          ? !source.token_path
                          : !source.token_path && !source.token_env) ||
                        !source.oauth_client_path ||
                        Boolean(activeJob)
                      }
                      title={`Authorize read-only ${source.kind === 'github' ? 'GitHub' : source.kind === 'discord' ? 'Discord' : source.kind === 'slack' ? 'Slack' : 'Google'} access in the browser`}
                      onClick={() => void authorizeSource(source)}
                    >
                      {runningThis && activeJob?.operation === 'authorization' ? (
                        <LoaderCircle className="spin" size={14} />
                      ) : (
                        <KeyRound size={14} />
                      )}
                    </button>
                  )}
                  <button
                    type="button"
                    className="source-icon-button quick-tooltip"
                    aria-label="Validate"
                    data-tooltip="Validate"
                    disabled={!canValidate || Boolean(activeJob) || !workspaceAssigned}
                    title={canValidate ? 'Read-only bounded validation' : 'Save changes first'}
                    onClick={() => void validateSource(source)}
                  >
                    {runningThis && activeJob?.operation === 'validation' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Play size={14} />
                    )}
                  </button>
                  <button
                    type="button"
                    className="source-icon-button quick-tooltip"
                    aria-label="Trial sync"
                    data-tooltip="Trial sync"
                    disabled={
                      !canValidate || !source.enabled || Boolean(activeJob) || !workspaceAssigned
                    }
                    title="Validation-gated trial sync; max 25 documents, 5 MiB, no reconciliation"
                    onClick={() => void trialSyncSource(source)}
                  >
                    {runningThis && activeJob?.operation === 'trial-sync' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Play size={14} />
                    )}
                  </button>
                  <button
                    type="button"
                    className="source-icon-button quick-tooltip"
                    aria-label="Initial sync"
                    data-tooltip="Initial sync"
                    disabled={
                      !canValidate || !source.enabled || Boolean(activeJob) || !workspaceAssigned
                    }
                    title="Guided initial sync; fixed budget, validation-gated, no reconciliation"
                    onClick={() => openInitialSync(source)}
                  >
                    {runningThis && activeJob?.operation === 'initial-sync' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Zap size={14} />
                    )}
                  </button>
                  <button
                    type="button"
                    className="source-icon-button quick-tooltip"
                    aria-label={`Remove ${source.name}`}
                    data-tooltip={`Remove ${source.name}`}
                    disabled={sourceLocked}
                    title={`Remove ${source.name}`}
                    onClick={() => {
                      if (
                        window.confirm(
                          `Remove ${source.name} from configuration? Existing indexed data is not deleted.`
                        )
                      ) {
                        if (initialSync?.source === source.name) setInitialSync(null)
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

              {!workspaceAssigned && (
                <div className="source-unassigned-note" role="alert">
                  <AlertTriangle size={15} />
                  <span>
                    This source uses the legacy <code>{source.project || 'unassigned'}</code> scope.
                    Assign it to a workspace below before enabling, validating, or syncing it.
                  </span>
                </div>
              )}

              <details className="source-settings-details">
                <summary>
                  <span>Advanced source settings</span>
                  <small>Workspace, credentials, filters, and safety limits</small>
                </summary>
                <div className="form-grid source-form-grid">
                  <Field label="Source name" hint="stable lowercase identifier">
                    <input
                      value={source.name}
                      disabled={sourceLocked || !source.editable}
                      required
                      maxLength={64}
                      pattern="[a-z0-9][a-z0-9_-]*"
                      onChange={(event) => changeSource(index, { name: event.target.value })}
                    />
                  </Field>
                  <Field label="Connector">
                    <select
                      value={source.kind}
                      disabled={sourceLocked || !source.editable}
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
                      disabled={sourceLocked}
                      onChange={(event) => changeSource(index, { project: event.target.value })}
                    >
                      {!workspaceAssigned && source.project && (
                        <option value={source.project}>Unassigned: {source.project}</option>
                      )}
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
                          disabled={sourceLocked || !source.editable}
                          required={source.enabled}
                          placeholder="/Users/you/Documents"
                          onChange={(event) =>
                            changeSource(index, { root: event.target.value || null })
                          }
                        />
                        <button
                          type="button"
                          disabled={sourceLocked || !source.editable}
                          aria-label="Choose source directory"
                          title="Choose source directory"
                          data-tooltip="Choose source directory"
                          className="quick-tooltip"
                          onClick={() => void choosePath(index, 'directory', 'root')}
                        >
                          <FolderOpen size={14} />
                        </button>
                      </div>
                    </Field>
                  )}
                  {source.kind === 'buzz' && (
                    <Field
                      label="Community chooser"
                      hint="assign the communities this workspace may index; the list comes from Buzz's read-only agents/teams.json identity file in the configured data directory, so make sure the Buzz app has written it first"
                      wide
                    >
                      <div className="source-repository-chooser">
                        <button
                          type="button"
                          className="secondary-button"
                          aria-label="Discover communities"
                          disabled={
                            !canValidate || sourceLocked || buzzCommunitiesLoading === source.name
                          }
                          onClick={() => void discoverBuzzCommunities(index, source)}
                        >
                          {buzzCommunitiesLoading === source.name ? (
                            <LoaderCircle className="spin" size={14} />
                          ) : (
                            <RefreshCw size={14} />
                          )}{' '}
                          Discover communities
                        </button>
                        {buzzCommunities[source.name] && (
                          <div className="source-repository-options">
                            {buzzCommunities[source.name].communities.length === 0 ? (
                              <small>No communities recorded in the identity file.</small>
                            ) : (
                              buzzCommunities[source.name].communities.map((community) => (
                                <label key={community.id}>
                                  <input
                                    type="checkbox"
                                    checked={source.communities.includes(community.id)}
                                    disabled={sourceLocked || !source.editable}
                                    onChange={() => toggleBuzzCommunity(index, source, community)}
                                  />
                                  <span>{community.name}</span>
                                </label>
                              ))
                            )}
                            {buzzCommunities[source.name].truncated && (
                              <small>
                                Buzz returned more than 100 communities; only the first 100 are
                                shown.
                              </small>
                            )}
                          </div>
                        )}
                      </div>
                    </Field>
                  )}
                  <Field label="Source label" hint="identifier stored on indexed documents">
                    <input
                      aria-label="Source label"
                      value={source.source || ''}
                      disabled={sourceLocked || !source.editable}
                      maxLength={128}
                      placeholder={source.name}
                      onChange={(event) =>
                        changeSource(index, { source: event.target.value || null })
                      }
                    />
                  </Field>
                  {source.kind === 'filesystem' && (
                    <Field label="Excluded paths" hint="comma or line separated, relative paths">
                      <input
                        value={source.exclude.join(', ')}
                        disabled={sourceLocked || !source.editable}
                        onChange={(event) =>
                          changeSource(index, { exclude: splitList(event.target.value) })
                        }
                      />
                    </Field>
                  )}
                  {isGoogleSource(source.kind) && (
                    <>
                      <Field
                        label="Google OAuth token file"
                        hint="private token created by Cortana; optional when a token path environment variable is configured"
                        wide
                      >
                        <div className="path-input">
                          <input
                            value={source.token_path || ''}
                            disabled={sourceLocked || !source.editable}
                            required={source.enabled && !source.token_env}
                            placeholder="/Users/you/.config/cortana/google-token.json"
                            onChange={(event) =>
                              changeSource(index, { token_path: event.target.value || null })
                            }
                          />
                          <button
                            type="button"
                            disabled={sourceLocked || !source.editable}
                            aria-label="Choose Google token destination"
                            title="Choose Google token destination"
                            data-tooltip="Choose Google token destination"
                            className="quick-tooltip"
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
                            disabled={sourceLocked || !source.editable}
                            placeholder="/Users/you/Downloads/google-oauth-client.json"
                            onChange={(event) =>
                              changeSource(index, {
                                oauth_client_path: event.target.value || null,
                              })
                            }
                          />
                          <button
                            type="button"
                            disabled={sourceLocked || !source.editable}
                            aria-label="Choose Google OAuth client JSON"
                            title="Choose Google OAuth client JSON"
                            data-tooltip="Choose Google OAuth client JSON"
                            className="quick-tooltip"
                            onClick={() =>
                              void choosePath(index, 'oauth-client', 'oauth_client_path')
                            }
                          >
                            <FolderOpen size={14} />
                          </button>
                        </div>
                      </Field>
                      <Field
                        label="Google token path environment variable"
                        hint="optional; its value must be an absolute OAuth token JSON path"
                      >
                        <input
                          value={source.token_env || ''}
                          disabled={sourceLocked || !source.editable}
                          pattern="[A-Z_][A-Z0-9_]*"
                          placeholder="CORTANA_GOOGLE_TOKEN_PATH"
                          onChange={(event) =>
                            changeSource(index, { token_env: event.target.value || null })
                          }
                        />
                      </Field>
                      <Field
                        label="Google token path value"
                        hint="write-only path; leave blank to keep the existing value"
                      >
                        <div className="secret-input">
                          <input
                            type="password"
                            autoComplete="new-password"
                            disabled={sourceLocked || !source.editable || !source.token_env}
                            value={source.token_env ? secretValues[source.token_env] || '' : ''}
                            onChange={(event) => {
                              if (source.token_env) {
                                onSecret({
                                  ...secretValues,
                                  [source.token_env]: event.target.value,
                                })
                              }
                            }}
                          />
                          {source.token_env &&
                            secret?.configured &&
                            !clearedSecrets.has(secret.name) && (
                              <button
                                type="button"
                                disabled={sourceLocked}
                                onClick={() => onClearSecret(source.token_env!)}
                              >
                                Clear
                              </button>
                            )}
                        </div>
                      </Field>
                      <Field label="Google query" hint="optional provider-native filter" wide>
                        <input
                          value={source.query || ''}
                          disabled={sourceLocked || !source.editable}
                          maxLength={2048}
                          placeholder={source.kind === 'gmail' ? 'newer_than:1y' : ''}
                          onChange={(event) =>
                            changeSource(index, { query: event.target.value || null })
                          }
                        />
                      </Field>
                    </>
                  )}
                  {source.kind === 'github' && (
                    <>
                      <Field
                        label="Repository chooser"
                        hint="discover accessible repositories, then select only the ones Cortana may index"
                        wide
                      >
                        <div className="source-repository-chooser">
                          <button
                            type="button"
                            className="secondary-button"
                            disabled={
                              !canValidate ||
                              sourceLocked ||
                              githubRepositoriesLoading === source.name
                            }
                            onClick={() => void discoverGithubRepositories(index, source)}
                          >
                            {githubRepositoriesLoading === source.name ? (
                              <LoaderCircle className="spin" size={14} />
                            ) : (
                              <RefreshCw size={14} />
                            )}{' '}
                            Discover repositories
                          </button>
                          {githubRepositories[source.name] && (
                            <div className="source-repository-options">
                              {githubRepositories[source.name].items.length === 0 ? (
                                <small>No accessible repositories returned.</small>
                              ) : (
                                githubRepositories[source.name].items.map((repository) => (
                                  <label key={repository.id}>
                                    <input
                                      type="checkbox"
                                      checked={source.repositories.includes(repository.full_name)}
                                      disabled={sourceLocked || !source.editable}
                                      onChange={() =>
                                        toggleGithubRepository(index, source, repository.full_name)
                                      }
                                    />
                                    <span>
                                      {repository.full_name}
                                      {repository.private ? ' · private' : ''}
                                    </span>
                                  </label>
                                ))
                              )}
                            </div>
                          )}
                        </div>
                      </Field>
                      <Field
                        label="GitHub OAuth token file"
                        hint="private token created by Cortana; use this for OAuth or leave blank for an environment token"
                        wide
                      >
                        <div className="path-input">
                          <input
                            value={source.token_path || ''}
                            disabled={sourceLocked || !source.editable}
                            required={source.enabled && !source.token_env}
                            placeholder="/Users/you/.config/cortana/github-token.json"
                            onChange={(event) =>
                              changeSource(index, { token_path: event.target.value || null })
                            }
                          />
                          <button
                            type="button"
                            disabled={sourceLocked || !source.editable}
                            aria-label="Choose GitHub token destination"
                            title="Choose GitHub token destination"
                            data-tooltip="Choose GitHub token destination"
                            className="quick-tooltip"
                            onClick={() => void choosePath(index, 'github-token', 'token_path')}
                          >
                            <FolderOpen size={14} />
                          </button>
                        </div>
                      </Field>
                    </>
                  )}
                  {(source.kind === 'github' ||
                    source.kind === 'slack' ||
                    source.kind === 'discord') && (
                    <>
                      {source.kind === 'discord' && (
                        <Field
                          label="Server chooser"
                          hint="assign the servers this workspace may index; authorize with Discord first, then discover and check the servers to assign"
                          wide
                        >
                          <div className="source-repository-chooser">
                            <button
                              type="button"
                              className="secondary-button"
                              aria-label="Discover servers"
                              disabled={
                                !canValidate ||
                                sourceLocked ||
                                discordServersLoading === source.name
                              }
                              onClick={() => void discoverDiscordServers(index, source)}
                            >
                              {discordServersLoading === source.name ? (
                                <LoaderCircle className="spin" size={14} />
                              ) : (
                                <RefreshCw size={14} />
                              )}{' '}
                              Discover servers
                            </button>
                            {discordServers[source.name] && (
                              <div className="source-repository-options">
                                {discordServers[source.name].guilds.length === 0 ? (
                                  <small>No accessible servers returned.</small>
                                ) : (
                                  discordServers[source.name].guilds.map((guild) => (
                                    <label key={guild.id}>
                                      <input
                                        type="checkbox"
                                        checked={source.servers.includes(guild.id)}
                                        disabled={sourceLocked || !source.editable}
                                        onChange={() =>
                                          toggleDiscordServer(index, source, guild.id)
                                        }
                                      />
                                      <span>{guild.name}</span>
                                    </label>
                                  ))
                                )}
                                {discordServers[source.name].truncated && (
                                  <small>
                                    Discord returned more than 100 servers; only the first 100 are
                                    shown.
                                  </small>
                                )}
                              </div>
                            )}
                          </div>
                        </Field>
                      )}
                      {source.kind === 'discord' && (
                        <Field
                          label="Channel chooser"
                          hint="discover channels with the bot token, then select only the channels Cortana may index; channels outside assigned servers stay available when no servers are assigned"
                          wide
                        >
                          <div className="source-repository-chooser">
                            <button
                              type="button"
                              className="secondary-button"
                              aria-label="Discover channels"
                              disabled={
                                !canValidate ||
                                sourceLocked ||
                                discordChannelsLoading === source.name
                              }
                              onClick={() => void discoverDiscordChannels(index, source)}
                            >
                              {discordChannelsLoading === source.name ? (
                                <LoaderCircle className="spin" size={14} />
                              ) : (
                                <RefreshCw size={14} />
                              )}{' '}
                              Discover channels
                            </button>
                            {discordChannels[source.name] && (
                              <div className="source-repository-options">
                                {discordChannels[source.name].guilds.length === 0 ? (
                                  <small>No accessible servers returned.</small>
                                ) : (
                                  discordChannels[source.name].guilds.map((guild) => {
                                    const serversAssigned = source.servers.length > 0
                                    const assigned =
                                      !serversAssigned || source.servers.includes(guild.id)
                                    return (
                                      <div
                                        key={guild.id}
                                        className={`discord-guild${assigned ? '' : ' discord-guild-unassigned'}`}
                                      >
                                        <strong>{guild.name}</strong>
                                        {!assigned && (
                                          <small> · not assigned to this workspace</small>
                                        )}
                                        {guild.truncated && <small> · first 100 channels</small>}
                                        {guild.channels.length === 0 ? (
                                          <small>No channels returned.</small>
                                        ) : (
                                          guild.channels.map((channel) => (
                                            <label
                                              key={channel.id}
                                              title={
                                                assigned
                                                  ? undefined
                                                  : 'Assign this server in the server chooser before selecting its channels'
                                              }
                                            >
                                              <input
                                                type="checkbox"
                                                checked={source.channels.includes(channel.id)}
                                                disabled={sourceLocked || !source.editable}
                                                onChange={() =>
                                                  toggleDiscordChannel(index, source, channel.id)
                                                }
                                              />
                                              <span>
                                                {channel.name} · {channel.kind}
                                              </span>
                                            </label>
                                          ))
                                        )}
                                      </div>
                                    )
                                  })
                                )}
                                {discordChannels[source.name].truncated && (
                                  <small>
                                    Discord returned more than 100 servers; only the first 100 are
                                    shown.
                                  </small>
                                )}
                              </div>
                            )}
                          </div>
                        </Field>
                      )}
                      <Field
                        label={source.kind === 'github' ? 'Repositories' : 'Channel IDs'}
                        hint={
                          source.kind === 'github'
                            ? 'one owner/repository per line; only these repositories are indexed'
                            : 'comma or line separated'
                        }
                        wide
                      >
                        <textarea
                          value={
                            source.kind === 'github'
                              ? source.repositories.join('\n')
                              : source.channels.join(', ')
                          }
                          disabled={sourceLocked || !source.editable}
                          required={source.enabled}
                          rows={source.kind === 'github' ? 3 : 1}
                          placeholder={
                            source.kind === 'github' ? 'owner/repository' : 'Channel IDs'
                          }
                          aria-label={
                            source.kind === 'github' ? 'GitHub repositories' : 'Channel IDs'
                          }
                          onChange={(event) => {
                            const values = splitList(event.target.value)
                            changeSource(
                              index,
                              source.kind === 'github'
                                ? { repositories: values }
                                : { channels: values }
                            )
                          }}
                        />
                      </Field>
                      <Field
                        label="Token variable"
                        hint={
                          source.kind === 'discord'
                            ? 'Discord bot token; required for channel listing and sync'
                            : secret?.configured && !clearedSecrets.has(secret.name)
                              ? `Configured via ${secret.source}`
                              : 'stored in Cortana owner-only secret file'
                        }
                      >
                        <input
                          value={source.token_env || ''}
                          disabled={sourceLocked || !source.editable}
                          required={
                            source.enabled &&
                            (source.kind === 'discord' ||
                              (source.kind !== 'github' && !source.token_path))
                          }
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
                            disabled={sourceLocked || !source.editable || !source.token_env}
                            value={source.token_env ? secretValues[source.token_env] || '' : ''}
                            onChange={(event) => {
                              if (source.token_env) {
                                onSecret({
                                  ...secretValues,
                                  [source.token_env]: event.target.value,
                                })
                              }
                            }}
                          />
                          {source.token_env &&
                            secret?.configured &&
                            !clearedSecrets.has(secret.name) && (
                              <button
                                type="button"
                                disabled={sourceLocked}
                                onClick={() => onClearSecret(source.token_env!)}
                              >
                                Clear
                              </button>
                            )}
                        </div>
                      </Field>
                      {source.kind === 'discord' && (
                        <>
                          <Field
                            label="Discord OAuth token file"
                            hint="private user token created by Cortana; used only to list servers for workspace assignment"
                            wide
                          >
                            <div className="path-input">
                              <input
                                value={source.token_path || ''}
                                disabled={sourceLocked || !source.editable}
                                placeholder="/Users/you/.config/cortana/discord-user-token.json"
                                onChange={(event) =>
                                  changeSource(index, { token_path: event.target.value || null })
                                }
                              />
                              <button
                                type="button"
                                disabled={sourceLocked || !source.editable}
                                aria-label="Choose Discord OAuth token destination"
                                title="Choose Discord OAuth token destination"
                                data-tooltip="Choose Discord OAuth token destination"
                                className="quick-tooltip"
                                onClick={() =>
                                  void choosePath(index, 'discord-token', 'token_path')
                                }
                              >
                                <FolderOpen size={14} />
                              </button>
                            </div>
                          </Field>
                          <Field
                            label="Discord OAuth client JSON"
                            hint="JSON containing the OAuth app client_id; required for browser authorization"
                            wide
                          >
                            <div className="path-input">
                              <input
                                value={source.oauth_client_path || ''}
                                disabled={sourceLocked || !source.editable}
                                placeholder="/Users/you/.config/cortana/discord-oauth-client.json"
                                onChange={(event) =>
                                  changeSource(index, {
                                    oauth_client_path: event.target.value || null,
                                  })
                                }
                              />
                              <button
                                type="button"
                                disabled={sourceLocked || !source.editable}
                                aria-label="Choose Discord OAuth client JSON"
                                title="Choose Discord OAuth client JSON"
                                data-tooltip="Choose Discord OAuth client JSON"
                                className="quick-tooltip"
                                onClick={() =>
                                  void choosePath(index, 'oauth-client', 'oauth_client_path')
                                }
                              >
                                <FolderOpen size={14} />
                              </button>
                            </div>
                          </Field>
                        </>
                      )}
                      {source.kind === 'slack' && (
                        <>
                          <Field
                            label="Workspace chooser"
                            hint="assign the workspace this source may index; authorize with Slack first, then discover and check the workspace to assign. A Slack user token is scoped to exactly one workspace, so at most one team can be assigned per source"
                            wide
                          >
                            <div className="source-repository-chooser">
                              <button
                                type="button"
                                className="secondary-button"
                                aria-label="Discover workspaces"
                                disabled={
                                  !canValidate ||
                                  sourceLocked ||
                                  slackWorkspacesLoading === source.name
                                }
                                onClick={() => void discoverSlackWorkspaces(index, source)}
                              >
                                {slackWorkspacesLoading === source.name ? (
                                  <LoaderCircle className="spin" size={14} />
                                ) : (
                                  <RefreshCw size={14} />
                                )}{' '}
                                Discover workspaces
                              </button>
                              {slackWorkspaces[source.name] && (
                                <div className="source-repository-options">
                                  {slackWorkspaces[source.name].teams.length === 0 ? (
                                    <small>No accessible workspaces returned.</small>
                                  ) : (
                                    slackWorkspaces[source.name].teams.map((team) => (
                                      <label key={team.id}>
                                        <input
                                          type="checkbox"
                                          checked={source.teams.includes(team.id)}
                                          disabled={sourceLocked || !source.editable}
                                          onChange={() => toggleSlackTeam(index, source, team)}
                                        />
                                        <span>{team.name}</span>
                                      </label>
                                    ))
                                  )}
                                  {slackWorkspaces[source.name].truncated && (
                                    <small>
                                      Slack returned more than 100 teams; only the first 100 are
                                      shown.
                                    </small>
                                  )}
                                </div>
                              )}
                            </div>
                          </Field>
                          <Field
                            label="Slack OAuth token file"
                            hint="private user token created by Cortana; used only to list the workspace for assignment. The SLACK_BOT_TOKEN environment variable is separate and stays the message-sync credential"
                            wide
                          >
                            <div className="path-input">
                              <input
                                value={source.token_path || ''}
                                disabled={sourceLocked || !source.editable}
                                placeholder="/Users/you/.config/cortana/slack-user-token.json"
                                onChange={(event) =>
                                  changeSource(index, { token_path: event.target.value || null })
                                }
                              />
                              <button
                                type="button"
                                disabled={sourceLocked || !source.editable}
                                aria-label="Choose Slack OAuth token destination"
                                title="Choose Slack OAuth token destination"
                                data-tooltip="Choose Slack OAuth token destination"
                                className="quick-tooltip"
                                onClick={() => void choosePath(index, 'slack-token', 'token_path')}
                              >
                                <FolderOpen size={14} />
                              </button>
                            </div>
                          </Field>
                          <Field
                            label="Slack OAuth client JSON"
                            hint="JSON containing the OAuth app client_id; required for browser authorization. Register the loopback redirect URI http://127.0.0.1:47521/callback in the Slack app first"
                            wide
                          >
                            <div className="path-input">
                              <input
                                value={source.oauth_client_path || ''}
                                disabled={sourceLocked || !source.editable}
                                placeholder="/Users/you/.config/cortana/slack-oauth-client.json"
                                onChange={(event) =>
                                  changeSource(index, {
                                    oauth_client_path: event.target.value || null,
                                  })
                                }
                              />
                              <button
                                type="button"
                                disabled={sourceLocked || !source.editable}
                                aria-label="Choose Slack OAuth client JSON"
                                title="Choose Slack OAuth client JSON"
                                data-tooltip="Choose Slack OAuth client JSON"
                                className="quick-tooltip"
                                onClick={() =>
                                  void choosePath(index, 'oauth-client', 'oauth_client_path')
                                }
                              >
                                <FolderOpen size={14} />
                              </button>
                            </div>
                          </Field>
                        </>
                      )}
                    </>
                  )}
                  {source.editable && (
                    <>
                      <Field label="Document limit" hint="blank uses global budget">
                        <input
                          type="number"
                          disabled={sourceLocked}
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
                          disabled={sourceLocked}
                          min={1024}
                          max={1099511627776}
                          value={source.max_bytes ?? ''}
                          onChange={(event) =>
                            changeSource(index, { max_bytes: optionalNumber(event.target.value) })
                          }
                        />
                      </Field>
                      <Field
                        label="Content limit (characters)"
                        hint="blank uses connector defaults"
                      >
                        <input
                          type="number"
                          disabled={sourceLocked}
                          min={1}
                          max={10000000}
                          value={source.max_content_chars ?? ''}
                          onChange={(event) =>
                            changeSource(index, {
                              max_content_chars: optionalNumber(event.target.value),
                            })
                          }
                        />
                      </Field>
                      <Field label="Duration limit (seconds)" hint="blank uses the global budget">
                        <input
                          type="number"
                          disabled={sourceLocked}
                          min={1}
                          max={86400}
                          value={source.max_duration_seconds ?? ''}
                          onChange={(event) =>
                            changeSource(index, {
                              max_duration_seconds: optionalNumber(event.target.value),
                            })
                          }
                        />
                      </Field>
                      <Field label="Document labels" hint="comma or line separated" wide>
                        <input
                          disabled={sourceLocked}
                          value={source.labels.join(', ')}
                          onChange={(event) =>
                            changeSource(index, { labels: splitList(event.target.value) })
                          }
                        />
                      </Field>
                      <Field
                        label="Document ACL labels"
                        hint="comma or line separated; leave blank only for public data"
                        wide
                      >
                        <input
                          disabled={sourceLocked}
                          value={source.acl.join(', ')}
                          onChange={(event) =>
                            changeSource(index, { acl: splitList(event.target.value) })
                          }
                        />
                      </Field>
                    </>
                  )}
                </div>
              </details>
            </article>
          )
        })}
      </div>

      {initialSync && initialSyncSource && (
        <InitialSyncFlow
          source={initialSyncSource}
          flow={initialSync}
          busy={Boolean(activeJob) || !canValidate}
          onBudget={(budget) => void requestPlan(initialSync.source, budget)}
          onValidate={() => void validateInitialSyncBudget(sourceOf(settings, initialSync.source))}
          onStart={() => void startInitialSync(sourceOf(settings, initialSync.source))}
          onClose={() => setInitialSync(null)}
        />
      )}

      {error && (
        <div className="safety-note" role="alert">
          {error}
        </div>
      )}
      {observedJob && (
        <div className={`source-validation-job ${observedJob.status}`}>
          <div>
            <StatusGlyph
              passed={observedJob.status === 'succeeded'}
              optional={observedJob.status === 'cancelled'}
            />
            <span>
              <strong>
                {observedJob.source} · {observedJob.operation} · {observedJob.status}
              </strong>
              <small>{observedJob.summary}</small>
            </span>
            {['running', 'cancelling'].includes(observedJob.status) && (
              <button
                type="button"
                disabled={observedJob.status === 'cancelling'}
                onClick={() => void cancel()}
              >
                <CircleStop size={14} /> Cancel
              </button>
            )}
            {observedJob.retryable && (
              <button
                type="button"
                disabled={!canValidate || Boolean(activeJob)}
                onClick={() => {
                  const source = settings.sources.find((item) => item.name === observedJob.source)
                  if (source) {
                    if (observedJob.operation === 'authorization') void authorizeSource(source)
                    else if (observedJob.operation === 'trial-sync') void trialSyncSource(source)
                    else if (observedJob.operation === 'initial-sync') {
                      void openInitialSync(
                        source,
                        (observedJob.budget as InitialSyncBudget | null) || 'small'
                      )
                    } else void validateSource(source)
                  }
                }}
              >
                <RefreshCw size={14} /> Retry
              </button>
            )}
          </div>
          {observedJob.log && <pre>{observedJob.log}</pre>}
        </div>
      )}

      <div className="safety-note">
        <AlertTriangle size={16} />
        <span>
          Source validation checks a bounded snapshot and writes only metadata about the outcome.
          Trial sync is separately confirmed, requires an exact successful validation, limits work
          to 25 documents and 5 MiB, and never performs deletion reconciliation. Initial sync is
          planned first, uses one of three fixed budgets (up to 2,000 documents, 128 MiB, 60
          minutes), requires validation at equal or larger limits, and never escalates beyond the
          selected budget.
        </span>
      </div>
    </SettingsSection>
  )
}

function sourceOf(settings: DesktopSettings, name: string): SourceSettings {
  return settings.sources.find((item) => item.name === name)!
}

function budgetLabel(budget: InitialSyncBudget) {
  const tier = INITIAL_SYNC_BUDGETS.find((item) => item.budget === budget)
  return tier
    ? `${tier.documents.toLocaleString()} documents or ${mebibytes(tier.bytes)} MiB for up to ${minutes(tier.seconds)} minutes`
    : budget
}

function mebibytes(bytes: number) {
  return Math.round(bytes / 1048576)
}

function minutes(seconds: number) {
  return Math.round(seconds / 60)
}

function InitialSyncFlow({
  source,
  flow,
  busy,
  onBudget,
  onValidate,
  onStart,
  onClose,
}: {
  source: SourceSettings
  flow: {
    source: string
    budget: InitialSyncBudget
    plan: DesktopInitialSyncPlan | null
    planning: boolean
    flowError: string
  }
  busy: boolean
  onBudget: (budget: InitialSyncBudget) => void
  onValidate: () => void
  onStart: () => void
  onClose: () => void
}) {
  const plan = flow.plan
  return (
    <section className="initial-sync-flow" aria-label={`Initial sync plan for ${source.name}`}>
      <header>
        <div>
          <span className="eyebrow">Guided initial sync</span>
          <strong>{source.name}</strong>
        </div>
        <button
          type="button"
          aria-label="Close initial sync plan"
          title="Close initial sync plan"
          data-tooltip="Close initial sync plan"
          className="quick-tooltip"
          onClick={onClose}
        >
          <X size={15} />
        </button>
      </header>
      <div className="initial-sync-budgets" role="radiogroup" aria-label="Initial sync budget">
        {INITIAL_SYNC_BUDGETS.map((tier) => (
          <label key={tier.budget} className={flow.budget === tier.budget ? 'selected' : ''}>
            <input
              type="radio"
              name="initial-sync-budget"
              value={tier.budget}
              checked={flow.budget === tier.budget}
              disabled={flow.planning || busy}
              onChange={() => onBudget(tier.budget)}
            />
            <span>
              <strong>{tier.budget[0].toUpperCase() + tier.budget.slice(1)}</strong>
              <small>{budgetLabel(tier.budget)}</small>
            </span>
          </label>
        ))}
      </div>
      {flow.planning && <p className="initial-sync-state">Requesting a native plan…</p>}
      {flow.flowError && (
        <div className="safety-note" role="alert">
          {flow.flowError}
        </div>
      )}
      {plan && (
        <>
          <dl className="initial-sync-plan">
            <div>
              <dt>Source</dt>
              <dd>
                {plan.source} · {plan.kind} · {plan.project}
              </dd>
            </div>
            <div>
              <dt>Selected budget</dt>
              <dd>
                {plan.budget_documents.toLocaleString()} documents · {mebibytes(plan.budget_bytes)}{' '}
                MiB · {minutes(plan.budget_seconds)} minutes
              </dd>
            </div>
            <div>
              <dt>Validation gate</dt>
              <dd>{plan.requires_validation ? 'Required at equal or larger limits' : 'None'}</dd>
            </div>
            <div>
              <dt>Deletion reconciliation</dt>
              <dd>Disabled</dd>
            </div>
            <div>
              <dt>Writes indexed data</dt>
              <dd>Yes — committed batches become searchable</dd>
            </div>
          </dl>
          {!plan.enabled && (
            <div className="safety-note">
              <AlertTriangle size={16} />
              <span>Enable this source and save before an initial sync.</span>
            </div>
          )}
          {plan.validation_covers_budget !== true && (
            <div className="safety-note">
              <AlertTriangle size={16} />
              <span>
                {plan.validation_covers_budget === false
                  ? 'The latest validation used smaller limits. Run a read-only validation with this budget before syncing.'
                  : 'This source has no validation record. Run a read-only validation with this budget before syncing.'}
              </span>
              {!busy && (
                <button type="button" onClick={onValidate}>
                  Validate for this budget
                </button>
              )}
            </div>
          )}
          <div className="initial-sync-actions">
            <button
              type="button"
              className="primary-button"
              disabled={
                !plan.enabled || plan.validation_covers_budget !== true || busy || flow.planning
              }
              onClick={onStart}
            >
              <Zap size={15} /> Start initial sync
            </button>
            <span>
              Execution requires explicit confirmation and reuses the native validation-gated,
              no-reconcile source-job boundary.
            </span>
          </div>
        </>
      )}
    </section>
  )
}

function newSource(settings: DesktopSettings, project?: string): SourceSettings {
  return {
    name: nextAvailableIdentifier(
      'source',
      settings.sources.map((source) => source.name)
    ),
    kind: 'filesystem',
    enabled: false,
    project: project || settings.workspaces[0]?.id || 'personal',
    root: null,
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
}

function nextAvailableIdentifier(prefix: string, used: readonly string[]): string {
  const occupied = new Set(used)
  for (let number = 1; ; number += 1) {
    const candidate = `${prefix}-${number}`
    if (!occupied.has(candidate)) return candidate
  }
}

function deriveWorkspaceIdentifier(name: string): string {
  const normalized = name
    .trim()
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^\w\s-]/g, '')
    .replace(/[\s_]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-+|-+$/g, '')

  if (!normalized) return 'workspace'

  const candidate = normalized.slice(0, 32).replace(/-[0-9]+$/, '')
  if (/^[a-z0-9][a-z0-9-]*$/.test(candidate)) return candidate
  return `workspace-${normalized}`.slice(0, 32)
}

function ensureWorkspaceIdentifierUnique(base: string, used: readonly string[]): string {
  const trimmed = base.trim() || 'workspace'
  const occupied = new Set(used)
  if (!occupied.has(trimmed) && isWorkspaceIdentifierSafe(trimmed)) return trimmed

  let counter = 1
  while (occupied.has(`${trimmed}-${counter}`)) counter += 1
  return `${trimmed}-${counter}`
}

function isWorkspaceIdentifierSafe(value: string) {
  return /^[a-z0-9][a-z0-9_-]*$/.test(value)
}

function isWorkspaceIdDerivedFromName(workspace: WorkspaceSettings) {
  return (
    isWorkspaceIdentifierSafe(workspace.id) &&
    workspace.id === deriveWorkspaceIdentifier(workspace.name)
  )
}

function SafeMarkdown({ text }: { text: string }) {
  return <div className="safe-markdown">{renderMarkdownToNodes(text)}</div>
}

function renderMarkdownToNodes(text: string): ReactNode[] {
  const nodes: ReactNode[] = []
  const lines = text.replace(/\r\n/g, '\n').split('\n')
  let currentList: { ordered: boolean; items: string[] } | null = null

  const closeList = () => {
    if (!currentList) return
    const key = nodes.length
    if (currentList.ordered) {
      nodes.push(
        <ol key={`list-${key}`}>
          {currentList.items.map((item, index) => (
            <li key={`${key}-${index}`}>{parseInlineMarkdown(item)}</li>
          ))}
        </ol>
      )
    } else {
      nodes.push(
        <ul key={`list-${key}`}>
          {currentList.items.map((item, index) => (
            <li key={`${key}-${index}`}>{parseInlineMarkdown(item)}</li>
          ))}
        </ul>
      )
    }
    currentList = null
  }

  for (const line of lines) {
    const trimmed = line.trimEnd()
    const heading = trimmed.match(/^(#{1,6})\s+(.+)$/)
    const bullet = trimmed.match(/^[-*]\s+(.+)$/)
    const ordered = trimmed.match(/^\d+\.\s+(.+)$/)

    if (!trimmed) {
      closeList()
      continue
    }

    if (heading) {
      closeList()
      const level = heading[1].length
      const title = parseInlineMarkdown(heading[2])
      if (level === 1) nodes.push(<h1 key={`h-${nodes.length}`}>{title}</h1>)
      else if (level === 2) nodes.push(<h2 key={`h-${nodes.length}`}>{title}</h2>)
      else nodes.push(<h3 key={`h-${nodes.length}`}>{title}</h3>)
      continue
    }

    if (bullet) {
      if (!currentList || currentList.ordered) {
        closeList()
        currentList = { ordered: false, items: [] }
      }
      currentList.items.push(bullet[1])
      continue
    }

    if (ordered) {
      if (!currentList || !currentList.ordered) {
        closeList()
        currentList = { ordered: true, items: [] }
      }
      currentList.items.push(ordered[1])
      continue
    }

    closeList()
    nodes.push(<p key={`p-${nodes.length}`}>{parseInlineMarkdown(trimmed)}</p>)
  }

  closeList()
  return nodes
}

function parseInlineMarkdown(value: string): ReactNode[] {
  const parts = value.split(/(`[^`]*`|\[[^\]]+\]\([^)]+\))/g)
  const nodes: ReactNode[] = []

  for (const [index, part] of parts.entries()) {
    if (!part) continue
    if (part.startsWith('`') && part.endsWith('`')) {
      nodes.push(<code key={`code-${index}`}>{part.slice(1, -1)}</code>)
      continue
    }

    const link = part.match(/^\[([^\]]+)\]\(([^)]+)\)$/)
    if (link) {
      const url = safeMarkdownUrl(link[2])
      if (url) {
        nodes.push(
          <a key={`link-${index}`} href={url} target="_blank" rel="noreferrer">
            {link[1]}
          </a>
        )
      } else {
        nodes.push(<span key={`text-${index}`}>{part}</span>)
      }
      continue
    }

    nodes.push(<span key={`text-${index}`}>{part}</span>)
  }
  return nodes
}

function safeMarkdownUrl(value: string): string | null {
  try {
    const candidate = new URL(value)
    if (candidate.protocol === 'http:' || candidate.protocol === 'https:') return candidate.href
    return null
  } catch {
    return null
  }
}

function defaultTokenEnv(kind: SourceKind): string | null {
  if (kind === 'github') return 'GITHUB_TOKEN'
  if (kind === 'slack') return 'SLACK_BOT_TOKEN'
  if (kind === 'discord') return 'DISCORD_BOT_TOKEN'
  return null
}

function isGoogleSource(kind: SourceKind) {
  return ['google-drive', 'gmail', 'google-calendar'].includes(kind)
}

function hasBrowserSetup(kind: SourceKind) {
  return isGoogleSource(kind) || kind === 'github' || kind === 'slack' || kind === 'discord'
}

function splitList(value: string) {
  return value
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean)
}

function optionalNumber(value: string): number | null {
  if (value === '') return null
  const number = Number(value)
  return Number.isFinite(number) ? number : null
}

function referencedSecretNames(settings: DesktopSettings): Set<string> {
  const names = new Set<string>()
  for (const name of [
    settings.embedding.api_key_env,
    settings.query.api_key_env,
    settings.hindsight.token_env,
    settings.honcho.token_env,
  ]) {
    if (name) names.add(name)
  }
  settings.sources.forEach((source) => {
    if (source.token_env) names.add(source.token_env)
  })
  settings.auth_principals.forEach((principal) => names.add(principal.token_env))
  return names
}

function validateSourceIdentityScopes(sources: readonly SourceSettings[]): string | null {
  const seen = new Map<string, string>()
  for (const source of sources) {
    const configured = source.source
    if (
      configured !== null &&
      (configured.trim().length === 0 ||
        configured !== configured.trim() ||
        Array.from(configured).some((character) => {
          const codePoint = character.codePointAt(0) ?? 0
          return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)
        }))
    ) {
      return `Source label for \`${source.name}\` must not be empty, padded with whitespace, or contain control characters.`
    }
    const canonical = configured ?? source.name.trim()
    const scope = `${source.project}\u0000${canonical}`
    const previous = seen.get(scope)
    if (previous) {
      return `Source identifier \`${canonical}\` is duplicated in workspace \`${source.project}\` (\`${previous}\` and \`${source.name}\`). Choose a unique source label before saving.`
    }
    seen.set(scope, source.name)
  }
  return null
}

type ProviderValue = DesktopSettings['embedding'] | DesktopSettings['query']

type ModelChoice = {
  value: string
  label: string
}

/** Provider-advertised catalog captured for one provider kind. */
type ProviderModelsState = {
  kind: ProviderModelKind
  /** Normalized base URL the catalog was fetched from. */
  provider: string
  mode: 'local' | 'cloud'
  key_env: string | null
  models: ProviderModelEntry[]
  truncated: boolean
}

function normalizeProviderUrl(value: string): string {
  return value.trim().replace(/\/+$/, '')
}

function EmbeddingSection({
  settings,
  secretValues,
  onSecret,
  clearedSecrets,
  onClearSecret,
  update,
  advertisedModels,
  modelsLoading,
  modelsError,
  modelsTruncated,
  onRefreshModels,
}: {
  settings: DesktopSettings
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
  advertisedModels: readonly ModelChoice[] | null
  modelsLoading: boolean
  modelsError: string
  modelsTruncated: boolean
  onRefreshModels: () => void
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
      advertisedModels={advertisedModels}
      modelsLoading={modelsLoading}
      modelsError={modelsError}
      modelsTruncated={modelsTruncated}
      onRefreshModels={onRefreshModels}
      modelCatalog={
        settings.embedding.provider === 'local'
          ? [
              { value: 'Qwen/Qwen3-Embedding-0.6B', label: 'Qwen/Qwen3-Embedding-0.6B' },
              { value: 'Qwen/Qwen3-Embedding-4B', label: 'Qwen/Qwen3-Embedding-4B' },
            ]
          : [
              { value: 'gpt-4o-mini', label: 'gpt-4o-mini' },
              { value: 'gpt-4o', label: 'gpt-4o' },
              { value: 'text-embedding-3-small', label: 'text-embedding-3-small' },
            ]
      }
    >
      <div className="settings-note">
        <strong>Local service command:</strong>{' '}
        {settings.embedding_service_program
          ? `${settings.embedding_service_program} (managed in config.toml)`
          : 'automatic command derived from the model and loopback endpoint'}
        . Desktop preserves explicit executable commands but does not edit shell command arrays.
      </div>
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
          hint="0 disables new embedding-cache writes"
          value={settings.embedding.cache_max_entries}
          min={0}
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
  modelCatalog,
  advertisedModels,
  modelsLoading,
  modelsError,
  modelsTruncated,
  onRefreshModels,
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
  modelCatalog: readonly ModelChoice[]
  /** Provider-advertised models; null when unavailable or stale. */
  advertisedModels: readonly ModelChoice[] | null
  modelsLoading: boolean
  modelsError: string
  modelsTruncated: boolean
  onRefreshModels: () => void
  update: (provider: T) => void
  children?: ReactNode
}) {
  const secret = provider.api_key_env
    ? secrets.find((item) => item.name === provider.api_key_env)
    : undefined
  // Provider-advertised models take precedence only while available; the
  // static catalog (local Qwen defaults, cloud presets) remains the safe
  // fallback whenever discovery is unavailable or the endpoint changed.
  const activeCatalog =
    advertisedModels && advertisedModels.length > 0 ? advertisedModels : modelCatalog
  const catalogValues = activeCatalog.map((candidate) => candidate.value)
  // The select mode is derived from the active catalog so a provider refresh
  // can never leave a stale select visible: when the current model is not in
  // the active catalog the custom input is shown with the value preserved.
  // The explicit override remembers only a user's own catalog/custom choice
  // and is cleared whenever the available catalog changes.
  const [explicitModelMode, setExplicitModelMode] = useState<'catalog' | 'custom' | null>(null)
  const modelMode: 'catalog' | 'custom' =
    explicitModelMode ?? (catalogValues.includes(provider.model) ? 'catalog' : 'custom')

  useEffect(() => {
    setExplicitModelMode(null)
  }, [catalogValues.join('\u0000')])

  const modelInput = (
    <Field label="Model">
      <input
        aria-label="Model"
        value={provider.model}
        onChange={(event) => update({ ...provider, model: event.target.value })}
        required
        maxLength={256}
      />
    </Field>
  )

  const modelSelect = (
    <Field label="Model">
      <select
        aria-label="Model catalog"
        value={modelMode === 'catalog' ? provider.model : 'custom'}
        onChange={(event) => {
          const selected = event.target.value
          if (selected === 'custom') {
            setExplicitModelMode('custom')
            return
          }
          if (catalogValues.includes(selected)) {
            setExplicitModelMode('catalog')
            update({ ...provider, model: selected })
          } else {
            // Unmatched selections (programmatic or label-based) open the
            // custom field with the current model preserved.
            setExplicitModelMode('custom')
          }
        }}
      >
        {activeCatalog.map((candidate) => (
          <option key={candidate.value} value={candidate.value}>
            {candidate.label}
          </option>
        ))}
        <option value="custom">Custom</option>
      </select>
    </Field>
  )

  const modelControls = (
    <div className="model-field">
      {modelMode === 'custom' ? modelInput : modelSelect}
      <div className="model-refresh">
        <button
          type="button"
          className="secondary-button"
          aria-label={`Refresh ${title} models from provider`}
          disabled={modelsLoading}
          onClick={onRefreshModels}
        >
          {modelsLoading ? <LoaderCircle className="spin" size={14} /> : <RefreshCw size={14} />}{' '}
          Refresh models
        </button>
      </div>
      {modelsError && <p className="settings-inline-error">{modelsError}</p>}
      {advertisedModels && advertisedModels.length > 0 && (
        <small className="model-note">
          {advertisedModels.length} model{advertisedModels.length === 1 ? '' : 's'} advertised by
          the provider{modelsTruncated ? ' (first 512 shown)' : ''}. A current model that is not
          advertised stays selected in the custom field.
        </small>
      )}
    </div>
  )

  return (
    <SettingsSection title={title} description={description}>
      <div className="form-grid">
        <Field label="Provider">
          <select
            value={provider.provider}
            onChange={(event) => {
              const nextProvider = event.target.value as 'local' | 'cloud'
              const loopback = isLoopbackUrl(provider.base_url)
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
        {modelControls}
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
  advertisedModels,
  modelsLoading,
  modelsError,
  modelsTruncated,
  onRefreshModels,
}: {
  settings: DesktopSettings
  secrets: DesktopSettings['secrets']
  secretValues: Record<string, string>
  onSecret: (values: Record<string, string>) => void
  clearedSecrets: Set<string>
  onClearSecret: (name: string) => void
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
  advertisedModels: readonly ModelChoice[] | null
  modelsLoading: boolean
  modelsError: string
  modelsTruncated: boolean
  onRefreshModels: () => void
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
      advertisedModels={advertisedModels}
      modelsLoading={modelsLoading}
      modelsError={modelsError}
      modelsTruncated={modelsTruncated}
      onRefreshModels={onRefreshModels}
      modelCatalog={
        settings.query.provider === 'local'
          ? [
              { value: 'qwen2.5-72b-instruct', label: 'qwen2.5-72b-instruct' },
              { value: 'gemma2-27b-it', label: 'gemma2-27b-it' },
            ]
          : [
              { value: 'gpt-4o-mini', label: 'gpt-4o-mini' },
              { value: 'gpt-4o', label: 'gpt-4o' },
              { value: 'claude-3-5-sonnet-20241022', label: 'claude-3.5-sonnet' },
              { value: 'gemini-1.5-flash', label: 'gemini-1.5-flash' },
            ]
      }
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
          hint="0 disables new answer-cache writes"
          value={settings.query.cache_max_entries}
          min={0}
          max={1000000}
          onChange={(cache_max_entries) => setQuery({ ...settings.query, cache_max_entries })}
        />
        <NumberField
          label="Cache lifetime (seconds)"
          hint="0 disables answer-cache reads"
          value={settings.query.cache_ttl_seconds}
          min={0}
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
        <NumberField
          label="Sync freshness (hours)"
          hint="0 disables stale-sync warnings in the source health view"
          value={settings.ingestion.sync_freshness_hours}
          min={0}
          max={8760}
          onChange={(sync_freshness_hours) => setIngestion({ sync_freshness_hours })}
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

function AdvancedSection({ settings, update, dirty }: SettingsSectionProps & { dirty: boolean }) {
  const [portableBusy, setPortableBusy] = useState<'export' | 'import' | 'open-secret' | ''>('')
  const [portableNotice, setPortableNotice] = useState('')
  const [portableError, setPortableError] = useState('')
  const setRuntime = (patch: Partial<DesktopSettings['runtime']>) =>
    update((current) => ({ ...current, runtime: { ...current.runtime, ...patch } }))

  const exportSettings = async () => {
    setPortableBusy('export')
    setPortableNotice('')
    setPortableError('')
    try {
      const result = await exportDesktopSettings()
      if (!result) return
      const omitted = result.omitted_external_sources.length
        ? ` Executable connectors omitted: ${result.omitted_external_sources.join(', ')}.`
        : ''
      setPortableNotice(`Redacted settings exported to ${result.path}.${omitted}`)
    } catch (caught) {
      setPortableError(caught instanceof Error ? caught.message : 'Settings export failed')
    } finally {
      setPortableBusy('')
    }
  }

  const importSettings = async () => {
    setPortableBusy('import')
    setPortableNotice('')
    setPortableError('')
    try {
      const result = await importDesktopSettings()
      if (!result) return
      if (
        !window.confirm(
          `Load the validated settings from ${result.path} into this form?\n\nSecret values are never imported. Existing executable connectors are preserved. Saving a changed principal list may remove credentials for principals you remove. Nothing is written until you choose Save changes.`
        )
      ) {
        return
      }
      update((current) => ({ ...current, ...result.settings }))
      const preserved = result.preserved_external_sources.length
        ? ` Preserved executable connectors: ${result.preserved_external_sources.join(', ')}.`
        : ''
      setPortableNotice(`Imported settings are ready for review.${preserved}`)
    } catch (caught) {
      setPortableError(caught instanceof Error ? caught.message : 'Settings import failed')
    } finally {
      setPortableBusy('')
    }
  }

  const openSecretFile = async () => {
    setPortableBusy('open-secret')
    setPortableNotice('')
    setPortableError('')
    try {
      await openDesktopSecretFile()
      setPortableNotice('Opened the active secret file in your default application.')
    } catch (caught) {
      setPortableError(caught instanceof Error ? caught.message : 'Unable to open secret file')
    } finally {
      setPortableBusy('')
    }
  }

  return (
    <SettingsSection
      title="Local runtime"
      description="Storage and audit configuration for this machine. Moving the data directory requires a restart and does not copy existing data."
    >
      <div className="form-grid">
        <Field
          label="Effective secret file"
          hint={
            settings.secret_file_managed
              ? 'Owner-only Desktop-managed path for provider, connector, and agent tokens'
              : 'Externally managed runtime.env_file; Desktop will not write this path'
          }
          wide
        >
          <input
            value={settings.secret_file_path}
            title={settings.secret_file_path}
            readOnly
            aria-readonly="true"
          />
        </Field>
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
      <div className="portable-settings">
        <div>
          <strong>Redacted settings backup</strong>
          <p>
            Export configuration without secret values or executable connector commands. Import
            validates a bounded preview and never writes until you save.
          </p>
        </div>
        <div className="service-actions">
          <button
            type="button"
            disabled={Boolean(portableBusy) || dirty}
            title={dirty ? 'Save or discard draft changes before exporting' : 'Export settings'}
            onClick={() => void exportSettings()}
          >
            {portableBusy === 'export' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <Download size={14} />
            )}
            Export
          </button>
          <button
            type="button"
            disabled={Boolean(portableBusy)}
            onClick={() => void importSettings()}
          >
            {portableBusy === 'import' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <Upload size={14} />
            )}
            Import preview
          </button>
          <button
            type="button"
            disabled={Boolean(portableBusy)}
            onClick={() => void openSecretFile()}
          >
            {portableBusy === 'open-secret' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <FolderOpen size={14} />
            )}
            Open secret file
          </button>
        </div>
      </div>
      {(portableNotice || portableError) && (
        <div className={`safety-note ${portableError ? 'error' : ''}`} role="status">
          {portableError ? <AlertTriangle size={16} /> : <Check size={16} />}
          <span>{portableError || portableNotice}</span>
        </div>
      )}
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
  hint,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  hint?: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
}) {
  return (
    <Field label={label} hint={hint}>
      <input
        type="number"
        aria-label={label}
        value={value}
        min={min}
        max={max}
        onChange={(event) => {
          const raw = event.target.value
          if (!raw) return
          const next = Number(raw)
          if (!Number.isFinite(next) || !Number.isInteger(next)) return
          onChange(Math.min(max, Math.max(min, next)))
        }}
        required
      />
    </Field>
  )
}
