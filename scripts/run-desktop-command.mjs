#!/usr/bin/env node

import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

import { restoreReleasePleaseAnnotation } from './desktop-lockfile.mjs'

const root = resolve(fileURLToPath(new URL('..', import.meta.url)))
const desktopRoot = resolve(root, 'apps/desktop')
const lockfile = resolve(desktopRoot, 'src-tauri/Cargo.lock')
const [command, ...args] = process.argv.slice(2)

if (!command) {
  throw new Error('usage: run-desktop-command.mjs COMMAND [ARGUMENT ...]')
}

let result
try {
  result = spawnSync(command, args, { cwd: desktopRoot, stdio: 'inherit' })
} finally {
  // Cargo may rewrite a lockfile while preserving its semantic contents but
  // dropping the Release Please marker comment. Restore that repository-owned
  // annotation after every desktop command that can invoke Cargo, including
  // Tauri builds, so local commands never leave a false dirty diff.
  restoreReleasePleaseAnnotation(lockfile)
}

if (result.error) throw result.error
if (result.signal) {
  console.error(`${command} terminated by ${result.signal}`)
  process.exit(1)
}
process.exit(result.status ?? 1)
