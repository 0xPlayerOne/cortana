#!/usr/bin/env node

import { cpSync, mkdirSync, rmSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const source = resolve(root, 'src/cortana')
const destination = resolve(root, 'apps/desktop/src-tauri/resources/cortana-connectors')

rmSync(destination, { recursive: true, force: true })
mkdirSync(destination, { recursive: true })
cpSync(resolve(root, 'pyproject.toml'), resolve(destination, 'pyproject.toml'))
cpSync(resolve(root, 'README.md'), resolve(destination, 'README.md'))
cpSync(resolve(root, 'LICENSE'), resolve(destination, 'LICENSE'))
cpSync(source, resolve(destination, 'src/cortana'), {
  recursive: true,
  filter: (path) => {
    const normalized = path.replaceAll('\\', '/')
    return !normalized.includes('/__pycache__/') && !normalized.endsWith('.pyc')
  },
})

console.log(`Prepared desktop connector resources: ${destination}`)
