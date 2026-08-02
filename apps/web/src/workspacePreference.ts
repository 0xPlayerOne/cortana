const STORAGE_KEY = 'cortana.workspace-selection.v1'
const MAX_WORKSPACE_ID_LENGTH = 128

export function readWorkspacePreference(): string {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY)?.trim() ?? ''
    return value.length > 0 && value.length <= MAX_WORKSPACE_ID_LENGTH ? value : ''
  } catch {
    // A hardened WebView or private browsing mode may deny localStorage.
    return ''
  }
}

export function writeWorkspacePreference(workspace: string): void {
  try {
    const value = workspace.trim()
    if (!value) {
      window.localStorage.removeItem(STORAGE_KEY)
      return
    }
    if (value.length > MAX_WORKSPACE_ID_LENGTH) {
      window.localStorage.removeItem(STORAGE_KEY)
      return
    }
    window.localStorage.setItem(STORAGE_KEY, value)
  } catch {
    // Workspace selection remains session-local when storage is unavailable.
  }
}
