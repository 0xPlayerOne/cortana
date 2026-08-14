#!/usr/bin/env node

import { readdirSync } from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const testRoots = ['apps/web/src', 'scripts']
const testSuffixes = ['.test.ts', '.test.tsx', '.test.mjs']

function collectTests(directory) {
  const entries = readdirSync(join(root, directory), { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name === 'target') {
      continue
    }
    const path = join(directory, entry.name)
    if (entry.isDirectory()) {
      files.push(...collectTests(path))
      continue
    }
    if (testSuffixes.some((suffix) => entry.name.endsWith(suffix))) {
      files.push(path)
    }
  }
  return files
}

const tests = testRoots.flatMap(collectTests).sort()
if (tests.length === 0) {
  console.error('No JavaScript test files were found.')
  process.exit(1)
}

// Bun's module mocks are process-global. These suites replace the shared API
// bridge and must not share a worker with another replacement. The remaining
// pure/component tests can stay grouped to avoid paying process-startup cost
// for every small unit file.
const bun = process.env.BUN_BIN || (process.versions.bun ? process.execPath : 'bun')
const bunArgs = [
  'test',
  '--isolate',
  '--parallel=1',
  '--max-concurrency=1',
  '--timeout=20000',
  '--reporter',
  'dots',
]
const isolatedNames = new Set([
  'App.desktop.test.tsx',
  'App.test.tsx',
  'App.utility.test.tsx',
  'BuzzCommunities.test.tsx',
  'DiscordChannels.test.tsx',
  'DiscordServers.test.tsx',
  'ProviderModels.test.tsx',
  'SlackWorkspaces.test.tsx',
  'SourceInitialSync.test.tsx',
  'sourceJobs.test.ts',
])
const groups = [
  tests.filter((test) => !isolatedNames.has(basename(test))),
  ...tests.filter((test) => isolatedNames.has(basename(test))).map((test) => [test]),
].filter((group) => group.length > 0)

for (const group of groups) {
  const labels = group.map((test) => relative(root, join(root, test)))
  console.log(`\n▶ ${labels.join(' + ')}`)
  const result = spawnSync(bun, [...bunArgs, ...group], {
    cwd: root,
    env: process.env,
    stdio: 'inherit',
  })
  if (result.error) {
    console.error(`Failed to start Bun for ${label}: ${result.error.message}`)
    process.exit(1)
  }
  if (result.status !== 0) {
    console.error(`JavaScript test group failed: ${labels.join(', ')}`)
    process.exit(result.status ?? 1)
  }
}

console.log(`\nPassed ${tests.length} isolated JavaScript test files.`)
