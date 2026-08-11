import { readFileSync, writeFileSync } from 'node:fs'

const annotation = /name = "cortana-desktop"\nversion = "[^"]+" # x-release-please-version\n/
const versionLine = /(name = "cortana-desktop"\nversion = "[^"]+")(?! # x-release-please-version)\n/

export function hasReleasePleaseAnnotation(path) {
  try {
    return annotation.test(readFileSync(path, 'utf8'))
  } catch {
    return false
  }
}

export function restoreReleasePleaseAnnotation(path) {
  let source
  try {
    source = readFileSync(path, 'utf8')
  } catch (error) {
    // A fresh checkout may not have generated the desktop lockfile yet. The
    // wrapper must preserve that pre-Cargo behavior while still surfacing
    // permission, encoding, and other real filesystem failures.
    if (error?.code === 'ENOENT') return
    throw error
  }
  const restored = source.replace(versionLine, '$1 # x-release-please-version\n')
  if (restored !== source) writeFileSync(path, restored)
}
