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
  const prototype = entries.find(
    (entry) => entry.src?.endsWith('/M7ShadcnPrototype.tsx') || entry.name === 'M7ShadcnPrototype'
  )

  if (!renderer || !legacy || !prototype) {
    throw new Error('Vite manifest is missing the renderer, legacy, or lazy M7 prototype assets')
  }

  const legacyCss = legacy.css?.[0]
  const prototypeCss = prototype.css?.[0]
  if (!legacyCss || !prototypeCss) {
    throw new Error('Vite manifest is missing the renderer CSS assets')
  }

  const rendererKey = Object.entries(manifest).find(([, entry]) => entry === renderer)?.[0]
  const legacyKey = Object.entries(manifest).find(([, entry]) => entry === legacy)?.[0]
  const prototypeKey = Object.entries(manifest).find(([, entry]) => entry === prototype)?.[0]
  const initialRendererKeys = staticImportKeys(manifest, [rendererKey])
  const initialLegacyKeys = staticImportKeys(manifest, [rendererKey, legacyKey])
  const prototypeKeys = staticImportKeys(manifest, [prototypeKey])
  const incrementalPrototypeKeys = [...prototypeKeys].filter((key) => !initialRendererKeys.has(key))

  const measurements = [
    ['legacy-default initial JavaScript', uniqueAssetBytes(manifest, initialLegacyKeys), 475_000],
    ['legacy-default renderer CSS', size(legacyCss), 80_000],
    ['lazy shadcn prototype entry', size(prototype.file), 220_000],
    [
      'lazy shadcn incremental JavaScript graph',
      uniqueAssetBytes(manifest, incrementalPrototypeKeys),
      330_000,
    ],
    ['lazy shadcn prototype CSS', size(prototypeCss), 125_000],
  ]

  for (const [label, bytes, budget] of measurements) {
    assertWithinBudget(label, bytes, budget)
    console.log(`${label}: ${bytes}/${budget} bytes`)
  }
}

if (import.meta.main) verifyWebBundleBudget()
