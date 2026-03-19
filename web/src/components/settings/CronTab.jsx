import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Field, Input, SaveBar } from './fields'

function formatTimeout(seconds) {
  if (seconds === null || seconds === undefined || seconds === '') return ''
  const n = Number(seconds)
  if (isNaN(n) || n <= 0) return ''
  if (n >= 3600) {
    const h = (n / 3600).toFixed(1).replace(/\.0$/, '')
    return `= ${h} hour${h === '1' ? '' : 's'}`
  }
  const m = (n / 60).toFixed(1).replace(/\.0$/, '')
  return `= ${m} minute${m === '1' ? '' : 's'}`
}

export default function CronTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/cron', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load settings' }))
  }, [])

  if (!config) return <div className="text-dim text-sm">Loading...</div>

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true)
    setStatus(null)
    try {
      const resp = await fetch('/api/settings/cron', {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify(config),
      })
      if (resp.ok) setStatus({ type: 'ok', text: 'Saved' })
      else setStatus({ type: 'error', text: 'Save failed: ' + resp.statusText })
    } catch (e) {
      setStatus({ type: 'error', text: 'Save failed: ' + e.message })
    }
    setSaving(false)
  }

  const timeoutHint = formatTimeout(config.timeout_seconds)

  return (
    <div>
      <Field label="Max retries">
        <Input id="max_retries" type="number" value={config.max_retries ?? ''} onChange={v => set('max_retries', v === '' ? null : Number(v))} placeholder="3" />
      </Field>
      <Field label="Timeout (seconds)">
        <div>
          <Input id="timeout_seconds" type="number" value={config.timeout_seconds ?? ''} onChange={v => set('timeout_seconds', v === '' ? null : Number(v))} placeholder="300" />
          {timeoutHint && <div className="text-dim text-xs mt-1">{timeoutHint}</div>}
        </div>
      </Field>
      <Field label="Max concurrent runs">
        <Input id="max_concurrent" type="number" value={config.max_concurrent ?? ''} onChange={v => set('max_concurrent', v === '' ? null : Number(v))} placeholder="5" />
      </Field>

      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
