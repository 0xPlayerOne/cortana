const externalUrlSchemes = new Set(['http:', 'https:', 'mailto:', 'file:'])

export function safeSourceLink(href: string): string | null {
  let parsed: URL
  try {
    parsed = new URL(href)
  } catch {
    return null
  }
  if (!externalUrlSchemes.has(parsed.protocol)) return null
  if (parsed.username || parsed.password) return null
  if (
    parsed.protocol === 'file:' &&
    ((parsed.hostname && parsed.hostname !== 'localhost') || parsed.search || parsed.hash)
  ) {
    return null
  }
  return href
}
