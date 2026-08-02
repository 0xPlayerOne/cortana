const externalUrlSchemes = new Set(['http:', 'https:', 'mailto:', 'file:', 'slack:'])

function validSlackLink(parsed: URL): boolean {
  if (parsed.hostname !== 'channel' || parsed.pathname || parsed.hash) return false
  const params = new URLSearchParams(parsed.search)
  const values = new Map<string, string>()
  for (const [key, value] of params) {
    if (values.has(key) || !['team', 'id', 'message'].includes(key)) return false
    values.set(key, value)
  }
  const channel = values.get('id')
  const message = values.get('message')
  const team = values.get('team')
  return (
    channel !== undefined &&
    /^[A-Za-z0-9_-]{1,128}$/.test(channel) &&
    message !== undefined &&
    /^\d+(?:\.\d+)?$/.test(message) &&
    (team === undefined || /^[A-Za-z0-9_-]{0,128}$/.test(team))
  )
}

export function safeSourceLink(
  href: string,
  options: { allowLocalFile?: boolean } = {}
): string | null {
  let parsed: URL
  try {
    parsed = new URL(href)
  } catch {
    return null
  }
  if (!externalUrlSchemes.has(parsed.protocol)) return null
  if (parsed.username || parsed.password) return null
  if (parsed.protocol === 'file:' && !options.allowLocalFile) return null
  if (parsed.protocol === 'slack:' && !validSlackLink(parsed)) return null
  if (
    parsed.protocol === 'file:' &&
    ((parsed.hostname && parsed.hostname !== 'localhost') || parsed.search || parsed.hash)
  ) {
    return null
  }
  return href
}
