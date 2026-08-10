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
  const source = readFileSync(path, 'utf8')
  const restored = source.replace(versionLine, '$1 # x-release-please-version\n')
  if (restored !== source) writeFileSync(path, restored)
}
