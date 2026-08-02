/**
 * Return the modifier label users should press on this platform.
 *
 * The keyboard handlers intentionally accept both Meta and Control so the
 * same application works in a browser and in Tauri. Keep the visible labels
 * in one place so Windows/Linux users are not shown macOS-only shortcuts.
 */
export function shortcutModifier(): '⌘' | 'Ctrl' {
  if (typeof navigator === 'undefined') return 'Ctrl'
  return /Mac|iPhone|iPad|iPod/.test(navigator.platform) ? '⌘' : 'Ctrl'
}

export function shortcutLabel(keys: string): string {
  return keys.replace(/\bMOD\b/g, shortcutModifier())
}
