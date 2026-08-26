export type RendererMode = 'legacy' | 'shadcn'

export function resolveRendererMode({
  search,
  buildRenderer,
  allowQueryOverride,
}: {
  search: string
  buildRenderer: string | undefined
  allowQueryOverride: boolean
}): RendererMode {
  if (buildRenderer === 'shadcn') return 'shadcn'
  if (
    allowQueryOverride &&
    new URLSearchParams(search).get('renderer')?.toLowerCase() === 'shadcn'
  ) {
    return 'shadcn'
  }
  return 'legacy'
}
