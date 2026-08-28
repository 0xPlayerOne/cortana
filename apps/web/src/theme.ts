export type ThemeMode =
  'blue' | 'accessible' | 'forest' | 'plum' | 'sand' | 'graphite' | 'teal' | 'rose'

const THEME_KEY = 'cortana.theme.v1'
export const THEME_EVENT = 'cortana:theme-changed'

export const SUPPORTED_THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'blue', label: 'Navy' },
  { id: 'accessible', label: 'Blue' },
  { id: 'forest', label: 'Forest' },
  { id: 'plum', label: 'Plum' },
  { id: 'sand', label: 'Sand' },
  { id: 'graphite', label: 'Graphite' },
  { id: 'teal', label: 'Teal' },
  { id: 'rose', label: 'Rose' },
]

export function isThemeMode(value: string | null): value is ThemeMode {
  return (
    value === 'blue' ||
    value === 'accessible' ||
    value === 'forest' ||
    value === 'plum' ||
    value === 'sand' ||
    value === 'graphite' ||
    value === 'teal' ||
    value === 'rose'
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
    if (typeof window !== 'undefined') window.dispatchEvent(new Event(THEME_EVENT))
  } catch {
    // Theme preference remains session-local when storage is unavailable.
  }
}

export function applyTheme(theme: ThemeMode, options: { persist?: boolean } = {}): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-theme', theme)
  }
  if (options.persist !== false) writeThemePreference(theme)
}
