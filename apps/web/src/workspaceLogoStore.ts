const STORAGE_KEY = 'cortana.workspace-logos.v1'
const LOGO_EVENT = 'cortana:workspace-logo'
const MAX_LOGO_BYTES = 200_000
const ALLOWED_LOGO_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif'])

type WorkspaceLogoMap = Record<string, string>

function readLogoMap(): WorkspaceLogoMap {
  try {
    if (typeof localStorage === 'undefined') return {}
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}')
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    return Object.fromEntries(
      Object.entries(parsed).filter(
        ([key, value]) =>
          /^[a-z0-9][a-z0-9_-]*$/.test(key) &&
          typeof value === 'string' &&
          isWorkspaceLogoDataUrl(value)
      )
    )
  } catch {
    return {}
  }
}

export function isWorkspaceLogoDataUrl(value: string): boolean {
  return (
    value.length <= MAX_LOGO_BYTES &&
    /^data:image\/(?:png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$/.test(value)
  )
}

export function readWorkspaceLogo(workspaceId: string): string | null {
  return readLogoMap()[workspaceId] ?? null
}

export function writeWorkspaceLogo(workspaceId: string, logo: string | null): void {
  if (!/^[a-z0-9][a-z0-9_-]*$/.test(workspaceId)) return
  const next = readLogoMap()
  if (logo && isWorkspaceLogoDataUrl(logo)) next[workspaceId] = logo
  else delete next[workspaceId]
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent(LOGO_EVENT))
  } catch {
    // A presentation logo is optional; keep the current in-memory view alive
    // when storage is unavailable or full.
  }
}

export function readWorkspaceLogoFile(file: File): Promise<string> {
  if (!ALLOWED_LOGO_TYPES.has(file.type)) {
    return Promise.reject(new Error('Choose a PNG, JPEG, WebP, or GIF image.'))
  }
  if (file.size > MAX_LOGO_BYTES) {
    return Promise.reject(new Error('Workspace logos must be 200 KB or smaller.'))
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(new Error('Workspace logo could not be read.'))
    reader.onload = () => {
      const value = typeof reader.result === 'string' ? reader.result : ''
      if (!isWorkspaceLogoDataUrl(value)) {
        reject(new Error('Workspace logo could not be validated.'))
        return
      }
      resolve(value)
    }
    reader.readAsDataURL(file)
  })
}

export { LOGO_EVENT }
