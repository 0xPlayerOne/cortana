#!/usr/bin/env node

import { readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { buildTestGroups, resolveMaxParallel, scheduleGroups } from './js-test-groups.mjs'

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
  'SettingsView.demo.test.tsx',
  'SourceInitialSync.test.tsx',
  'sourceJobs.test.ts',
  'prepare-desktop-resources.test.mjs',
])
const exclusiveNames = new Set([
  'App.desktop.test.tsx',
  'App.utility.test.tsx',
  'SourceInitialSync.test.tsx',
  'sourceJobs.test.ts',
  'prepare-desktop-resources.test.mjs',
])
const groups = buildTestGroups(tests, isolatedNames)
const maxParallel = resolveMaxParallel(groups.length)

function runGroup(group) {
  const labels = group.map((test) => relative(root, join(root, test)))
  const started = performance.now()
  return new Promise((resolveResult) => {
    const child = spawn(bun, [...bunArgs, ...group], {
      cwd: root,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    child.once('error', (error) => {
      resolveResult({
        labels,
        code: 1,
        output: `Failed to start Bun: ${error.message}`,
        durationMs: performance.now() - started,
      })
    })
    child.once('close', (code) => {
      resolveResult({
        labels,
        code: code ?? 1,
        output: `${stdout}${stderr}`,
        durationMs: performance.now() - started,
      })
    })
  })
}

const results = Array(groups.length)
for (const batch of scheduleGroups(groups, maxParallel, exclusiveNames)) {
  const batchResults = await Promise.all(batch.map((index) => runGroup(groups[index])))
  batch.forEach((index, offset) => {
    results[index] = batchResults[offset]
  })
}

for (const result of results) {
  console.log(`\n▶ ${result.labels.join(' + ')} (${(result.durationMs / 1000).toFixed(2)}s)`)
  if (result.output) process.stdout.write(result.output)
  if (result.code !== 0) {
    console.error(`JavaScript test group failed: ${result.labels.join(', ')}`)
  }
}

const failed = results.filter((result) => result.code !== 0)
if (failed.length > 0) process.exit(failed[0].code || 1)

console.log(
  `\nPassed ${tests.length} JavaScript test files in ${maxParallel} parallel group(s); API-mock suites ran in isolated processes.`
)
