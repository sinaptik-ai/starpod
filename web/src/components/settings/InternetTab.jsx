import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Toggle, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

export default function InternetTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/internet', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/internet', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Internet access" desc="Web search and page fetching for the agent">
        <Row label="Enabled" helpTip="When enabled, the agent can use WebSearch and WebFetch tools to access the internet.">
          <Toggle checked={config.enabled} onChange={v => set('enabled', v)} />
        </Row>
        <Row label="Brave API key" helpTip="API key for Brave Search. Required for WebSearch to work.">
          <Input value={config.brave_api_key || ''} onChange={v => set('brave_api_key', v || null)} placeholder="BSA..." mono />
        </Row>
        <Row label="Timeout" sub="seconds" helpTip="Request timeout for web fetch operations.">
          <Input type="number" value={config.timeout_secs ?? ''} onChange={v => set('timeout_secs', v === '' ? null : Number(v))} placeholder="15" />
        </Row>
        <Row label="Max fetch size" sub="bytes" helpTip="Maximum raw HTTP response size in bytes, applied before HTML processing.">
          <Input type="number" value={config.max_fetch_bytes ?? ''} onChange={v => set('max_fetch_bytes', v === '' ? null : Number(v))} placeholder="2097152" />
        </Row>
        <Row label="Max text chars" sub="characters" helpTip="Maximum extracted text length after readability extraction and markdown conversion.">
          <Input type="number" value={config.max_text_chars ?? ''} onChange={v => set('max_text_chars', v === '' ? null : Number(v))} placeholder="50000" />
        </Row>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
