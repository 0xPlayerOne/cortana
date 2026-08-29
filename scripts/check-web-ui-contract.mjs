#!/usr/bin/env node

import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { resolve } from 'node:path'

const root = resolve(import.meta.dir, '..')
const sourceRoot = resolve(root, 'apps/web/src')

const removedFiles = [
  'LegacyRenderer.tsx',
  'ShadcnRenderer.tsx',
  'rendererMode.ts',
  'components/Navigation.tsx',
  'components/ui/Button.tsx',
  'components/ui/buttonClasses.ts',
  'components/m7/M7SurfacePrimitives.tsx',
  'components/m7/M7SurfacePrimitives.shadcn.ts',
  'styles.css',
  'styles/buttons.css',
  'styles/context.css',
  'styles/responsive.css',
  'styles/settings.css',
  'styles/shell.css',
  'styles/tokens.css',
  'styles/utility.css',
  'styles/workspace.css',
]

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) return sourceFiles(path)
    return /\.(?:css|ts|tsx)$/.test(entry.name) && !/\.test\./.test(entry.name) ? [path] : []
  })
}

export function normalizePathSeparators(path) {
  return path.replaceAll('\\', '/')
}

function relativeSourcePath(file) {
  return normalizePathSeparators(file.slice(sourceRoot.length + 1))
}

function isShadcnFile(file) {
  return relativeSourcePath(file).startsWith('components/shadcn/')
}

export function verifyWebUiContract() {
  const failures = []
  for (const relative of removedFiles) {
    if (existsSync(resolve(sourceRoot, relative))) failures.push(`${relative} must stay removed`)
  }

  const files = sourceFiles(sourceRoot)
  const forbidden = [
    ['legacy button class', /cortana-button/],
    ['legacy pseudo-tooltip contract', /quick-tooltip|data-tooltip/],
    ['renderer mode flag', /VITE_CORTANA_RENDERER|data-cortana-renderer|data-renderer=/],
    ['renderer adapter', /LegacyRenderer|ShadcnRenderer|M7SurfacePrimitives/],
  ]
  for (const file of files) {
    const source = readFileSync(file, 'utf8')
    for (const [label, pattern] of forbidden) {
      if (pattern.test(source))
        failures.push(`${normalizePathSeparators(file.slice(root.length + 1))} contains ${label}`)
    }
    if (!isShadcnFile(file)) {
      for (const element of ['input', 'select', 'textarea']) {
        if (new RegExp(`<${element}\\b`).test(source)) {
          failures.push(`${relativeSourcePath(file)} contains raw <${element}>`)
        }
      }
    }
  }

  const approvedRawButtons = new Map([
    ['components/RendererErrorBoundary.tsx', 1],
    ['components/Workspace.tsx', 1],
  ])
  for (const [relative, expected] of approvedRawButtons) {
    const source = readFileSync(resolve(sourceRoot, relative), 'utf8')
    const count = source.match(/<button\b/g)?.length ?? 0
    if (count !== expected)
      failures.push(`${relative} has ${count}/${expected} approved raw buttons`)
  }
  for (const file of files) {
    if (isShadcnFile(file)) continue
    const relative = relativeSourcePath(file)
    if (approvedRawButtons.has(relative)) continue
    if (/<button\b/.test(readFileSync(file, 'utf8')))
      failures.push(`${relative} contains raw <button>`)
  }

  if (failures.length > 0) throw new Error(`Web UI contract failed:\n${failures.join('\n')}`)
  console.log('Web UI contract: single shadcn renderer with approved custom-renderer exceptions')
}

if (import.meta.main) verifyWebUiContract()
