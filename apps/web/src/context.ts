import type { Evidence } from './types'

export function estimateTokens(value: string): number {
  return Math.max(1, Math.ceil(value.length / 4))
}

export function buildAgentContext(query: string, evidence: Evidence[]): string {
  const sources = evidence.map((item, index) => {
    const location = item.uri ? ` (${item.uri})` : ''
    return [
      `### [${index + 1}] ${item.title}${location}`,
      `Source: ${item.source} · Updated: ${item.updated_at}`,
      item.content,
    ].join('\n')
  })
  return [
    '# Cortana evidence context',
    `Query: ${query}`,
    'Use only the evidence below for factual claims. Cite sources with [n].',
    '',
    ...sources,
  ].join('\n\n')
}
