import type { DesktopSettings, SourceSettings, WorkspaceSettings } from '../../types'

export function deriveWorkspaceIdentifier(name: string): string {
  const normalized = name
    .trim()
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^\w\s-]/g, '')
    .replace(/[\s_]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-+|-+$/g, '')

  if (!normalized) return 'workspace'

  const candidate = normalized.slice(0, 32).replace(/-[0-9]+$/, '')
  if (/^[a-z0-9][a-z0-9-]*$/.test(candidate)) return candidate
  return `workspace-${normalized}`.slice(0, 32)
}

export function ensureWorkspaceIdentifierUnique(base: string, used: readonly string[]): string {
  const trimmed = base.trim() || 'workspace'
  const occupied = new Set(used)
  if (!occupied.has(trimmed) && isWorkspaceIdentifierSafe(trimmed)) return trimmed

  let counter = 1
  while (occupied.has(`${trimmed}-${counter}`)) counter += 1
  return `${trimmed}-${counter}`
}

function isWorkspaceIdentifierSafe(value: string) {
  return /^[a-z0-9][a-z0-9_-]*$/.test(value)
}

export function isWorkspaceIdDerivedFromName(workspace: WorkspaceSettings) {
  return (
    isWorkspaceIdentifierSafe(workspace.id) &&
    workspace.id === deriveWorkspaceIdentifier(workspace.name)
  )
}

export function referencedSecretNames(settings: DesktopSettings): Set<string> {
  const names = new Set<string>()
  for (const name of [settings.embedding.api_key_env, settings.query.api_key_env]) {
    if (name) names.add(name)
  }
  settings.sources.forEach((source) => {
    if (source.token_env) names.add(source.token_env)
  })
  settings.auth_principals.forEach((principal) => names.add(principal.token_env))
  return names
}

export function validateSourceIdentityScopes(sources: readonly SourceSettings[]): string | null {
  const seen = new Map<string, string>()
  for (const source of sources) {
    const configured = source.source
    if (
      configured !== null &&
      (configured.trim().length === 0 ||
        configured !== configured.trim() ||
        Array.from(configured).some((character) => {
          const codePoint = character.codePointAt(0) ?? 0
          return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)
        }))
    ) {
      return `Source label for \`${source.name}\` must not be empty, padded with whitespace, or contain control characters.`
    }
    const canonical = configured ?? source.name.trim()
    const scope = `${source.project}\u0000${canonical}`
    const previous = seen.get(scope)
    if (previous) {
      return `Source identifier \`${canonical}\` is duplicated in workspace \`${source.project}\` (\`${previous}\` and \`${source.name}\`). Choose a unique source label before saving.`
    }
    seen.set(scope, source.name)
  }
  return null
}
