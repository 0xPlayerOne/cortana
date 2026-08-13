import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { expect, test } from 'bun:test'

const script = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), 'prepare-desktop-sidecar.mjs'),
  'utf8'
)

test('sidecar preparation serializes writers and publishes atomically', () => {
  expect(script).toContain('function acquireLock()')
  expect(script).toContain('const releaseLock = acquireLock()')
  expect(script).toContain('mkdtempSync(join(sidecarDirectory')
  expect(script).toContain('renameSync(stagedDestination, destination)')
  expect(script).toContain('releaseLock()')
  expect(script).not.toContain('copyFileSync(source, destination)')
})
