import { describe, expect, test } from 'bun:test'

import { resolveRendererMode } from './rendererMode'

describe('resolveRendererMode', () => {
  test('keeps the legacy renderer as the temporary packaged default', () => {
    expect(
      resolveRendererMode({ search: '', buildRenderer: undefined, allowQueryOverride: false })
    ).toBe('legacy')
  })

  test('enables the shadcn renderer from the explicit build flag', () => {
    expect(
      resolveRendererMode({
        search: '',
        buildRenderer: 'shadcn',
        allowQueryOverride: false,
      })
    ).toBe('shadcn')
  })

  test('allows a development-only query override for visual acceptance', () => {
    expect(
      resolveRendererMode({
        search: '?demo=1&renderer=shadcn',
        buildRenderer: undefined,
        allowQueryOverride: true,
      })
    ).toBe('shadcn')
    expect(
      resolveRendererMode({
        search: '?renderer=shadcn',
        buildRenderer: undefined,
        allowQueryOverride: false,
      })
    ).toBe('legacy')
  })
})
