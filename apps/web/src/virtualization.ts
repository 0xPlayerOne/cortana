export type VirtualRange = {
  start: number
  end: number
  offsetTop: number
  totalHeight: number
}

export function virtualRange(
  count: number,
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  overscan = 4
): VirtualRange {
  if (count <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0, offsetTop: 0, totalHeight: 0 }
  }
  const safeScrollTop = Math.max(0, scrollTop)
  const visibleStart = Math.min(count - 1, Math.floor(safeScrollTop / rowHeight))
  const visibleEnd = Math.min(
    count,
    Math.ceil((safeScrollTop + Math.max(0, viewportHeight)) / rowHeight)
  )
  const start = Math.max(0, visibleStart - Math.max(0, overscan))
  const end = Math.min(count, Math.max(start + 1, visibleEnd + Math.max(0, overscan)))
  return {
    start,
    end,
    offsetTop: start * rowHeight,
    totalHeight: count * rowHeight,
  }
}
