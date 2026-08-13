const STORAGE_KEY = 'cortana.workspace-logos.v1'
const LOGO_EVENT = 'cortana:workspace-logo'
const MAX_LOGO_BYTES = 200_000
const MAX_LOGO_DIMENSION = 1024
const COMPRESSION_QUALITIES = [0.82, 0.7, 0.58, 0.46, 0.34, 0.22, 0.1] as const
// Base64 encodes every 3 bytes as 4 characters, so the exact encoded payload
// for a MAX_LOGO_BYTES file is 4 * ceil(MAX_LOGO_BYTES / 3). The extra 64
// characters leave room for the data:image/...;base64, prefix so any file
// that passes the byte-size check in readWorkspaceLogoFile also passes
// validation here.
const MAX_LOGO_DATA_URL_LENGTH = 4 * Math.ceil(MAX_LOGO_BYTES / 3) + 64
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
    value.length <= MAX_LOGO_DATA_URL_LENGTH &&
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
  const readAsDataUrl = (): Promise<string> =>
    new Promise((resolve, reject) => {
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

  if (file.size <= MAX_LOGO_BYTES) return readAsDataUrl()
  return compressWorkspaceLogo(file)
}

function loadLogoImage(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    if (typeof Image === 'undefined') {
      reject(new Error('Workspace logo compression is not supported in this browser.'))
      return
    }
    const image = new Image()
    let objectUrl: string | null = null
    const cleanup = () => {
      if (objectUrl && typeof URL !== 'undefined' && typeof URL.revokeObjectURL === 'function') {
        URL.revokeObjectURL(objectUrl)
      }
      image.onload = null
      image.onerror = null
    }
    image.onload = () => {
      cleanup()
      resolve(image)
    }
    image.onerror = () => {
      cleanup()
      reject(new Error('Workspace logo could not be decoded for compression.'))
    }
    if (typeof URL !== 'undefined' && typeof URL.createObjectURL === 'function') {
      try {
        objectUrl = URL.createObjectURL(file)
        image.src = objectUrl
        return
      } catch {
        // Fall through to a data URL for browsers that reject this File.
      }
    }
    const reader = new FileReader()
    reader.onerror = () => {
      cleanup()
      reject(new Error('Workspace logo could not be read.'))
    }
    reader.onload = () => {
      const value = typeof reader.result === 'string' ? reader.result : ''
      if (!value) {
        cleanup()
        reject(new Error('Workspace logo could not be read.'))
        return
      }
      image.src = value
    }
    reader.readAsDataURL(file)
  })
}

function canvasToBlob(canvas: HTMLCanvasElement, quality: number): Promise<Blob | null> {
  return new Promise((resolve) => canvas.toBlob(resolve, 'image/jpeg', quality))
}

async function compressWorkspaceLogo(file: File): Promise<string> {
  if (typeof document === 'undefined') {
    throw new Error('Workspace logo compression is not supported in this browser.')
  }
  const image = await loadLogoImage(file)
  if (!image.naturalWidth || !image.naturalHeight) {
    throw new Error('Workspace logo could not be decoded for compression.')
  }
  const canvas = document.createElement('canvas')
  const context = canvas.getContext('2d')
  if (!context) throw new Error('Workspace logo compression is not supported in this browser.')

  let width = image.naturalWidth
  let height = image.naturalHeight
  const initialScale = Math.min(1, MAX_LOGO_DIMENSION / Math.max(width, height))
  width = Math.max(1, Math.round(width * initialScale))
  height = Math.max(1, Math.round(height * initialScale))

  for (let pass = 0; pass < 8; pass += 1) {
    canvas.width = width
    canvas.height = height
    context.clearRect(0, 0, width, height)
    context.drawImage(image, 0, 0, width, height)
    for (const quality of COMPRESSION_QUALITIES) {
      const blob = await canvasToBlob(canvas, quality)
      if (!blob || blob.size > MAX_LOGO_BYTES) continue
      const value = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onerror = () => reject(new Error('Workspace logo could not be read.'))
        reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : '')
        reader.readAsDataURL(blob)
      })
      if (isWorkspaceLogoDataUrl(value)) return value
    }
    if (width === 1 && height === 1) break
    width = Math.max(1, Math.round(width * 0.75))
    height = Math.max(1, Math.round(height * 0.75))
  }
  throw new Error('Workspace logo could not be compressed below 200 KB.')
}

export { LOGO_EVENT }
