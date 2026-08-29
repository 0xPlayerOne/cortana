export type ThemeMode =
  | 'blue'
  | 'accessible'
  | 'forest'
  | 'plum'
  | 'sand'
  | 'graphite'
  | 'teal'
  | 'rose'
  | 'slate'
  | 'indigo'
  | 'emerald'
  | 'amber'

export const DEFAULT_THEME: ThemeMode = 'graphite'

export const SUPPORTED_THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'blue', label: 'Navy' },
  { id: 'accessible', label: 'Blue' },
  { id: 'forest', label: 'Forest' },
  { id: 'plum', label: 'Plum' },
  { id: 'sand', label: 'Sand' },
  { id: 'graphite', label: 'Graphite' },
  { id: 'teal', label: 'Teal' },
  { id: 'rose', label: 'Rose' },
  { id: 'slate', label: 'Slate' },
  { id: 'indigo', label: 'Indigo' },
  { id: 'emerald', label: 'Emerald' },
  { id: 'amber', label: 'Amber' },
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
    value === 'rose' ||
    value === 'slate' ||
    value === 'indigo' ||
    value === 'emerald' ||
    value === 'amber'
  )
}

export function applyTheme(theme: ThemeMode): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-theme', theme)
  }
}
