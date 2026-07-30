#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const crate = resolve(root, 'third_party/glib-0.18.5')
const variantIterator = resolve(crate, 'src/variant_iter.rs')
const manifest = resolve(crate, 'Cargo.toml')

verifyHash(variantIterator, 'a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc')
verifyHash(manifest, 'bcd52d812b4c111864ae8e88ba6c0a8311eb0cf781b5e81634950b4619592132')

const source = readFileSync(variantIterator, 'utf8')
if (
  !source.includes('let mut p: *mut libc::c_char = std::ptr::null_mut();') ||
  !source.includes('&mut p,') ||
  source.includes('\n                &p,\n')
) {
  throw new Error('vendored glib no longer contains the reviewed VariantStrIter backport')
}

const manifestBody = readFileSync(manifest, 'utf8')
if (!/^name = "glib"$/m.test(manifestBody) || !/^version = "0\.18\.5"$/m.test(manifestBody)) {
  throw new Error('vendored glib package identity changed')
}

console.log('Verified vendored glib 0.18.5 security backport')

function verifyHash(path, expected) {
  const actual = createHash('sha256').update(readFileSync(path)).digest('hex')
  if (actual !== expected) {
    throw new Error(`${path} hash mismatch: expected ${expected}, got ${actual}`)
  }
}
