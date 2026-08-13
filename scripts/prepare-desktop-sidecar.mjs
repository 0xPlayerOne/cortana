#!/usr/bin/env node

import {
  chmodSync,
  copyFileSync,
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
import { spawnSync } from 'node:child_process'

import { restoreReleasePleaseAnnotation } from './desktop-lockfile.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const release = process.argv.includes('--release')
const target = process.env.CORTANA_DESKTOP_TARGET || capture('rustc', ['--print', 'host-tuple'])
if (!/^[a-z0-9_]+-[a-z0-9_.-]+$/i.test(target)) {
  throw new Error(`invalid desktop target triple: ${target}`)
}
const windows = target.includes('windows')
const extension = windows ? '.exe' : ''
const profile = release ? 'release' : 'debug'
const args = ['build', '--locked', '--target', target]
if (release) args.push('--release')
const desktopLockfile = resolve(root, 'apps/desktop/src-tauri/Cargo.lock')
const sidecarDirectory = resolve(root, 'apps/desktop/src-tauri/binaries')
const lockPath = `${sidecarDirectory}.lock`
const lockWaitMs = 100
const lockTimeoutMs = 60_000
const lockStaleMs = 10 * 60_000

function acquireLock() {
  const startedAt = Date.now()
  mkdirSync(sidecarDirectory, { recursive: true })
  while (true) {
    try {
      mkdirSync(lockPath)
      writeFileSync(resolve(lockPath, 'owner'), `${process.pid}\n`, { flag: 'wx' })
      return () => rmSync(lockPath, { recursive: true, force: true })
    } catch (error) {
      if (existsSync(lockPath)) {
        try {
          if (Date.now() - statSync(lockPath).mtimeMs > lockStaleMs) {
            rmSync(lockPath, { recursive: true, force: true })
            continue
          }
        } catch {
          // Another process may be completing the lock acquisition.
        }
      }
      if (Date.now() - startedAt >= lockTimeoutMs) {
        throw new Error(`Timed out waiting for desktop sidecar lock: ${lockPath}`, { cause: error })
      }
      sleep(lockWaitMs)
    }
  }
}

function sleep(milliseconds) {
  const buffer = new SharedArrayBuffer(4)
  Atomics.wait(new Int32Array(buffer), 0, 0, milliseconds)
}

const releaseLock = acquireLock()
let stagingDirectory = null
try {
  run('cargo', args)
  // Cargo may rewrite a lockfile while preserving its semantic contents but
  // dropping the Release Please marker comment. Keep the generated lockfile
  // authoritative while restoring that repository-owned release annotation so
  // local desktop tests and builds do not leave a false dirty diff.
  restoreReleasePleaseAnnotation(desktopLockfile)
  const source = resolve(root, 'target', target, profile, `cortana${extension}`)
  const destination = resolve(
    root,
    'apps/desktop/src-tauri/binaries',
    `cortana-${target}${extension}`
  )
  stagingDirectory = mkdtempSync(join(sidecarDirectory, '.cortana-sidecar-staging-'))
  const stagedDestination = resolve(stagingDirectory, `cortana-${target}${extension}`)
  copyFileSync(source, stagedDestination)
  if (!windows) chmodSync(stagedDestination, 0o755)
  renameSync(stagedDestination, destination)
  console.log(`Prepared desktop runtime sidecar: ${destination}`)
} finally {
  if (stagingDirectory) rmSync(stagingDirectory, { recursive: true, force: true })
  // Cargo failure or a copy error must not leave the shared lock behind.
  releaseLock()
}

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.error}`)
  return result.stdout.trim()
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`${command} failed with status ${result.status}`)
}
