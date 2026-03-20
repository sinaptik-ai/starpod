import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Toggle, SaveBar } from './fields'

export default function MemoryTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/memory', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <div className="text-dim text-sm py-8 text-center">Loading...</div>

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))
  const mmr = config.mmr_lambda ?? 0.7

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/memory', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Search">
        <Row label="Half-life" sub="days" helpTip="How fast daily logs fade. 7 = recent dominate. 90 = old logs persist.">
          <Input type="number" value={config.half_life_days ?? ''} onChange={v => set('half_life_days', v === '' ? null : Number(v))} placeholder="30" />
        </Row>
        <Row label="MMR lambda" helpTip="0 = max diversity, 1 = pure relevance. 0.7 is a good default.">
          <div className="flex items-center gap-3 w-full">
            <input type="range" min="0" max="1" step="0.05" value={mmr} onChange={e => set('mmr_lambda', Number(e.target.value))} className="s-range flex-1" />
            <span className="font-mono text-xs text-secondary w-8 text-right shrink-0">{mmr.toFixed(2)}</span>
          </div>
        </Row>
        <Toggle label="Vector search" checked={config.vector_search ?? false} onChange={v => set('vector_search', v)} helpTip="Semantic search via embeddings. More accurate but slower." />
      </Card>

      <Card title="Indexing">
        <Row label="Chunk size" sub="chars" helpTip="How files are split. Larger = more context per result.">
          <Input type="number" value={config.chunk_size ?? ''} onChange={v => set('chunk_size', v === '' ? null : Number(v))} placeholder="400" />
        </Row>
        <Row label="Chunk overlap" sub="chars" helpTip="Prevents content from being split at boundaries.">
          <Input type="number" value={config.chunk_overlap ?? ''} onChange={v => set('chunk_overlap', v === '' ? null : Number(v))} placeholder="80" />
        </Row>
      </Card>

      <Card title="Storage">
        <Toggle label="Export sessions" checked={config.export_sessions ?? false} onChange={v => set('export_sessions', v)} helpTip="Save closed session transcripts for long-term search." />
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
