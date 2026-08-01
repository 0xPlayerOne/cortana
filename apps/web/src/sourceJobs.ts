import { useCallback, useEffect, useRef, useState } from 'react'

import { getDesktopSourceValidation, isDesktopApp } from './api'
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
  return [next, ...jobs.filter((job) => job.id !== next.id)].slice(0, MAX_SOURCE_JOB_SNAPSHOTS)
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

/**
 * Owns the cross-view source-job snapshot list. SettingsView reports started
 * jobs through remember(); while the view is unmounted the list survives here
 * and only active ids are polled. A missing-job error drops the id; any other
 * error retains the last snapshot.
 */
export function useSourceJobs() {
  const [jobs, setJobs] = useState<DesktopSourceJob[]>([])
  const jobsRef = useRef(jobs)
  jobsRef.current = jobs

  const remember = useCallback((job: DesktopSourceJob) => {
    setJobs((current) => upsertJob(current, job))
  }, [])

  useEffect(() => {
    if (!isDesktopApp) return
    let disposed = false
    const timer = window.setInterval(() => {
      const ids = activeJobIds(jobsRef.current)
      if (ids.length === 0) return
      void Promise.allSettled(ids.map((id) => getDesktopSourceValidation(id))).then((results) => {
        if (disposed) return
        results.forEach((result, index) => {
          if (result.status === 'fulfilled') {
            setJobs((current) => upsertJob(current, result.value))
          } else if (isMissingJobError(result.reason)) {
            setJobs((current) => dropJob(current, ids[index]))
          }
          // Any other error retains the last snapshot: the job may still be running.
        })
      })
    }, SOURCE_JOB_POLL_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [])

  return { jobs, remember, track: remember }
}
