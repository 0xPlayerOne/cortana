import {
  AlertTriangle,
  Check,
  CircleStop,
  Download,
  FolderOpen,
  KeyRound,
  LoaderCircle,
  Upload,
} from 'lucide-react'
import { useEffect, useState } from 'react'

import {
  cancelDesktopVaultExport,
  exportDesktopSettings,
  getDesktopVaultExport,
  importDesktopSettings,
  migrateDesktopSecrets,
  openDesktopSecretFile,
  startDesktopVaultExport,
} from '../../api'
import type { DesktopSettings, DesktopVaultExport } from '../../types'
import { useSettingsConfirm } from './SettingsConfirm'
import { Field, NumberField, SettingsSection, type SettingsSectionProps } from './SettingsLayout'
import {
  SettingsAlert,
  SettingsButton as Button,
  SettingsFieldGroup,
  SettingsInput as Input,
} from './SettingsSurface'

export function AdvancedSettingsSection({
  settings,
  update,
  dirty,
}: SettingsSectionProps & { dirty: boolean }) {
  const confirm = useSettingsConfirm()
  const [portableBusy, setPortableBusy] = useState<
    'export' | 'import' | 'open-secret' | 'migrate-secrets' | ''
  >('')
  const [portableNotice, setPortableNotice] = useState('')
  const [portableError, setPortableError] = useState('')
  const [vaultSelected, setVaultSelected] = useState(
    () => new Set(settings.workspaces.map((workspace) => workspace.id))
  )
  const [vaultJob, setVaultJob] = useState<DesktopVaultExport | null>(null)
  const [vaultError, setVaultError] = useState('')
  const setRuntime = (patch: Partial<DesktopSettings['runtime']>) =>
    update((current) => ({ ...current, runtime: { ...current.runtime, ...patch } }))

  useEffect(() => {
    if (!vaultJob || !['running', 'cancelling'].includes(vaultJob.status)) return
    let active = true
    const timer = window.setTimeout(async () => {
      try {
        const next = await getDesktopVaultExport(vaultJob.id)
        if (active) setVaultJob(next)
      } catch (caught) {
        if (active) {
          setVaultError(caught instanceof Error ? caught.message : 'Vault status unavailable')
        }
      }
    }, 250)
    return () => {
      active = false
      window.clearTimeout(timer)
    }
  }, [vaultJob])

  const startVaultExport = async (dryRun: boolean) => {
    const workspaces = [...vaultSelected]
    if (!workspaces.length) {
      setVaultError('Select at least one workspace.')
      return
    }
    if (
      !dryRun &&
      !(await confirm(
        `Export ${workspaces.length} selected workspace${workspaces.length === 1 ? '' : 's'} as a derived Obsidian vault?\n\nCortana will replace only a vault it manages, retain the previous complete export, and never ingest edits from the exported Markdown.`
      ))
    ) {
      return
    }
    setVaultError('')
    try {
      const job = await startDesktopVaultExport(workspaces, dryRun)
      if (job) setVaultJob(job)
    } catch (caught) {
      setVaultError(caught instanceof Error ? caught.message : 'Vault export failed')
    }
  }

  const cancelVaultExport = async () => {
    if (!vaultJob) return
    try {
      setVaultJob(await cancelDesktopVaultExport(vaultJob.id))
    } catch (caught) {
      setVaultError(caught instanceof Error ? caught.message : 'Vault cancellation failed')
    }
  }

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
        !(await confirm(
          `Load the validated settings from ${result.path} into this form?\n\nSecret values are never imported. Existing executable connectors are preserved. Saving a changed principal list may remove credentials for principals you remove. Nothing is written until you choose Save changes.`
        ))
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

  const migrateSecrets = async () => {
    if (dirty) return
    if (
      !(await confirm(
        'Move configured secret-file values into platform secure storage? This is explicit and recoverable, removes migrated plaintext values from secrets.env, and never includes secret values in the audit log.'
      ))
    ) {
      return
    }
    setPortableBusy('migrate-secrets')
    setPortableNotice('')
    setPortableError('')
    try {
      const result = await migrateDesktopSecrets()
      setPortableNotice(
        result.migrated === 0
          ? 'Secure storage is already active, or no secret-file values were eligible to migrate.'
          : `Migrated ${result.migrated} secret${result.migrated === 1 ? '' : 's'} to platform secure storage. Plaintext file values were removed.`
      )
    } catch (caught) {
      setPortableError(caught instanceof Error ? caught.message : 'Secure-storage migration failed')
    } finally {
      setPortableBusy('')
    }
  }

  return (
    <SettingsSection
      title="Local runtime"
      description="Storage and audit configuration for this machine. Moving the data directory requires a restart and does not copy existing data."
    >
      <SettingsFieldGroup className="form-grid">
        <Field
          label="Effective secret file"
          hint={
            settings.secret_file_managed
              ? 'Owner-only Desktop-managed path for provider, connector, and agent tokens'
              : 'Externally managed runtime.env_file; Desktop will not write this path'
          }
          wide
        >
          <Input
            value={settings.secret_file_path}
            title={settings.secret_file_path}
            readOnly
            aria-readonly="true"
          />
        </Field>
        <Field label="Data directory" wide>
          <Input
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
      </SettingsFieldGroup>
      <div className="portable-settings">
        <div>
          <strong>Redacted settings backup</strong>
          <p>
            Export configuration without secret values or executable connector commands. Import
            validates a bounded preview and never writes until you save.
          </p>
        </div>
        <div className="service-actions">
          <Button
            variant="secondary"
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
          </Button>
          <Button
            variant="secondary"
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
          </Button>
          <Button
            variant="secondary"
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
          </Button>
          <Button
            variant="secondary"
            type="button"
            disabled={Boolean(portableBusy) || dirty}
            title={
              dirty ? 'Save or discard draft changes before migrating secrets' : 'Migrate secrets'
            }
            onClick={() => void migrateSecrets()}
          >
            {portableBusy === 'migrate-secrets' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <KeyRound size={14} />
            )}
            Migrate to secure storage
          </Button>
        </div>
      </div>
      {(portableNotice || portableError) && (
        <SettingsAlert
          className={`safety-note ${portableError ? 'error' : ''}`}
          variant={portableError ? 'destructive' : 'default'}
          role={portableError ? 'alert' : 'status'}
        >
          {portableError ? <AlertTriangle size={16} /> : <Check size={16} />}
          <span>{portableError || portableNotice}</span>
        </SettingsAlert>
      )}
      <div className="portable-settings">
        <div>
          <strong>Derived Obsidian vault</strong>
          <p>
            Export authorized canonical documents as deterministic Markdown. The vault is a
            read-only projection from Cortana’s perspective and can be removed or rebuilt at any
            time.
          </p>
          <fieldset
            className="vault-workspace-picker"
            disabled={Boolean(vaultJob && ['running', 'cancelling'].includes(vaultJob.status))}
          >
            <legend>Workspaces to export</legend>
            {settings.workspaces.map((workspace) => (
              <label key={workspace.id}>
                <input
                  type="checkbox"
                  checked={vaultSelected.has(workspace.id)}
                  onChange={(event) =>
                    setVaultSelected((current) => {
                      const next = new Set(current)
                      if (event.target.checked) next.add(workspace.id)
                      else next.delete(workspace.id)
                      return next
                    })
                  }
                />
                {workspace.name}
              </label>
            ))}
          </fieldset>
        </div>
        <div className="service-actions">
          <Button
            variant="secondary"
            type="button"
            disabled={
              dirty || Boolean(vaultJob && ['running', 'cancelling'].includes(vaultJob.status))
            }
            title={dirty ? 'Save or discard workspace changes first' : 'Preview vault export'}
            onClick={() => void startVaultExport(true)}
          >
            <Download size={14} /> Preview vault export
          </Button>
          <Button
            variant="secondary"
            type="button"
            disabled={
              dirty || Boolean(vaultJob && ['running', 'cancelling'].includes(vaultJob.status))
            }
            title={dirty ? 'Save or discard workspace changes first' : 'Export Obsidian vault'}
            onClick={() => void startVaultExport(false)}
          >
            {vaultJob?.status === 'running' ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <FolderOpen size={14} />
            )}
            Export vault
          </Button>
          {vaultJob && ['running', 'cancelling'].includes(vaultJob.status) && (
            <Button
              variant="danger"
              type="button"
              disabled={vaultJob.status === 'cancelling'}
              onClick={() => void cancelVaultExport()}
            >
              <CircleStop size={14} /> Cancel vault export
            </Button>
          )}
        </div>
      </div>
      {vaultJob && (
        <SettingsAlert
          className={`safety-note ${vaultJob.status === 'failed' ? 'error' : ''}`}
          variant={vaultJob.status === 'failed' ? 'destructive' : 'default'}
          role={vaultJob.status === 'failed' ? 'alert' : 'status'}
          aria-live="polite"
        >
          {vaultJob.status === 'failed' ? <AlertTriangle size={16} /> : <Check size={16} />}
          <span>
            {vaultJob.status === 'succeeded' && vaultJob.report
              ? `${vaultJob.dry_run ? 'Previewed' : 'Exported'} ${vaultJob.report.documents} documents; ${vaultJob.report.content_rewrites} content rewrites and ${vaultJob.report.unchanged_documents} unchanged.`
              : `Vault export ${vaultJob.phase}: ${vaultJob.documents_completed} documents scanned, ${vaultJob.files_written} files staged.`}
          </span>
        </SettingsAlert>
      )}
      {vaultError && (
        <SettingsAlert className="safety-note error" variant="destructive" role="alert">
          <AlertTriangle size={16} />
          <span>{vaultError}</span>
        </SettingsAlert>
      )}
    </SettingsSection>
  )
}
