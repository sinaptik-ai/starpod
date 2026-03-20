import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

function fmtTimeout(s) {
  const n = Number(s)
  if (!n || n <= 0) return null
  if (n >= 3600) return (n / 3600).toFixed(1).replace(/\.0$/, '') + 'h'
  return (n / 60).toFixed(0) + 'min'
}

export default function CronTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/cron', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))
  const hint = fmtTimeout(config.default_timeout_secs)

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/cron', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Defaults">
        <Row label="Max retries">
          <Input type="number" value={config.default_max_retries ?? ''} onChange={v => set('default_max_retries', v === '' ? null : Number(v))} placeholder="3" />
        </Row>
        <Row label="Timeout" sub={hint ? `= ${hint}` : 'seconds'}>
          <Input type="number" value={config.default_timeout_secs ?? ''} onChange={v => set('default_timeout_secs', v === '' ? null : Number(v))} placeholder="7200" />
        </Row>
        <Row label="Concurrent runs">
          <Input type="number" value={config.max_concurrent_runs ?? ''} onChange={v => set('max_concurrent_runs', v === '' ? null : Number(v))} placeholder="1" />
        </Row>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
