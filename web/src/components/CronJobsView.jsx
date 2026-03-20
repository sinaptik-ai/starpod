import React, { useState, useEffect } from 'react'
import { useApp } from '../contexts/AppContext'
import { useUser } from './AuthGate'
import { authHeaders } from '../lib/api'

function formatEpoch(epoch) {
  if (!epoch) return '—'
  const d = new Date(epoch * 1000)
  const now = new Date()
  const diff = now - d
  const mins = Math.floor(diff / 60000)
  if (mins >= 0 && mins < 1) return 'just now'
  if (mins > 0 && mins < 60) return mins + 'm ago'
  const hrs = Math.floor(mins / 60)
  if (hrs > 0 && hrs < 24) return hrs + 'h ago'
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

function formatNextRun(epoch) {
  if (!epoch) return '—'
  const d = new Date(epoch * 1000)
  const now = new Date()
  const diff = d - now
  if (diff < 0) return 'overdue'
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'now'
  if (mins < 60) return 'in ' + mins + 'm'
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return 'in ' + hrs + 'h'
  const days = Math.floor(hrs / 24)
  return 'in ' + days + 'd'
}

function formatSchedule(schedule) {
  if (!schedule) return '—'
  switch (schedule.kind) {
    case 'cron': return schedule.expr
    case 'interval': {
      const ms = schedule.every_ms
      if (ms >= 86400000) return 'every ' + Math.round(ms / 86400000) + 'd'
      if (ms >= 3600000) return 'every ' + Math.round(ms / 3600000) + 'h'
      if (ms >= 60000) return 'every ' + Math.round(ms / 60000) + 'm'
      return 'every ' + Math.round(ms / 1000) + 's'
    }
    case 'one_shot': return 'once @ ' + schedule.at
    default: return '—'
  }
}

export default function CronJobsView() {
  const { dispatch } = useApp()
  const { isAdmin } = useUser()
  const [jobs, setJobs] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)

  useEffect(() => {
    fetch('/api/cron/jobs', { headers: authHeaders() })
      .then(r => {
        if (!r.ok) throw new Error(r.statusText)
        return r.json()
      })
      .then(data => {
        setJobs(data || [])
        setLoading(false)
      })
      .catch(e => {
        setError(e.message)
        setLoading(false)
      })
  }, [])

  return (
    <div className="flex flex-col h-[100dvh] bg-bg">
      {/* Fixed header */}
      <div className="shrink-0 border-b border-border-subtle">
        <div className="max-w-[740px] mx-auto px-5">
          <div className="flex items-center gap-3 h-12">
            <button
              onClick={() => dispatch({ type: 'HIDE_CRON' })}
              className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
            >
              <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                <path d="M19 12H5M12 19l-7-7 7-7" />
              </svg>
            </button>
            <h1 className="text-primary text-lg font-semibold">Cron Jobs</h1>
          </div>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-[740px] mx-auto px-5 py-6">
          {loading && (
            <div className="text-center text-dim text-sm py-12 font-mono">Loading...</div>
          )}
          {error && (
            <div className="text-center text-err text-sm py-12 font-mono">{error}</div>
          )}
          {!loading && !error && jobs.length === 0 && (
            <div className="text-center text-dim text-sm py-12 font-mono">
              No cron jobs scheduled
            </div>
          )}
          {!loading && !error && jobs.length > 0 && (
            <div className="space-y-2">
              {jobs.map(job => (
                <div
                  key={job.id}
                  className="bg-surface border border-border-main rounded-lg px-4 py-3"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-medium text-primary truncate">{job.name}</span>
                        <span className={`text-[11px] px-1.5 py-0.5 rounded font-mono ${
                          job.enabled
                            ? 'bg-green-500/10 text-green-400'
                            : 'bg-neutral-500/10 text-dim'
                        }`}>
                          {job.enabled ? 'active' : 'paused'}
                        </span>
                      </div>
                      <div className="text-[12px] text-dim mt-1 font-mono truncate">
                        {job.prompt}
                      </div>
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-x-4 gap-y-1 mt-2 text-[11px] text-muted font-mono">
                    <span title="Schedule">{formatSchedule(job.schedule)}</span>
                    <span title="Last run">last: {formatEpoch(job.last_run_at)}</span>
                    <span title="Next run">next: {formatNextRun(job.next_run_at)}</span>
                    {isAdmin && job.user_id && (
                      <span title="User" className="text-accent/70">user: {job.user_id}</span>
                    )}
                    {isAdmin && !job.user_id && (
                      <span title="Agent-level" className="text-accent/70">agent</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
