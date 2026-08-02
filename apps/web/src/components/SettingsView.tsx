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

import {
  cancelDesktopInstaller,
  cancelDesktopSourceValidation,
  checkDesktopUpdate,
  exportDesktopSettings,
  getDesktopAudit,
  getDesktopInstaller,
  getDesktopInfo,
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
import { INITIAL_SYNC_BUDGETS } from '../types'
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
  InitialSyncBudget,
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

export function SettingsView({
  desktopSettings: externalSettings,
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
  const [localReadiness, setLocalReadiness] = useState<DesktopReadiness | null>(null)
  const setupReadiness = externalReadiness === undefined ? localReadiness : externalReadiness
  const setSetupReadiness = onReadiness ?? setLocalReadiness
  const [localInstallerJob, setLocalInstallerJob] = useState<DesktopInstallJob | null>(null)
  const installerJob = externalInstallerJob === undefined ? localInstallerJob : externalInstallerJob
  const setInstallerJob = onInstallerJob ?? setLocalInstallerJob

  useEffect(() => {
    if (!isDesktopApp) return
    if (externalSettings) {
      // The shell owns the saved snapshot. Do not replace an in-progress local
      // draft when a parent status update re-renders this view.
      if (!dirty) setSettings(externalSettings)
      return
    }
    void getDesktopSettings()
      .then(setSettings)
      .catch((caught: unknown) =>
        setError(caught instanceof Error ? caught.message : 'Unable to load settings')
      )
  }, [externalSettings, dirty])

  useEffect(() => setSection(initialSection), [initialSection])

  useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])

  const update = (change: (draft: DesktopSettings) => DesktopSettings) => {
    setSettings((current) => (current ? change(current) : current))
    setSaved(false)
    setDirty(true)
  }

  const discard = async () => {
    if (!dirty || saving) return
    if (!window.confirm('Discard unsaved Cortana settings changes?')) return
    setSaving(true)
    setError('')
    try {
      const next = await getDesktopSettings()
      setSettings(next)
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
        <div className="settings-header-actions">
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
          {(
            [
              'readiness',
              'services',
              'updates',
              'access',
              'audit',
              'workspaces',
              'sources',
              'embedding',
              'query',
              'hindsight',
              'honcho',
              'ingestion',
              'advanced',
            ] as Section[]
          ).map((item) => (
            <button
              type="button"
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
        <div className={`settings-banner ${error ? 'error' : ''}`} role="status">
          {error ? <AlertTriangle size={16} /> : <Check size={16} />}
          {error ||
            (saved
              ? 'Settings saved. Restart affected services to apply them.'
              : 'A service restart is required.')}
          {!error && settings.restart_required && (
            <button
              type="button"
              className="secondary-button"
              onClick={() => setSection('services')}
            >
              Open services
            </button>
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
      refreshInFlightRef.current = false
    }
  }

  useEffect(() => {
    if (externalServices !== undefined) return
    void refresh()
    const timer = window.setInterval(() => void refresh(), 15_000)
    return () => {
      mountedRef.current = false
      window.clearInterval(timer)
    }
  }, [externalServices])

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
      if (!mountedRef.current) return
      setReport(next)
      onServicesError?.('')
      onServiceActivity?.({
        target: service.name,
        action,
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : 'Service action failed'
      setLocalError(message)
      onServiceActivity?.({
        target: service.name,
        action,
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      setBusy('')
    }
  }

  const toggleAutostart = async (enabled: boolean) => {
    setBusy('autostart')
    actionInFlightRef.current = true
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
    if (
      !window.confirm(
        `${action} the Cortana server and embedding services?\n\nRecurring sync and backup are explicitly excluded.`
      )
    ) {
      return
    }
    setBusy(`all:${action}`)
    actionInFlightRef.current = true
    servicesRequestRef.current += 1
    setLocalError('')
    onServiceActivity?.({ target: 'core services', action, status: 'running', detail: null })
    try {
      const next = await runDesktopServicesActionAll(action)
      if (!mountedRef.current) return
      setReport(next)
      onServicesError?.('')
      onServiceActivity?.({ target: 'core services', action, status: 'succeeded', detail: null })
      if (action === 'restart') onRestarted?.()
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : 'Whole-app service action failed'
      setLocalError(message)
      onServiceActivity?.({
        target: 'core services',
        action,
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      setBusy('')
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
      if (!mountedRef.current) return
      setReport(next)
      onServicesError?.('')
      onServiceActivity?.({
        target: 'core services',
        action: 'install',
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message =
        caught instanceof Error ? caught.message : 'Cortana services could not be installed'
      setLocalError(message)
      onServiceActivity?.({
        target: 'core services',
        action: 'install',
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      setBusy('')
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
      if (!mountedRef.current) return
      setReport(next)
      setScheduleApplyPending(false)
      onServicesError?.('')
      onServiceActivity?.({
        target: 'recurring sync',
        action: 'install',
        status: 'succeeded',
        detail: null,
      })
    } catch (caught) {
      const message =
        caught instanceof Error ? caught.message : 'Recurring sync could not be installed'
      setLocalError(message)
      onServiceActivity?.({
        target: 'recurring sync',
        action: 'install',
        status: 'failed',
        detail: message,
      })
    } finally {
      actionInFlightRef.current = false
      setBusy('')
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
  const [localUpdate, setLocalUpdate] = useState<DesktopUpdate | null>(null)
  const update = externalDesktopUpdate === undefined ? localUpdate : externalDesktopUpdate
  const setUpdate = onDesktopUpdate ?? setLocalUpdate
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const pollInFlightRef = useRef(false)

  useEffect(() => {
    if (externalDesktopUpdate !== undefined && externalDesktopUpdate !== null) return
    void getDesktopUpdate()
      .then(setUpdate)
      .catch((caught: unknown) => {
        setError(caught instanceof Error ? caught.message : 'Updater status unavailable')
      })
  }, [externalDesktopUpdate, setUpdate])

  useEffect(() => {
    if (externalDesktopUpdate !== undefined || busy !== 'install') return
    const poll = () => {
      if (pollInFlightRef.current) return
      pollInFlightRef.current = true
      void getDesktopUpdate()
        .then(setUpdate)
        .catch(() => {})
        .finally(() => {
          pollInFlightRef.current = false
        })
    }
    const timer = window.setInterval(poll, 400)
    return () => window.clearInterval(timer)
  }, [busy, externalDesktopUpdate])

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
        <div className="safety-note">
          <AlertTriangle size={16} /> <span>{error || update?.error}</span>
        </div>
      )}
      {update?.release_notes && (
        <div className="release-notes">
          <h3>Version {update.available_version}</h3>
          <pre>{update.release_notes}</pre>
        </div>
      )}
      <div className="release-notes">
        <h3>Installed changelog</h3>
        <pre>{update?.changelog || 'Loading changelog…'}</pre>
      </div>
      {update && (
        <button type="button" className="link-button" onClick={() => void openDesktopProject()}>
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
                  aria-label={`Remove ${principal.principal}`}
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

  return (
    <SettingsSection
      title="Audit trail"
      description="Bounded metadata-only runtime and Desktop events. Queries, document contents, bearer tokens, and secret values are excluded."
    >
      <div className="source-settings-toolbar">
        <span>
          {runtime.length} runtime · {desktop.length} Desktop events
        </span>
        <button type="button" disabled={loading} onClick={() => void refresh()}>
          {loading ? <LoaderCircle className="spin" size={14} /> : <RefreshCw size={14} />}
          Refresh
        </button>
      </div>
      {error && <div className="safety-note">{error}</div>}
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
  const [scanning, setScanning] = useState(false)
  const [migratingGeneration, setMigratingGeneration] = useState(false)
  const [error, setError] = useState('')
  const [migrationNotice, setMigrationNotice] = useState('')
  const autoScanAttemptedRef = useRef(false)

  useEffect(() => {
    if (!pollInstaller || !job || !['running', 'cancelling'].includes(job.status)) return
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
  }, [job, onJob, onReadinessScan, onResult, pollInstaller])

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
      const next = await (onReadinessScan ? onReadinessScan() : scanDesktopReadiness())
      onResult(next)
      setMigrationNotice('Embedding generation adopted and readiness was rescanned.')
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
        <div className="safety-note">
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
  const addWorkspace = () =>
    update((current) => ({
      ...current,
      workspaces: [
        ...current.workspaces,
        {
          id: nextAvailableIdentifier(
            'workspace',
            current.workspaces.map((workspace) => workspace.id)
          ),
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
                  disabled={settings.sources.some((source) => source.project === workspace.id)}
                  title={
                    settings.sources.some((source) => source.project === workspace.id)
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
            <Field label="Scope ID" hint="lowercase letters, numbers, dashes">
              <input
                value={workspace.id}
                disabled={settings.sources.some((source) => source.project === workspace.id)}
                title={
                  settings.sources.some((source) => source.project === workspace.id)
                    ? 'Move assigned sources before changing this workspace ID'
                    : undefined
                }
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
  const [initialSync, setInitialSync] = useState<{
    source: string
    budget: InitialSyncBudget
    plan: DesktopInitialSyncPlan | null
    planning: boolean
    flowError: string
  } | null>(null)
  const validationPlanKey = useRef('')
  const sharedJobIds = useRef(new Set<string>())

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
    if (sourceJobs) return
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
  }, [job, initialSync, onJob])
  const activeJob =
    (job && ['running', 'cancelling'].includes(job.status) ? job : undefined) ??
    sourceJobs?.find((candidate) => ['running', 'cancelling'].includes(candidate.status))
  const observedJob = activeJob ?? job ?? sourceJobs?.[0]
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
    if (
      !window.confirm(
        `Authorize ${source.name} with Google?\n\nCortana will open the system browser, listen only on a random 127.0.0.1 callback port, request read-only scopes for Google sources sharing this token, and store the resulting token in the configured private file.`
      )
    ) {
      return
    }
    setError('')
    try {
      applyJob(await startDesktopSourceAuthorization(source.name))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Google authorization failed to start')
    }
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
    if (!observedJob) return
    try {
      applyJob(await cancelDesktopSourceValidation(observedJob.id))
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : 'Source validation could not be cancelled'
      )
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
        {settings.sources.map((source, index) => {
          const secret = source.token_env
            ? settings.secrets.find((item) => item.name === source.token_env)
            : undefined
          const runningThis = activeJob?.source === source.name
          const sourceLocked = runningThis
          return (
            <article className="source-settings-card" key={`${source.name}:${index}`}>
              <header>
                <label className="source-enable">
                  <input
                    type="checkbox"
                    checked={source.enabled}
                    disabled={sourceLocked}
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
                      disabled={!canValidate || sourceLocked}
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
                        (!source.token_path && !source.token_env) ||
                        !source.oauth_client_path ||
                        Boolean(activeJob)
                      }
                      title="Authorize read-only Google access with PKCE"
                      onClick={() => void authorizeSource(source)}
                    >
                      {runningThis && activeJob?.operation === 'authorization' ? (
                        <LoaderCircle className="spin" size={14} />
                      ) : (
                        <KeyRound size={14} />
                      )}
                      Authorize
                    </button>
                  )}
                  <button
                    type="button"
                    disabled={!canValidate || Boolean(activeJob)}
                    title={canValidate ? 'Read-only bounded validation' : 'Save changes first'}
                    onClick={() => void validateSource(source)}
                  >
                    {runningThis && activeJob?.operation === 'validation' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Play size={14} />
                    )}
                    Validate
                  </button>
                  <button
                    type="button"
                    disabled={!canValidate || !source.enabled || Boolean(activeJob)}
                    title="Validation-gated trial sync; max 25 documents, 5 MiB, no reconciliation"
                    onClick={() => void trialSyncSource(source)}
                  >
                    {runningThis && activeJob?.operation === 'trial-sync' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Play size={14} />
                    )}
                    Trial sync
                  </button>
                  <button
                    type="button"
                    disabled={!canValidate || !source.enabled || Boolean(activeJob)}
                    title="Guided initial sync; fixed budget, validation-gated, no reconciliation"
                    onClick={() => openInitialSync(source)}
                  >
                    {runningThis && activeJob?.operation === 'initial-sync' ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <Zap size={14} />
                    )}
                    Initial sync
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove ${source.name}`}
                    disabled={sourceLocked}
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
                        disabled={sourceLocked || !source.editable}
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
                        disabled={sourceLocked || !source.editable}
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
                {(source.kind === 'slack' || source.kind === 'discord') && (
                  <>
                    <Field label="Channel IDs" hint="comma or line separated" wide>
                      <input
                        value={source.channels.join(', ')}
                        disabled={sourceLocked || !source.editable}
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
                        disabled={sourceLocked || !source.editable}
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
                    <Field label="Content limit (characters)" hint="blank uses connector defaults">
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
            </article>
          )
        })}
      </div>

      {initialSync && settings.sources.find((item) => item.name === initialSync.source) && (
        <InitialSyncFlow
          source={settings.sources.find((item) => item.name === initialSync.source)!}
          flow={initialSync}
          busy={Boolean(activeJob) || !canValidate}
          onBudget={(budget) => void requestPlan(initialSync.source, budget)}
          onValidate={() => void validateInitialSyncBudget(sourceOf(settings, initialSync.source))}
          onStart={() => void startInitialSync(sourceOf(settings, initialSync.source))}
          onClose={() => setInitialSync(null)}
        />
      )}

      {error && <div className="safety-note">{error}</div>}
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
        <button type="button" aria-label="Close initial sync plan" onClick={onClose}>
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
      {flow.flowError && <div className="safety-note">{flow.flowError}</div>}
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

function newSource(settings: DesktopSettings): SourceSettings {
  return {
    name: nextAvailableIdentifier(
      'source',
      settings.sources.map((source) => source.name)
    ),
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

function nextAvailableIdentifier(prefix: string, used: readonly string[]): string {
  const occupied = new Set(used)
  for (let number = 1; ; number += 1) {
    const candidate = `${prefix}-${number}`
    if (!occupied.has(candidate)) return candidate
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
