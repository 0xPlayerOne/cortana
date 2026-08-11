import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { expect, test } from 'bun:test'

import {
  hasReleasePleaseAnnotation,
  restoreReleasePleaseAnnotation,
} from './desktop-lockfile.mjs'

const lockfile = `[[package]]
name = "cortana-desktop"
version = "0.30.3"
dependencies = []
`

test('restores a missing Release Please lockfile marker', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-lockfile-test-'))
  const path = join(directory, 'Cargo.lock')
  try {
    writeFileSync(path, lockfile)
    expect(hasReleasePleaseAnnotation(path)).toBe(false)

    restoreReleasePleaseAnnotation(path)

    const restored = readFileSync(path, 'utf8')
    expect(hasReleasePleaseAnnotation(path)).toBe(true)
    expect(restored).toContain('version = "0.30.3" # x-release-please-version')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('does not duplicate an existing Release Please marker', () => {
  const directory = mkdtempSync(join(tmpdir(), 'cortana-lockfile-test-'))
  const path = join(directory, 'Cargo.lock')
  const annotated = lockfile.replace(
    'version = "0.30.3"',
    'version = "0.30.3" # x-release-please-version'
  )
  try {
    writeFileSync(path, annotated)
    restoreReleasePleaseAnnotation(path)
    expect(readFileSync(path, 'utf8')).toBe(annotated)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
