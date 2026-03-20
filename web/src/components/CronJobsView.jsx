import React, { useState, useEffect, useCallback } from 'react'
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

const INPUT_CLASS = 'w-full bg-elevated border border-border-main rounded-lg px-3 py-2 text-[13px] text-primary font-mono placeholder:text-dim focus:outline-none focus:border-accent/50 transition-colors'
const LABEL_CLASS = 'block text-[11px] text-muted font-mono uppercase tracking-wider mb-1.5'

function scheduleToForm(schedule) {
  if (!schedule) return { type: 'interval', cronExpr: '0 9 * * *', intervalValue: 60, intervalUnit: 'm', oneShotAt: '' }
  switch (schedule.kind) {
    case 'cron':
      return { type: 'cron', cronExpr: schedule.expr, intervalValue: 60, intervalUnit: 'm', oneShotAt: '' }
    case 'interval': {
      const ms = schedule.every_ms
      if (ms >= 86400000) return { type: 'interval', cronExpr: '', intervalValue: Math.round(ms / 86400000), intervalUnit: 'd', oneShotAt: '' }
      if (ms >= 3600000) return { type: 'interval', cronExpr: '', intervalValue: Math.round(ms / 3600000), intervalUnit: 'h', oneShotAt: '' }
      if (ms >= 60000) return { type: 'interval', cronExpr: '', intervalValue: Math.round(ms / 60000), intervalUnit: 'm', oneShotAt: '' }
      return { type: 'interval', cronExpr: '', intervalValue: Math.round(ms / 1000), intervalUnit: 's', oneShotAt: '' }
    }
    case 'one_shot':
      return { type: 'one_shot', cronExpr: '', intervalValue: 60, intervalUnit: 'm', oneShotAt: schedule.at?.slice(0, 16) || '' }
    default:
      return { type: 'interval', cronExpr: '0 9 * * *', intervalValue: 60, intervalUnit: 'm', oneShotAt: '' }
  }
}

function buildSchedule(scheduleType, cronExpr, intervalValue, intervalUnit, oneShotAt) {
  switch (scheduleType) {
    case 'cron':
      return { kind: 'cron', expr: cronExpr }
    case 'interval': {
      const multipliers = { s: 1000, m: 60000, h: 3600000, d: 86400000 }
      return { kind: 'interval', every_ms: intervalValue * (multipliers[intervalUnit] || 60000) }
    }
    case 'one_shot':
      return { kind: 'one_shot', at: new Date(oneShotAt).toISOString() }
    default:
      return null
  }
}

function ScheduleFields({ scheduleType, setScheduleType, cronExpr, setCronExpr, intervalValue, setIntervalValue, intervalUnit, setIntervalUnit, oneShotAt, setOneShotAt }) {
  return (
    <div>
      <label className={LABEL_CLASS}>Schedule</label>
      <div className="flex gap-2 mb-2">
        {['interval', 'cron', 'one_shot'].map(t => (
          <button
            key={t}
            type="button"
            onClick={() => setScheduleType(t)}
            className={`text-[11px] font-mono px-2.5 py-1 rounded-md border transition-colors ${
              scheduleType === t
                ? 'border-accent/50 bg-accent/10 text-accent'
                : 'border-border-main text-muted hover:text-secondary'
            }`}
          >
            {t === 'one_shot' ? 'one-shot' : t}
          </button>
        ))}
      </div>
      {scheduleType === 'cron' && (
        <input
          type="text"
          value={cronExpr}
          onChange={e => setCronExpr(e.target.value)}
          placeholder="0 9 * * *"
          className={INPUT_CLASS}
        />
      )}
      {scheduleType === 'interval' && (
        <div className="flex gap-2">
          <input
            type="number"
            min="1"
            value={intervalValue}
            onChange={e => setIntervalValue(Number(e.target.value))}
            className={INPUT_CLASS + ' w-24'}
          />
          <select
            value={intervalUnit}
            onChange={e => setIntervalUnit(e.target.value)}
            className={INPUT_CLASS + ' w-24'}
          >
            <option value="s">sec</option>
            <option value="m">min</option>
            <option value="h">hour</option>
            <option value="d">day</option>
          </select>
        </div>
      )}
      {scheduleType === 'one_shot' && (
        <input
          type="datetime-local"
          value={oneShotAt}
          onChange={e => setOneShotAt(e.target.value)}
          className={INPUT_CLASS}
        />
      )}
    </div>
  )
}

