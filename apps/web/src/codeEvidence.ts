import type { Evidence } from './types'

export function codeRevisionLabel(evidence: Evidence): string | null {
  const code = evidence.metadata?.code
  if (!code || typeof code !== 'object') return null
  const repository = typeof code.repository === 'string' ? code.repository : 'repository'
  const branch = typeof code.branch === 'string' ? code.branch : 'detached'
  const commit = typeof code.commit_sha === 'string' ? code.commit_sha.slice(0, 8) : 'uncommitted'
  const state = code.dirty === true ? 'dirty' : 'committed'
  return `${repository} · ${branch} · ${commit} · ${state}`
}
