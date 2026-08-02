const externalUrlSchemes = new Set([
  'http:',
  'https:',
  'mailto:',
  'file:',
  'slack:',
  'notes:',
  'buzz:',
])

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

function validNotesLink(parsed: URL): boolean {
  if (parsed.hostname.toLowerCase() !== 'shownote' || parsed.pathname || parsed.hash) return false
  const values = new Map<string, string>()
  for (const [key, value] of parsed.searchParams) {
    if (values.has(key) || key !== 'identifier') return false
    values.set(key, value)
  }
  const identifier = values.get('identifier')
  return identifier !== undefined && validCustomLinkValue(identifier, 1024, true)
}

function validBuzzLink(parsed: URL): boolean {
  if (parsed.hostname.toLowerCase() !== 'persona' || parsed.search || parsed.hash) return false
  const segments = parsed.pathname.split('/')
  return (
    segments.length === 3 &&
    segments[0] === '' &&
    segments.slice(1).every((segment) => {
      try {
        return validCustomLinkValue(decodeURIComponent(segment), 256)
      } catch {
        return false
      }
    })
  )
}

function validCustomLinkValue(value: string, maximumLength: number, allowSlash = false): boolean {
  return (
    value.length > 0 &&
    value.length <= maximumLength &&
    (allowSlash || !value.includes('/')) &&
    ![...value].some((character) => character < ' ' || character === '\u007f')
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
  if (parsed.protocol === 'notes:' && !validNotesLink(parsed)) return null
  if (parsed.protocol === 'buzz:' && !validBuzzLink(parsed)) return null
  if (
    parsed.protocol === 'file:' &&
    ((parsed.hostname && parsed.hostname !== 'localhost') || parsed.search || parsed.hash)
  ) {
    return null
  }
  return href
}