function CreateJobForm({ onCreated, onCancel }) {
  const [name, setName] = useState('')
  const [prompt, setPrompt] = useState('')
  const [scheduleType, setScheduleType] = useState('interval')
  const [cronExpr, setCronExpr] = useState('0 9 * * *')
  const [intervalValue, setIntervalValue] = useState(60)
  const [intervalUnit, setIntervalUnit] = useState('m')
  const [oneShotAt, setOneShotAt] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)

  async function handleSubmit(e) {
    e.preventDefault()
    if (!name.trim() || !prompt.trim()) return
    setSaving(true)
    setError(null)
    try {
      const res = await fetch('/api/cron/jobs', {
        method: 'POST',
        headers: { ...authHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: name.trim(),
          prompt: prompt.trim(),
          schedule: buildSchedule(scheduleType, cronExpr, intervalValue, intervalUnit, oneShotAt),
          delete_after_run: scheduleType === 'one_shot',
        }),
      })
      if (!res.ok) {
        const data = await res.json().catch(() => ({}))
        throw new Error(data.error || res.statusText)
      }
      const job = await res.json()
      onCreated(job)
    } catch (err) {
      setError(err.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="bg-surface border border-border-main rounded-lg p-4 mb-4">
      <div className="space-y-3">
        <div>
          <label className={LABEL_CLASS}>Name</label>
          <input
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            placeholder="daily-report"
            className={INPUT_CLASS}
            autoFocus
          />
        </div>
        <div>
          <label className={LABEL_CLASS}>Prompt</label>
          <textarea
            value={prompt}
            onChange={e => setPrompt(e.target.value)}
            placeholder="What the agent should do when this job fires..."
            rows={3}
            className={INPUT_CLASS + ' resize-none'}
          />
        </div>
        <ScheduleFields {...{ scheduleType, setScheduleType, cronExpr, setCronExpr, intervalValue, setIntervalValue, intervalUnit, setIntervalUnit, oneShotAt, setOneShotAt }} />
        {error && <div className="text-[12px] text-err font-mono">{error}</div>}
        <div className="flex gap-2 justify-end pt-1">
          <button
            type="button"
            onClick={onCancel}
            className="text-[12px] font-mono text-muted hover:text-secondary px-3 py-1.5 rounded-md border border-border-main hover:border-border-main/80 transition-colors"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || !name.trim() || !prompt.trim()}
            className="text-[12px] font-mono text-bg bg-accent hover:bg-accent/90 disabled:opacity-40 px-3 py-1.5 rounded-md transition-colors"
          >
            {saving ? 'Creating...' : 'Create'}
          </button>
        </div>
      </div>
    </form>
  )
}

