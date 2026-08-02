import { useCallback, useEffect, useRef, useState } from 'react'

import { getDesktopSourceJobs, getDesktopSourceValidation, isDesktopApp } from './api'
import type { DesktopSourceJob } from './types'

/** Bounded snapshot list: the most recent job snapshots, newest first. */
export const MAX_SOURCE_JOB_SNAPSHOTS = 8

/** Polling cadence for active source jobs. */
export const SOURCE_JOB_POLL_MS = 1000

export function isActiveJob(job: DesktopSourceJob): boolean {
  return job.status === 'running' || job.status === 'cancelling'
}

export function describeSourceJob(job: DesktopSourceJob): string {
  return `${job.source} · ${job.operation} · ${job.status}`
}

/** Native error raised by desktop_source_validation_status for an unknown id. */
export function isMissingJobError(error: unknown): boolean {
  return error instanceof Error && error.message.includes('source job was not found')
}

/** Pure upsert: the latest snapshot is placed first and the list stays bounded. */
export function upsertJob(jobs: DesktopSourceJob[], next: DesktopSourceJob): DesktopSourceJob[] {
  const existing = jobs.find((job) => job.id === next.id)
  // A poll can resolve after the action boundary has already remembered a
  // newer result. Never let that older in-flight snapshot make a completed
  // job look active again or replace its terminal summary in the shell.
  if (existing && snapshotRegresses(existing, next)) return jobs
  return [next, ...jobs.filter((job) => job.id !== next.id)].slice(0, MAX_SOURCE_JOB_SNAPSHOTS)
}

/** Merge a native recovery snapshot without regressing a newer renderer state. */
export function mergeJobSnapshots(
  jobs: DesktopSourceJob[],
  recovered: DesktopSourceJob[]
): DesktopSourceJob[] {
  // Native snapshots are newest-first. Apply them oldest-first so the final
  // list keeps the same newest-first ordering as upsertJob while still using
  // its stale-poll protection for ids already remembered by the shell.
  return [...recovered].reverse().reduce(upsertJob, jobs)
}

function snapshotRegresses(existing: DesktopSourceJob, next: DesktopSourceJob): boolean {
  if (existing.completed_at_unix_seconds !== null) {
    if (next.completed_at_unix_seconds === null) return true
    if (next.completed_at_unix_seconds < existing.completed_at_unix_seconds) return true
  }
  return existing.status === 'cancelling' && next.status === 'running'
}

export function dropJob(jobs: DesktopSourceJob[], id: string): DesktopSourceJob[] {
  return jobs.filter((job) => job.id !== id)
}

export function activeJobs(jobs: DesktopSourceJob[]): DesktopSourceJob[] {
  return jobs.filter(isActiveJob)
}

export function activeJobIds(jobs: DesktopSourceJob[]): string[] {
  return activeJobs(jobs).map((job) => job.id)
}

/** Terminal snapshots retained for the Inbox's bounded operational history. */
export function recentCompletedJobs(jobs: DesktopSourceJob[]): DesktopSourceJob[] {
  return jobs.filter((job) => !isActiveJob(job))
}

/** Terminal source failures whose latest result for that workspace/source needs attention. */
export function sourceJobAttention(jobs: DesktopSourceJob[]): DesktopSourceJob[] {
  const seenSources = new Set<string>()
  return jobs.filter((job) => {
    const scope = `${job.project}\u0000${job.source}`
    if (seenSources.has(scope)) return false
    seenSources.add(scope)
    return !isActiveJob(job) && (job.status === 'failed' || job.status === 'cancelled')
  })
}

/** Return the fixed wall-clock budget enforced by the native job boundary. */
export function sourceJobBudgetSeconds(job: DesktopSourceJob): number | null {
  // Google OAuth uses a five-minute native loopback callback timeout. Keep
  // authorization activity bounded in the shell just like validation and
  // sync, while the token exchange remains covered by the sidecar timeout.
  if (job.operation === 'authorization') return 5 * 60
  if (job.operation === 'validation') {
    return job.budget === 'small'
      ? 15 * 60
      : job.budget === 'medium'
        ? 30 * 60
        : job.budget === 'large'
          ? 60 * 60
          : 60
  }
  if (job.operation === 'trial-sync') return 5 * 60
  if (job.operation === 'initial-sync') {
    return job.budget === 'small'
      ? 15 * 60
      : job.budget === 'medium'
        ? 30 * 60
        : job.budget === 'large'
          ? 60 * 60
          : null
  }
  return null
}

export function sourceJobElapsedSeconds(
  job: DesktopSourceJob,
  nowSeconds = Math.floor(Date.now() / 1000)
): number {
  const end = job.completed_at_unix_seconds ?? nowSeconds
  return Math.max(0, end - job.started_at_unix_seconds)
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const remainder = seconds % 60
  return remainder === 0 ? `${minutes}m` : `${minutes}m ${remainder}s`
}

/** Human-readable elapsed/budget telemetry for active-job surfaces. */
export function describeSourceJobProgress(
  job: DesktopSourceJob,
  nowSeconds = Math.floor(Date.now() / 1000)
): string {
  const elapsed = formatDuration(sourceJobElapsedSeconds(job, nowSeconds))
  const budget = sourceJobBudgetSeconds(job)
  return budget === null ? `${elapsed} elapsed` : `${elapsed} / ${formatDuration(budget)}`
}

/**
 * Owns the cross-view source-job snapshot list. SettingsView reports started
 * jobs through remember(); while the view is unmounted the list survives here
 * and only active ids are polled. A missing-job error drops the id; any other
 * error retains the last snapshot.
 */
export function useSourceJobs() {
  const [jobs, setJobs] = useState<DesktopSourceJob[]>([])
  const [error, setError] = useState('')
  const jobsRef = useRef(jobs)
  const pollingRef = useRef(false)
  jobsRef.current = jobs

  const remember = useCallback((job: DesktopSourceJob) => {
    setError('')
    setJobs((current) => upsertJob(current, job))
  }, [])

  useEffect(() => {
    if (!isDesktopApp) return
    let disposed = false
    void getDesktopSourceJobs()
      .then((next) => {
        if (disposed) return
        setJobs((current) => mergeJobSnapshots(current, next))
      })
      .catch(() => {
        // A fresh native process may have no source-job state yet. The
        // renderer can still discover jobs started during this session via
        // remember(), so an unavailable recovery snapshot is non-fatal.
      })
    const timer = window.setInterval(() => {
      if (pollingRef.current) return
      const ids = activeJobIds(jobsRef.current)
      if (ids.length === 0) {
        setError('')
        return
      }
      pollingRef.current = true
      void Promise.allSettled(ids.map((id) => getDesktopSourceValidation(id)))
        .then((results) => {
          if (disposed) return
          let nextError: string | null = null
          results.forEach((result, index) => {
            if (result.status === 'fulfilled') {
              setJobs((current) => upsertJob(current, result.value))
            } else if (isMissingJobError(result.reason)) {
              setJobs((current) => dropJob(current, ids[index]))
            } else if (result.reason instanceof Error) {
              nextError = result.reason.message || 'Source job status unavailable'
            } else {
              nextError = 'Source job status unavailable'
            }
          })
          if (nextError) setError(nextError)
          else if (
            results.every(
              (result) =>
                result.status === 'fulfilled' ||
                isMissingJobError(result.status === 'rejected' ? result.reason : undefined)
            )
          ) {
            setError('')
          }
        })
        .finally(() => {
          pollingRef.current = false
        })
    }, SOURCE_JOB_POLL_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [])

  return { jobs, remember, track: remember, error }
}
