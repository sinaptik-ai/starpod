import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Toggle, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

export default function BrowserTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/browser', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/browser', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Browser automation" desc="CDP-based browser control for web tasks (beta — works best with server-rendered pages)">
        <Row label="Enabled" helpTip="When enabled, the agent can use BrowserOpen, BrowserClick, BrowserType, BrowserExtract, and BrowserEval tools. Beta: uses Lightpanda, which works well for server-rendered pages but may not render JavaScript-heavy SPAs (Angular, React, Vue).">
          <Toggle checked={config.enabled} onChange={v => set('enabled', v)} />
        </Row>
        <Row label="CDP endpoint" sub="leave empty to auto-spawn" helpTip="WebSocket URL of an existing CDP browser (e.g. ws://127.0.0.1:9222). When empty, the agent auto-spawns a Lightpanda process.">
          <Input value={config.cdp_url || ''} onChange={v => set('cdp_url', v || null)} placeholder="ws://127.0.0.1:9222" mono />
        </Row>
        <Row label="Startup timeout" sub="seconds" helpTip="How long to wait for the auto-spawned browser to accept CDP connections.">
          <Input type="number" value={config.startup_timeout_secs ?? ''} onChange={v => set('startup_timeout_secs', v === '' ? null : Number(v))} placeholder="10" />
        </Row>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
