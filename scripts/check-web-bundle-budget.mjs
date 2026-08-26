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
  const entries = Object.values(manifest)
  const renderer = entries.find((entry) => entry.isEntry)
  const legacy = entries.find((entry) => entry.name === 'LegacyRenderer')
  const shadcn = entries.find(
    (entry) => entry.src?.endsWith('/ShadcnRenderer.tsx') || entry.name === 'ShadcnRenderer'
  )

  if (!renderer || !legacy || !shadcn) {
    throw new Error('Vite manifest is missing the renderer, legacy, or production shadcn assets')
  }

  const rendererKey = Object.entries(manifest).find(([, entry]) => entry === renderer)?.[0]
  const legacyKey = Object.entries(manifest).find(([, entry]) => entry === legacy)?.[0]
  const shadcnKey = Object.entries(manifest).find(([, entry]) => entry === shadcn)?.[0]
  const initialRendererKeys = staticImportKeys(manifest, [rendererKey])
  const initialLegacyKeys = staticImportKeys(manifest, [rendererKey, legacyKey])
  const shadcnKeys = staticImportKeys(manifest, [rendererKey, shadcnKey])
  const incrementalShadcnKeys = [...shadcnKeys].filter((key) => !initialRendererKeys.has(key))

  const measurements = [
    ['legacy-default initial JavaScript', uniqueAssetBytes(manifest, initialLegacyKeys), 475_000],
    ['legacy-default renderer CSS', uniqueCssBytes(manifest, initialLegacyKeys), 80_000],
    ['lazy production shadcn entry', size(shadcn.file), 50_000],
    [
      'production shadcn incremental JavaScript graph',
      uniqueAssetBytes(manifest, incrementalShadcnKeys),
      650_000,
    ],
    ['production shadcn CSS graph', uniqueCssBytes(manifest, shadcnKeys), 210_000],
  ]

  for (const [label, bytes, budget] of measurements) {
    assertWithinBudget(label, bytes, budget)
    console.log(`${label}: ${bytes}/${budget} bytes`)
  }
}

if (import.meta.main) verifyWebBundleBudget()
