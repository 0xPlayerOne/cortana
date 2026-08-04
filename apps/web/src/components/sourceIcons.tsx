import { sourceBrandForKind, sourceIconForKind } from './sourceIconData'

export function SourceIcon({ kind, size = 17 }: { kind: string; size?: number }) {
  const brand = sourceBrandForKind(kind)
  if (!brand) {
    const Icon = sourceIconForKind(kind)
    return <Icon size={size} aria-hidden="true" />
  }
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
      style={{ color: `#${brand.hex}` }}
    >
      <path d={brand.path} />
    </svg>
  )
}
