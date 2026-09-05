const PROVENANCES = Object.freeze(['published', 'prospective-source'])

export function resolveAcceptanceProvenance(env = process.env) {
  const value =
    typeof env.CORTANA_ACCEPTANCE_PROVENANCE === 'string' &&
    env.CORTANA_ACCEPTANCE_PROVENANCE.trim()
      ? env.CORTANA_ACCEPTANCE_PROVENANCE.trim()
      : PROVENANCES[0]
  if (!PROVENANCES.includes(value)) {
    throw new Error(`CORTANA_ACCEPTANCE_PROVENANCE must be one of ${PROVENANCES.join(', ')}`)
  }
  return value
}

export function resolveAcceptanceInstallationType({ published, prospective, env = process.env }) {
  if (
    typeof published !== 'string' ||
    !published ||
    typeof prospective !== 'string' ||
    !prospective
  ) {
    throw new Error('published and prospective installation types are required')
  }
  return resolveAcceptanceProvenance(env) === 'prospective-source' ? prospective : published
}
