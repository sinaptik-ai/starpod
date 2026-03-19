import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Section, Field, Input, Toggle, SaveBar } from './fields'

export default function MemoryTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/memory', { headers: apiHeaders() })
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
      const resp = await fetch('/api/settings/memory', {
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

  const mmrValue = config.mmr_lambda ?? 0.7

  return (
    <div>
      <Section title="Search" />
      <Field label="Half-life days" helpTip="How quickly daily logs fade from memory search. 7 = recent logs dominate. 90 = old logs stay relevant longer.">
        <Input id="half_life_days" type="number" value={config.half_life_days ?? ''} onChange={v => set('half_life_days', v === '' ? null : Number(v))} placeholder="30" />
      </Field>
      <Field label="MMR lambda" helpTip="Controls diversity in memory search results. 0 = maximum variety. 1 = pure relevance. 0.7 is a good default.">
        <div className="flex items-center gap-3">
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={mmrValue}
            onChange={e => set('mmr_lambda', Number(e.target.value))}
            className="settings-range flex-1"
          />
          <span className="text-xs text-secondary font-mono w-8 text-right">{mmrValue.toFixed(2)}</span>
        </div>
      </Field>
      <Field label="Vector search" helpTip="Enables semantic search using embeddings. More accurate but slower.">
        <Toggle id="vector_search" checked={config.vector_search ?? false} onChange={v => set('vector_search', v)} />
      </Field>

      <Section title="Indexing" />
      <div className="grid grid-cols-2 gap-4">
        <Field label="Chunk size" helpTip="Controls how files are split for indexing. Larger chunks give more context per result.">
          <Input id="chunk_size" type="number" value={config.chunk_size ?? ''} onChange={v => set('chunk_size', v === '' ? null : Number(v))} placeholder="400" />
        </Field>
        <Field label="Chunk overlap" helpTip="Overlap prevents important content from being split across chunk boundaries.">
          <Input id="chunk_overlap" type="number" value={config.chunk_overlap ?? ''} onChange={v => set('chunk_overlap', v === '' ? null : Number(v))} placeholder="80" />
        </Field>
      </div>

      <Section title="Storage" />
      <Field label="Export sessions" helpTip="When a session closes, save the full transcript so the agent can search past conversations.">
        <Toggle id="export_sessions" checked={config.export_sessions ?? false} onChange={v => set('export_sessions', v)} />
      </Field>

      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
