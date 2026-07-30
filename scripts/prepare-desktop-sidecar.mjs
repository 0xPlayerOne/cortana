#!/usr/bin/env node

import { chmodSync, copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

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
run('cargo', args)

const source = resolve(root, 'target', target, profile, `cortana${extension}`)
const destination = resolve(
  root,
  'apps/desktop/src-tauri/binaries',
  `cortana-${target}${extension}`
)
mkdirSync(dirname(destination), { recursive: true })
copyFileSync(source, destination)
if (!windows) chmodSync(destination, 0o755)
console.log(`Prepared desktop runtime sidecar: ${destination}`)

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.error}`)
  return result.stdout.trim()
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`${command} failed with status ${result.status}`)
}
