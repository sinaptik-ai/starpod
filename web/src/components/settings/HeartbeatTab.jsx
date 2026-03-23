import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Toggle, Textarea, SaveBar } from './fields'

export default function HeartbeatTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/heartbeat', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <div className="text-dim text-sm py-8 text-center">Loading...</div>

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/heartbeat', {
        method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config),
      })
      if (resp.ok) {
        setStatus({ type: 'ok', text: 'Saved' })
      } else {
        const data = await resp.json().catch(() => ({}))
        setStatus({ type: 'error', text: data.error || 'Failed' })
      }
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Heartbeat" desc="Instructions the agent follows on a recurring schedule.">
        <Toggle label="Enabled" checked={config.enabled} onChange={v => set('enabled', v)} />
        {config.enabled && (
          <>
            <Row label="Interval" sub="minutes between heartbeats">
              <Input type="number" value={config.interval_minutes ?? 30} onChange={v => set('interval_minutes', v === '' ? 30 : Number(v))} placeholder="30" />
            </Row>
          </>
        )}
      </Card>

      {config.enabled && (
        <div className="border border-border-subtle rounded-none overflow-hidden mt-4">
          <div className="flex items-center justify-between px-3 py-1.5 bg-surface border-b border-border-subtle">
            <span className="font-mono text-[11px] text-dim">HEARTBEAT.md</span>
          </div>
          <Textarea value={config.content} onChange={v => set('content', v)} rows={20} />
        </div>
      )}

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
