const WORKSPACE_SELECTION_KEY = 'cortana.workspace-selection.v1'
const SOURCE_SELECTION_KEY = 'cortana.source-selection.v1'
const MAX_SCOPE_ID_LENGTH = 128

export function readWorkspacePreference(): string {
  return readSelectionPreference(WORKSPACE_SELECTION_KEY)
}

export function writeWorkspacePreference(workspace: string): void {
  writeSelectionPreference(WORKSPACE_SELECTION_KEY, workspace)
}

export function readSourceSelectionPreference(): string {
  return readSelectionPreference(SOURCE_SELECTION_KEY)
}

export function writeSourceSelectionPreference(source: string): void {
  writeSelectionPreference(SOURCE_SELECTION_KEY, source)
}

function readSelectionPreference(key: string): string {
  try {
    const value = window.localStorage.getItem(key)?.trim() ?? ''
    return value.length > 0 && value.length <= MAX_SCOPE_ID_LENGTH ? value : ''
  } catch {
    // A hardened WebView or private browsing mode may deny localStorage.
    return ''
  }
}

function writeSelectionPreference(key: string, value: string): void {
  try {
    const next = value.trim()
    if (!next) {
      window.localStorage.removeItem(key)
      return
    }
    if (next.length > MAX_SCOPE_ID_LENGTH) {
      window.localStorage.removeItem(key)
      return
    }
    window.localStorage.setItem(key, next)
  } catch {
    // Selection scope remains session-local when storage is unavailable.
  }
}
