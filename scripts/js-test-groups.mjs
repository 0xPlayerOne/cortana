import { availableParallelism } from 'node:os'

// Two child processes are a safe default for Happy DOM and packaging tests;
// callers can raise or lower this explicitly when their runner has different
// CPU/memory characteristics.
const DEFAULT_MAX_PARALLEL = 2

/**
 * Keep API-mock suites in their own Bun process while grouping pure tests to
 * amortize process startup. The returned order is stable for reproducible
 * output and deterministic debugging when the concurrency cap is one.
 */
export function buildTestGroups(tests, isolatedNames) {
  const isolated = new Set(isolatedNames)
  return [
    tests.filter((test) => !isolated.has(test.split(/[\\/]/).pop())),
    ...tests.filter((test) => isolated.has(test.split(/[\\/]/).pop())).map((test) => [test]),
  ].filter((group) => group.length > 0)
}

/**
 * Schedule lightweight groups in bounded batches and run resource-heavy
 * groups exclusively. This avoids starving Happy DOM and packaging fixtures
 * while still parallelizing the independent API-mock suites.
 */
export function scheduleGroups(groups, maxParallel, exclusiveNames) {
  const exclusive = new Set(exclusiveNames)
  const pending = groups.map((_, index) => index)
  const batches = []
  while (pending.length > 0) {
    const isExclusive = (index) =>
      groups[index].some((test) => exclusive.has(test.split(/[\\/]/).pop()))
    const lightIndex = pending.findIndex((index) => !isExclusive(index))
    if (lightIndex >= 0) {
      const light = pending.splice(lightIndex, 1)[0]
      const additional = []
      while (additional.length < maxParallel - 1) {
        const nextLight = pending.findIndex((index) => !isExclusive(index))
        if (nextLight < 0) break
        additional.push(pending.splice(nextLight, 1)[0])
      }
      batches.push([light, ...additional])
      continue
    }
    batches.push([pending.shift()])
  }
  return batches
}

/**
 * Resolve the process-level concurrency cap. An explicit environment value is
 * useful for local debugging and constrained CI runners; otherwise use a
 * conservative cap that leaves at least one core for the coordinator.
 */
export function resolveMaxParallel(groupCount, value = process.env.CORTANA_TEST_MAX_PARALLEL) {
  if (groupCount < 1) return 0
  if (value !== undefined) {
    const parsed = Number(value)
    if (!Number.isInteger(parsed) || parsed < 1) {
      throw new Error('CORTANA_TEST_MAX_PARALLEL must be a positive integer')
    }
    return Math.min(groupCount, parsed)
  }
  return Math.min(groupCount, Math.max(1, Math.min(DEFAULT_MAX_PARALLEL, availableParallelism())))
}
