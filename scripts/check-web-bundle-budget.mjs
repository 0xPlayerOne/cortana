#!/usr/bin/env node

import { readFileSync, statSync } from 'node:fs'
import { resolve } from 'node:path'

const root = resolve(import.meta.dir, '..')
const dist = resolve(root, 'apps/web/dist')

export function assertWithinBudget(label, bytes, budget) {
  if (bytes > budget) {
    throw new Error(`${label} is ${bytes} bytes; budget is ${budget} bytes`)
  }
}

function size(path) {
  return statSync(resolve(dist, path)).size
}

function uniqueAssetBytes(manifest, keys) {
  const files = new Set([...keys].map((key) => manifest[key]?.file).filter(Boolean))
  return [...files].reduce((total, file) => total + size(file), 0)
}

function uniqueCssBytes(manifest, keys) {
  const files = new Set([...keys].flatMap((key) => manifest[key]?.css ?? []))
  return [...files].reduce((total, file) => total + size(file), 0)
}

export function staticImportKeys(manifest, roots) {
  const visited = new Set()
  const pending = [...roots]
  while (pending.length > 0) {
    const key = pending.pop()
    if (!key || visited.has(key) || !manifest[key]) continue
    visited.add(key)
    pending.push(...(manifest[key].imports ?? []))
  }
  return visited
}

export function verifyWebBundleBudget() {
  const manifest = JSON.parse(readFileSync(resolve(dist, '.vite/manifest.json'), 'utf8'))
  const entryKey = Object.entries(manifest).find(([, entry]) => entry.isEntry)?.[0]
  const appKey = Object.keys(manifest).find((key) => key.startsWith('_App-'))
  if (!entryKey || !appKey) throw new Error('Vite manifest is missing the application entry')

  const initialKeys = staticImportKeys(manifest, [entryKey, appKey])
  const productionKeys = new Set(
    Object.keys(manifest).filter(
      (key) => key !== 'src/demoDesktop.ts' && manifest[key]?.file?.endsWith('.js')
    )
  )

  const measurements = [
    ['initial application JavaScript graph', uniqueAssetBytes(manifest, initialKeys), 800_000],
    // M10 adds the optional vault-management surface and a graph-response
    // validator in a lazy chunk. Keep the startup ceiling fixed while
    // bounding the complete shipped graph near the reviewed 955,954 bytes.
    ['complete production JavaScript graph', uniqueAssetBytes(manifest, productionKeys), 960_000],
    // The knowledge graph, vault picker, and accessibility states extend the
    // shared stylesheet to 217,515 bytes in the reviewed M10 build.
    ['application CSS graph', uniqueCssBytes(manifest, productionKeys), 220_000],
  ]

  for (const [label, bytes, budget] of measurements) {
    assertWithinBudget(label, bytes, budget)
    console.log(`${label}: ${bytes}/${budget} bytes`)
  }
}

if (import.meta.main) verifyWebBundleBudget()
