import type { BrainGraphPage } from './types'

const graphNodeKinds = new Set<BrainGraphPage['nodes'][number]['kind']>([
  'workspace',
  'source',
  'document',
  'chunk',
  'entity',
  'memory',
  'observation',
  'mental-model',
  'repository',
  'file',
  'symbol',
])

const graphEdgeKinds = new Set<BrainGraphPage['edges'][number]['kind']>([
  'contains',
  'references',
  'backlink',
  'nearby',
  'same-thread',
  'authored-by',
  'mentions',
  'temporal',
  'semantically-related',
  'supports',
  'contradicts',
  'reinforces',
  'supersedes',
  'observes',
  'derives',
  'depends-on',
  'defines',
  'calls',
  'imports',
])

const graphEdgeOrigins = new Set<NonNullable<BrainGraphPage['edges'][number]['origin']>>([
  'explicit',
  'derived',
  'inferred',
])

function isBoundedStringList(value: unknown, maxItems: number): value is string[] {
  return (
    Array.isArray(value) &&
    value.length <= maxItems &&
    value.every((item) => typeof item === 'string' && item.length > 0 && item.length <= 2_048)
  )
}

export function parseBrainGraphPage(value: unknown): BrainGraphPage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Graph response was malformed')
  }
  const page = value as Record<string, unknown>
  if (
    !Array.isArray(page['nodes']) ||
    page['nodes'].length > 200 ||
    !Array.isArray(page['edges']) ||
    page['edges'].length > 400 ||
    !(page['next_cursor'] === null || typeof page['next_cursor'] === 'string')
  ) {
    throw new Error('Graph response was malformed')
  }
  const nodeIds = new Set<string>()
  for (const candidate of page['nodes']) {
    if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) {
      throw new Error('Graph response was malformed')
    }
    const node = candidate as Record<string, unknown>
    if (
      typeof node['id'] !== 'string' ||
      node['id'].length === 0 ||
      node['id'].length > 2_048 ||
      typeof node['label'] !== 'string' ||
      node['label'].length > 512 ||
      typeof node['project'] !== 'string' ||
      node['project'].length > 512 ||
      !(node['source'] === null || typeof node['source'] === 'string') ||
      !(node['document_id'] === null || typeof node['document_id'] === 'string') ||
      !graphNodeKinds.has(node['kind'] as BrainGraphPage['nodes'][number]['kind']) ||
      nodeIds.has(node['id'])
    ) {
      throw new Error('Graph response was malformed')
    }
    nodeIds.add(node['id'])
  }
  for (const candidate of page['edges']) {
    if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) {
      throw new Error('Graph response was malformed')
    }
    const edge = candidate as Record<string, unknown>
    const confidence = edge['confidence']
    const origin = edge['origin']
    const support = edge['support']
    if (
      typeof edge['source'] !== 'string' ||
      typeof edge['target'] !== 'string' ||
      !nodeIds.has(edge['source']) ||
      !nodeIds.has(edge['target']) ||
      !graphEdgeKinds.has(edge['kind'] as BrainGraphPage['edges'][number]['kind']) ||
      !(
        origin === undefined ||
        graphEdgeOrigins.has(origin as NonNullable<BrainGraphPage['edges'][number]['origin']>)
      ) ||
      !(
        confidence === undefined ||
        confidence === null ||
        (typeof confidence === 'number' &&
          Number.isFinite(confidence) &&
          confidence >= 0 &&
          confidence <= 1)
      ) ||
      !(
        support === undefined ||
        (support !== null &&
          typeof support === 'object' &&
          !Array.isArray(support) &&
          isBoundedStringList((support as Record<string, unknown>)['record_ids'], 256) &&
          isBoundedStringList((support as Record<string, unknown>)['invalidation_keys'], 256))
      )
    ) {
      throw new Error('Graph response was malformed')
    }
  }
  return value as BrainGraphPage
}
