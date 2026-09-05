import { expect, test } from 'bun:test'

import { demoDocumentId, demoDocumentRelations, demoEvidence } from './demo'

test('demo relationship fixture points to canonical documents', () => {
  const deploymentPlaybook = demoEvidence.find((item) => item.source_id === 'deployment-playbook')
  expect(deploymentPlaybook).toBeDefined()

  const relations = demoDocumentRelations(deploymentPlaybook!)

  expect(relations.backlinks).toMatchObject([
    { id: demoDocumentId(demoEvidence[0]), title: 'How do releases work?' },
  ])
  expect(relations.surrounding).toMatchObject([
    { id: demoDocumentId(demoEvidence[3]), title: 'Incident response playbook' },
  ])
})
