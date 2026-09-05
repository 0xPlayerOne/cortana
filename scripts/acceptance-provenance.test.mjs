import { expect, test } from 'bun:test'

import {
  resolveAcceptanceInstallationType,
  resolveAcceptanceProvenance,
} from './acceptance-provenance.mjs'

test('acceptance provenance defaults to immutable published evidence', () => {
  expect(resolveAcceptanceProvenance({})).toBe('published')
  expect(
    resolveAcceptanceInstallationType({
      published: 'published-lane',
      prospective: 'prospective-lane',
      env: {},
    })
  ).toBe('published-lane')
})

test('acceptance provenance explicitly labels current-source evidence', () => {
  const env = { CORTANA_ACCEPTANCE_PROVENANCE: 'prospective-source' }
  expect(resolveAcceptanceProvenance(env)).toBe('prospective-source')
  expect(
    resolveAcceptanceInstallationType({
      published: 'published-lane',
      prospective: 'prospective-lane',
      env,
    })
  ).toBe('prospective-lane')
})

test('acceptance provenance fails closed on unknown values or missing labels', () => {
  expect(() => resolveAcceptanceProvenance({ CORTANA_ACCEPTANCE_PROVENANCE: 'untrusted' })).toThrow(
    'must be one of'
  )
  expect(() =>
    resolveAcceptanceInstallationType({ published: 'published-lane', prospective: '' })
  ).toThrow('required')
})
