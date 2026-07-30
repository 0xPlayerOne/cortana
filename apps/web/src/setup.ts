import type { DesktopReadiness, DesktopSettings } from './types'

export type SetupStep = {
  section: 'readiness' | 'workspaces' | 'embedding' | 'sources'
  label: string
  detail: string
  complete: boolean
}

export function buildSetupSteps(
  settings: Pick<DesktopSettings, 'workspaces' | 'embedding' | 'sources'>,
  readiness: DesktopReadiness | null
): SetupStep[] {
  return [
    {
      section: 'readiness',
      label: 'System',
      detail: readiness
        ? readiness.tools_ready && readiness.core?.passed
          ? 'Tools and runtime checks passed'
          : 'Review missing tools or runtime checks'
        : 'Checking required tools automatically',
      complete: Boolean(readiness?.tools_ready && readiness.core?.passed),
    },
    {
      section: 'workspaces',
      label: 'Workspaces',
      detail: `${settings.workspaces.length} configured`,
      complete: settings.workspaces.length > 0,
    },
    {
      section: 'embedding',
      label: 'Embedding',
      detail: settings.embedding.model || 'Choose a local or cloud model',
      complete: Boolean(settings.embedding.base_url && settings.embedding.model),
    },
    {
      section: 'sources',
      label: 'Sources',
      detail: settings.sources.length
        ? `${settings.sources.length} configured`
        : 'Add at least one source',
      complete: settings.sources.length > 0,
    },
  ]
}
