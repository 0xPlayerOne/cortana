#!/usr/bin/env node

import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const LOCK_WAIT_MS = 100
const LOCK_TIMEOUT_MS = 60_000
const LOCK_STALE_MS = 10 * 60_000

function sleep(milliseconds) {
  // Atomics.wait gives this short-lived build helper a synchronous sleep without
  // spawning another process or allowing a second writer into the swap window.
  const buffer = new SharedArrayBuffer(4)
  Atomics.wait(new Int32Array(buffer), 0, 0, milliseconds)
}

function acquireLock(lockPath) {
  const startedAt = Date.now()
  while (true) {
    try {
      mkdirSync(lockPath)
      writeFileSync(join(lockPath, 'owner'), `${process.pid}\n`, { flag: 'wx' })
      return () => rmSync(lockPath, { recursive: true, force: true })
    } catch (error) {
      // A process killed during a build must not strand every later desktop
      // check forever. Only reclaim a lock whose directory is demonstrably old.
      if (existsSync(lockPath)) {
        try {
          if (Date.now() - statSync(lockPath).mtimeMs > LOCK_STALE_MS) {
            rmSync(lockPath, { recursive: true, force: true })
            continue
          }
        } catch {
          // Another writer may be completing its swap; continue waiting.
        }
      }
      if (Date.now() - startedAt >= LOCK_TIMEOUT_MS) {
        throw new Error(`Timed out waiting for desktop resource lock: ${lockPath}`, {
          cause: error,
        })
      }
      sleep(LOCK_WAIT_MS)
    }
  }
}

export function prepareResources(root = resolve(dirname(fileURLToPath(import.meta.url)), '..')) {
  const source = resolve(root, 'src/cortana')
  const destination = resolve(root, 'apps/desktop/src-tauri/resources/cortana-connectors')
  const parent = dirname(destination)
  mkdirSync(parent, { recursive: true })
  const lockPath = `${destination}.lock`
  const releaseLock = acquireLock(lockPath)
  let staging = null
  let previous = null

  try {
    staging = mkdtempSync(join(parent, '.cortana-connectors.staging-'))
    writeFileSync(
      resolve(staging, '.gitkeep'),
      '# Generated connector resources are prepared before Tauri dev, test, and release builds.\n'
    )
    cpSync(resolve(root, 'pyproject.toml'), resolve(staging, 'pyproject.toml'))
    cpSync(resolve(root, 'README.md'), resolve(staging, 'README.md'))
    cpSync(resolve(root, 'LICENSE'), resolve(staging, 'LICENSE'))
    cpSync(source, resolve(staging, 'src/cortana'), {
      recursive: true,
      filter: (path) => {
        const normalized = path.replaceAll('\\', '/')
        return !normalized.includes('/__pycache__/') && !normalized.endsWith('.pyc')
      },
    })

    // Swap only fully materialized trees. The lock serializes the brief rename
    // window so parallel Tauri checks can never observe a deleted or partial
    // connector directory.
    if (existsSync(destination)) {
      previous = `${destination}.previous-${process.pid}-${Date.now()}`
      renameSync(destination, previous)
    }
    renameSync(staging, destination)
    staging = null
    if (previous) rmSync(previous, { recursive: true, force: true })
    console.log(`Prepared desktop connector resources: ${destination}`)
    return destination
  } catch (error) {
    if (staging) rmSync(staging, { recursive: true, force: true })
    // If the new tree could not be installed, restore the last known-good tree.
    if (previous && !existsSync(destination) && existsSync(previous)) {
      renameSync(previous, destination)
      previous = null
    }
    throw error
  } finally {
    if (previous) rmSync(previous, { recursive: true, force: true })
    releaseLock()
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  prepareResources()
}
