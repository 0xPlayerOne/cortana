import { afterEach, expect, test } from 'bun:test'
import { cleanup, render } from '@testing-library/react'

import { WorkspaceLogo } from './workspaceLogos'
import {
  isWorkspaceLogoDataUrl,
  readWorkspaceLogo,
  readWorkspaceLogoFile,
  writeWorkspaceLogo,
} from './workspaceLogoStore'

afterEach(() => {
  cleanup()
  try {
    localStorage.clear()
  } catch {
    // Storage may be unavailable in exotic harness environments.
  }
})

const pngDataUrl = 'data:image/png;base64,iVBORw0KGgo='

test('workspace logo data URLs accept the base64-expanded size bound', () => {
  expect(isWorkspaceLogoDataUrl(pngDataUrl)).toBe(true)
  // A 200 KB file encodes to ~266 KB of base64 plus the prefix. The gate must
  // accept the full expanded size; the old byte-sized string gate rejected
  // every logo larger than ~146 KB with a misleading validation error.
  const fullSizePayload = 'A'.repeat(Math.ceil((200_000 * 4) / 3))
  expect(isWorkspaceLogoDataUrl(`data:image/png;base64,${fullSizePayload}`)).toBe(true)
  // Anything beyond the expanded bound (which leaves slack for the prefix) is
  // rejected.
  const oversizedPayload = 'A'.repeat(Math.ceil((200_000 * 4) / 3) + 64)
  expect(isWorkspaceLogoDataUrl(`data:image/png;base64,${oversizedPayload}`)).toBe(false)
})

test('workspace logo data URLs reject non-raster or malformed payloads', () => {
  expect(isWorkspaceLogoDataUrl('data:image/svg+xml;base64,AAAA')).toBe(false)
  expect(isWorkspaceLogoDataUrl('data:image/png;base64,not base64!')).toBe(false)
  expect(isWorkspaceLogoDataUrl('not-a-data-url')).toBe(false)
})

test('workspace logo files reject unsupported types and oversized files', async () => {
  const svg = new File(['<svg/>'], 'logo.svg', { type: 'image/svg+xml' })
  await expect(readWorkspaceLogoFile(svg)).rejects.toThrow(
    'Choose a PNG, JPEG, WebP, or GIF image.'
  )
  const oversized = new File([new Uint8Array(200_001)], 'big.png', { type: 'image/png' })
  await expect(readWorkspaceLogoFile(oversized)).rejects.toThrow(
    'Workspace logos must be 200 KB or smaller.'
  )
})

test('workspace logo files within the size bound produce a valid data URL', async () => {
  const file = new File(['logo-bytes'], 'logo.png', { type: 'image/png' })
  const dataUrl = await readWorkspaceLogoFile(file)
  expect(dataUrl.startsWith('data:image/png;base64,')).toBe(true)
  expect(isWorkspaceLogoDataUrl(dataUrl)).toBe(true)
})

test('workspace logos round-trip through local storage and reject invalid ids', () => {
  expect(readWorkspaceLogo('work')).toBeNull()
  writeWorkspaceLogo('work', pngDataUrl)
  expect(readWorkspaceLogo('work')).toBe(pngDataUrl)
  // Invalid workspace ids are ignored.
  writeWorkspaceLogo('Work!', pngDataUrl)
  expect(readWorkspaceLogo('Work!')).toBeNull()
  // Invalid payloads never persist; they clear any existing entry.
  writeWorkspaceLogo('work', 'data:image/svg+xml;base64,AAAA')
  expect(readWorkspaceLogo('work')).toBeNull()
  // Removing a logo clears the entry.
  writeWorkspaceLogo('work', pngDataUrl)
  writeWorkspaceLogo('work', null)
  expect(readWorkspaceLogo('work')).toBeNull()
})

test('WorkspaceLogo renders the workspace initial tile without a stored logo', () => {
  const { container } = render(
    <WorkspaceLogo workspace={{ id: 'work', name: 'Work', color: '#5A9BD5' }} />
  )
  const tile = container.querySelector('.workspace-logo') as HTMLElement
  expect(tile).toBeTruthy()
  expect(tile.className).toContain('workspace-logo--medium')
  expect(tile.textContent).toBe('W')
  expect(tile.getAttribute('style')).toContain('#5A9BD5')
  expect(tile.getAttribute('aria-hidden')).toBe('true')
})

test('WorkspaceLogo small variant composes with the workspace picker ring', () => {
  const { container } = render(
    <WorkspaceLogo workspace={{ id: 'work', name: 'Work', color: null }} size="small" />
  )
  const tile = container.querySelector('.workspace-logo') as HTMLElement
  expect(tile.className).toContain('workspace-logo--small')
  expect(tile.className).toContain('workspace-picker-mark')
})

test('WorkspaceLogo renders a stored logo image with decorative alt behavior', () => {
  writeWorkspaceLogo('work', pngDataUrl)
  const { container } = render(
    <WorkspaceLogo workspace={{ id: 'work', name: 'Work', color: '#5A9BD5' }} />
  )
  const img = container.querySelector('img.workspace-logo') as HTMLImageElement
  expect(img).toBeTruthy()
  expect(img.getAttribute('src')).toBe(pngDataUrl)
  expect(img.getAttribute('alt')).toBe('')
  expect(img.getAttribute('aria-hidden')).toBe('true')
})
