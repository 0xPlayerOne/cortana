export type ThemeMode = 'blue' | 'accessible'

const THEME_KEY = 'cortana.theme.v1'

export const SUPPORTED_THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'blue', label: 'Blue' },
  { id: 'accessible', label: 'Accessible' },
]

function isThemeMode(value: string | null): value is ThemeMode {
  return value === 'blue' || value === 'accessible'
}

export function readThemePreference(): ThemeMode {
  try {
    const raw = typeof localStorage === 'undefined' ? null : localStorage.getItem(THEME_KEY)
    return isThemeMode(raw) ? raw : 'blue'
  } catch {
    return 'blue'
  }
}

export function writeThemePreference(theme: ThemeMode): void {
  try {
    localStorage.setItem(THEME_KEY, theme)
  } catch {
    // Theme preference remains session-local when storage is unavailable.
  }
}

export function applyTheme(theme: ThemeMode): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-theme', theme)
  }
  writeThemePreference(theme)
}
