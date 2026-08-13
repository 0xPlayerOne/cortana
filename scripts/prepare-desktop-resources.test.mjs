import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { expect, test } from 'bun:test'

import { prepareResources } from './prepare-desktop-resources.mjs'

function makeFixture() {
  const root = mkdtempSync(join(tmpdir(), 'cortana-resource-test-'))
  writeFileSync(join(root, 'pyproject.toml'), '[project]\nname = "fixture"\n')
  writeFileSync(join(root, 'README.md'), 'fixture\n')
  writeFileSync(join(root, 'LICENSE'), 'fixture\n')
  writeFileSync(join(root, 'src-placeholder'), 'fixture\n')
  // The helper only needs the source tree and the three copied metadata files.
  const source = join(root, 'src', 'cortana')
  mkdirSync(source, { recursive: true })
  writeFileSync(join(source, 'connector.py'), 'print("ok")\n')
  return root
}

test('prepares a complete connector tree and leaves no staging directories', () => {
  const root = makeFixture()
  try {
    const destination = prepareResources(root)
    expect(readFileSync(join(destination, 'pyproject.toml'), 'utf8')).toContain('fixture')
    expect(readFileSync(join(destination, 'src', 'cortana', 'connector.py'), 'utf8')).toContain(
      'ok'
    )
    expect(readdirSync(join(root, 'apps', 'desktop', 'src-tauri', 'resources'))).toEqual([
      'cortana-connectors',
    ])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
