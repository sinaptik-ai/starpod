import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Toggle, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

export default function CompactionTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/compaction', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/compaction', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Context management" desc="Controls how the agent manages conversation context to stay within token limits">
        <Row label="Context budget" sub="tokens" helpTip="Token threshold that triggers conversation compaction. Set to ~80% of the model's context window.">
          <Input type="number" value={config.context_budget ?? ''} onChange={v => set('context_budget', v === '' ? null : Number(v))} placeholder="160000" />
        </Row>
        <Row label="Summary max tokens" sub="tokens" helpTip="Maximum tokens for the compaction summary response.">
          <Input type="number" value={config.summary_max_tokens ?? ''} onChange={v => set('summary_max_tokens', v === '' ? null : Number(v))} placeholder="4096" />
        </Row>
        <Row label="Min keep messages" helpTip="Minimum number of recent messages to preserve during compaction. These are never summarized.">
          <Input type="number" value={config.min_keep_messages ?? ''} onChange={v => set('min_keep_messages', v === '' ? null : Number(v))} placeholder="4" />
        </Row>
        <Row label="Memory flush" helpTip="Run a silent agentic turn before compaction to persist important memories from the conversation being compacted.">
          <Toggle checked={config.memory_flush} onChange={v => set('memory_flush', v)} />
        </Row>
      </Card>

      <Card title="Tool result sanitization" desc="Limits applied to all tool results before they enter the conversation">
        <Row label="Max result size" sub="bytes" helpTip="Hard byte limit for any single tool result. Also strips base64 data URIs and hex blobs exceeding 200 characters.">
          <Input type="number" value={config.max_tool_result_bytes ?? ''} onChange={v => set('max_tool_result_bytes', v === '' ? null : Number(v))} placeholder="50000" />
        </Row>
      </Card>

      <Card title="Tool result pruning" desc="Lightweight pruning of older tool results when context usage is high">
        <Row label="Prune threshold" sub="% of budget" helpTip="Percentage of context budget at which pruning triggers. Pruning runs before full compaction as a lighter-weight alternative.">
          <Input type="number" value={config.prune_threshold_pct ?? ''} onChange={v => set('prune_threshold_pct', v === '' ? null : Number(v))} placeholder="70" />
        </Row>
        <Row label="Prune size threshold" sub="characters" helpTip="Tool results longer than this are candidates for pruning. Pruned results keep the first 500 and last 200 characters.">
          <Input type="number" value={config.prune_tool_result_max_chars ?? ''} onChange={v => set('prune_tool_result_max_chars', v === '' ? null : Number(v))} placeholder="2000" />
        </Row>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
