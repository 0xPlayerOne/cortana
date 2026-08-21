import { spawn } from 'node:child_process'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { expect, test } from 'bun:test'

import { prepareResources } from './prepare-desktop-resources.mjs'

const PREPARE_SCRIPT = fileURLToPath(new URL('./prepare-desktop-resources.mjs', import.meta.url))

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
    // Native Rust memory is the only supported memory engine. Generated
    // connector resources must not resurrect the retired external-memory
    // package or its console entry points.
    expect(existsSync(join(destination, 'src', 'cortana', 'memory'))).toBe(false)
    expect(readFileSync(join(destination, 'pyproject.toml'), 'utf8')).not.toContain(
      'cortana-memory'
    )
    expect(readdirSync(join(root, 'apps', 'desktop', 'src-tauri', 'resources'))).toEqual([
      'cortana-connectors',
    ])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

function prepareInChildProcess(root) {
  const script = `import { prepareResources } from ${JSON.stringify(PREPARE_SCRIPT)}; prepareResources(${JSON.stringify(root)})`
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['--input-type=module', '-e', script], {
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    child.once('error', reject)
    child.once('close', (code) => {
      if (code === 0) {
        resolve(stdout)
      } else {
        reject(new Error(`resource preparation exited ${code}: ${stderr || stdout}`))
      }
    })
  })
}

test('serializes concurrent writers and leaves one complete published tree', async () => {
  const root = makeFixture()
  try {
    const source = join(root, 'src', 'cortana')
    for (let index = 0; index < 64; index += 1) {
      writeFileSync(join(source, `connector-${index}.py`), 'print("ok")\n'.repeat(4096))
    }

    const [first, second] = await Promise.all([
      prepareInChildProcess(root),
      prepareInChildProcess(root),
    ])
    expect(first).toContain('Prepared desktop connector resources')
    expect(second).toContain('Prepared desktop connector resources')

    const destination = join(
      root,
      'apps',
      'desktop',
      'src-tauri',
      'resources',
      'cortana-connectors'
    )
    expect(readFileSync(join(destination, 'src', 'cortana', 'connector-63.py'), 'utf8')).toContain(
      'print("ok")'
    )
    const resources = join(root, 'apps', 'desktop', 'src-tauri', 'resources')
    expect(existsSync(`${destination}.lock`)).toBe(false)
    expect(readdirSync(resources).filter((entry) => entry.includes('.staging-'))).toEqual([])
    expect(readdirSync(resources).filter((entry) => entry.includes('.previous-'))).toEqual([])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
