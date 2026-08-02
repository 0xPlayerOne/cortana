const STORAGE_KEY = 'cortana.favorite-documents.v1'

function readFavoriteIds(): Set<string> {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (!raw) return new Set()
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return new Set()
    return new Set(
      parsed.filter((value): value is string => typeof value === 'string' && value.length > 0)
    )
  } catch {
    // Private browsing and hardened webviews can deny localStorage. Favorites
    // remain optional and must never prevent document viewing.
    return new Set()
  }
}

function writeFavoriteIds(ids: Set<string>): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify([...ids].sort()))
  } catch {
    // A storage failure should not turn a local document action into an error.
  }
}

export function isFavoriteDocument(id: string): boolean {
  return readFavoriteIds().has(id)
}

export function toggleFavoriteDocument(id: string): boolean {
  const ids = readFavoriteIds()
  if (ids.has(id)) {
    ids.delete(id)
  } else {
    ids.add(id)
  }
  writeFavoriteIds(ids)
  return ids.has(id)
}