function EditJobForm({ job, onUpdated, onCancel }) {
  const [prompt, setPrompt] = useState(job.prompt)
  const sched = scheduleToForm(job.schedule)
  const [scheduleType, setScheduleType] = useState(sched.type)
  const [cronExpr, setCronExpr] = useState(sched.cronExpr)
  const [intervalValue, setIntervalValue] = useState(sched.intervalValue)
  const [intervalUnit, setIntervalUnit] = useState(sched.intervalUnit)
  const [oneShotAt, setOneShotAt] = useState(sched.oneShotAt)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)

  async function handleSubmit(e) {
    e.preventDefault()
    if (!prompt.trim()) return
    setSaving(true)
    setError(null)
    try {
      const res = await fetch('/api/cron/jobs/' + job.id, {
        method: 'PUT',
        headers: { ...authHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({
          prompt: prompt.trim(),
          schedule: buildSchedule(scheduleType, cronExpr, intervalValue, intervalUnit, oneShotAt),
        }),
      })
      if (!res.ok) {
        const data = await res.json().catch(() => ({}))
        throw new Error(data.error || res.statusText)
      }
      const updated = await res.json()
      onUpdated(updated)
    } catch (err) {
      setError(err.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="mt-3 ml-5 space-y-3">
      <div>
        <label className={LABEL_CLASS}>Prompt</label>
        <textarea
          value={prompt}
          onChange={e => setPrompt(e.target.value)}
          rows={3}
          className={INPUT_CLASS + ' resize-none'}
          autoFocus
        />
      </div>
      <ScheduleFields {...{ scheduleType, setScheduleType, cronExpr, setCronExpr, intervalValue, setIntervalValue, intervalUnit, setIntervalUnit, oneShotAt, setOneShotAt }} />
      {error && <div className="text-[12px] text-err font-mono">{error}</div>}
      <div className="flex gap-2 justify-end">
        <button
          type="button"
          onClick={onCancel}
          className="text-[12px] font-mono text-muted hover:text-secondary px-3 py-1.5 rounded-md border border-border-main transition-colors"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={saving || !prompt.trim()}
          className="text-[12px] font-mono text-bg bg-accent hover:bg-accent/90 disabled:opacity-40 px-3 py-1.5 rounded-md transition-colors"
        >
          {saving ? 'Saving...' : 'Save'}
        </button>
      </div>
    </form>
  )
}

function JobRow({ job, isAdmin, onUpdate, onDelete }) {
  const [expanded, setExpanded] = useState(false)
  const [editing, setEditing] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [toggling, setToggling] = useState(false)

  async function handleDelete() {
    if (!confirm('Delete "' + job.name + '"?')) return
    setDeleting(true)
    try {
      const res = await fetch('/api/cron/jobs/' + job.id, {
        method: 'DELETE',
        headers: authHeaders(),
      })
      if (!res.ok && res.status !== 204) throw new Error(res.statusText)
      onDelete(job.id)
    } catch {
      setDeleting(false)
    }
  }

  async function handleToggle() {
    setToggling(true)
    try {
      const res = await fetch('/api/cron/jobs/' + job.id, {
        method: 'PUT',
        headers: { ...authHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled: !job.enabled }),
      })
      if (!res.ok) throw new Error(res.statusText)
      const updated = await res.json()
      onUpdate(updated)
    } catch {
      // silent
    } finally {
      setToggling(false)
    }
  }

  function handleEdited(updated) {
    onUpdate(updated)
    setEditing(false)
  }

  return (
    <div className="bg-surface border border-border-main rounded-lg px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div
          className="flex-1 min-w-0 cursor-pointer"
          onClick={() => { if (!editing) setExpanded(v => !v) }}
        >
          <div className="flex items-center gap-2">
            <svg
              className={`w-3 h-3 text-dim shrink-0 transition-transform duration-150 ${expanded ? 'rotate-90' : ''}`}
              viewBox="0 0 16 16"
              fill="currentColor"
            >
              <path d="M6 3l5 5-5 5V3z" />
            </svg>
            <span className="text-[13px] font-medium text-primary truncate">{job.name}</span>
            <button
              onClick={e => { e.stopPropagation(); handleToggle() }}
              disabled={toggling}
              className={`text-[11px] px-1.5 py-0.5 rounded font-mono transition-colors ${
                job.enabled
                  ? 'bg-green-500/10 text-green-400 hover:bg-green-500/20'
                  : 'bg-neutral-500/10 text-dim hover:bg-neutral-500/20'
              }`}
              title={job.enabled ? 'Click to pause' : 'Click to activate'}
            >
              {job.enabled ? 'active' : 'paused'}
            </button>
          </div>
          {!expanded && !editing && (
            <div className="text-[12px] text-dim mt-1 font-mono truncate ml-5">
              {job.prompt}
            </div>
          )}
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => { setEditing(v => !v); setExpanded(true) }}
            className="text-dim hover:text-accent transition-colors p-1"
            title="Edit job"
          >
            <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M11.5 2.5l2 2-8 8H3.5v-2l8-8z" />
            </svg>
          </button>
          <button
            onClick={handleDelete}
            disabled={deleting}
            className="text-dim hover:text-err transition-colors p-1"
            title="Delete job"
          >
            <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M4 4l8 8M12 4l-8 8" />
            </svg>
          </button>
        </div>
      </div>
      {expanded && !editing && (
        <div className="mt-2 ml-5 text-[12px] text-secondary font-mono whitespace-pre-wrap break-words border-l-2 border-border-subtle pl-3">
          {job.prompt}
        </div>
      )}
      {editing && (
        <EditJobForm
          job={job}
          onUpdated={handleEdited}
          onCancel={() => setEditing(false)}
        />
      )}
      <div className="flex flex-wrap gap-x-4 gap-y-1 mt-2 text-[11px] text-muted font-mono ml-5">
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
  )
}

export default function CronJobsView() {
  const { isAdmin } = useUser()
  const [jobs, setJobs] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)
  const [showForm, setShowForm] = useState(false)

  const fetchJobs = useCallback(() => {
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

  useEffect(() => { fetchJobs() }, [fetchJobs])

  function handleCreated(job) {
    setJobs(prev => [...prev, job])
    setShowForm(false)
  }

  function handleUpdate(updated) {
    setJobs(prev => prev.map(j => j.id === updated.id ? updated : j))
  }

  function handleDelete(id) {
    setJobs(prev => prev.filter(j => j.id !== id))
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-[740px] mx-auto px-5 py-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-primary text-lg font-semibold">Cron Jobs</h1>
          {!showForm && (
            <button
              onClick={() => setShowForm(true)}
              className="text-[12px] font-mono text-muted hover:text-primary border border-border-main hover:border-accent/40 px-3 py-1.5 rounded-md transition-colors flex items-center gap-1.5"
            >
              <svg className="w-3 h-3" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M8 3v10M3 8h10" />
              </svg>
              Add job
            </button>
          )}
        </div>
        {showForm && (
          <CreateJobForm
            onCreated={handleCreated}
            onCancel={() => setShowForm(false)}
          />
        )}
        {loading && (
          <div className="text-center text-dim text-sm py-12 font-mono">Loading...</div>
        )}
        {error && (
          <div className="text-center text-err text-sm py-12 font-mono">{error}</div>
        )}
        {!loading && !error && jobs.length === 0 && !showForm && (
          <div className="text-center text-dim text-sm py-12 font-mono">
            No cron jobs scheduled
          </div>
        )}
        {!loading && !error && jobs.length > 0 && (
          <div className="space-y-2">
            {jobs.map(job => (
              <JobRow
                key={job.id}
                job={job}
                isAdmin={isAdmin}
                onUpdate={handleUpdate}
                onDelete={handleDelete}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
