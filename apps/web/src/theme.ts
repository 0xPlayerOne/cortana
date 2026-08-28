export type ThemeMode = 'blue' | 'accessible' | 'forest' | 'plum' | 'sand'

const THEME_KEY = 'cortana.theme.v1'

export const SUPPORTED_THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'blue', label: 'Navy' },
  { id: 'accessible', label: 'Blue' },
  { id: 'forest', label: 'Forest' },
  { id: 'plum', label: 'Plum' },
  { id: 'sand', label: 'Sand' },
]

function isThemeMode(value: string | null): value is ThemeMode {
  return (
    value === 'blue' ||
    value === 'accessible' ||
    value === 'forest' ||
    value === 'plum' ||
    value === 'sand'
  )
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
