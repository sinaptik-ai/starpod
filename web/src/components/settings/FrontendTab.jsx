import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Field, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

export default function FrontendTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/frontend', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))
  const prompts = config.prompts || config.suggested_prompts || []

  const updatePrompt = (i, val) => { const next = [...prompts]; next[i] = val; set('prompts', next); set('suggested_prompts', next) }
  const removePrompt = (i) => { const next = [...prompts]; next.splice(i, 1); set('prompts', next); set('suggested_prompts', next) }
  const addPrompt = () => { const next = [...prompts, '']; set('prompts', next); set('suggested_prompts', next) }

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/frontend', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Welcome screen">
        <Row label="Greeting">
          <Input value={config.greeting || ''} onChange={v => set('greeting', v)} placeholder="ready_" />
        </Row>
        <Field label="Suggested prompts" desc="Shown as clickable chips on the welcome screen.">
          <div className="flex flex-col gap-1.5">
            {prompts.map((p, i) => (
              <div key={i} className="prompt-item">
                <input
                  type="text"
                  className="s-input font-mono text-xs"
                  value={p}
                  onChange={e => updatePrompt(i, e.target.value)}
                  placeholder="Enter a prompt..."
                />
                <button onClick={() => removePrompt(i)} title="Remove">&times;</button>
              </div>
            ))}
            <button onClick={addPrompt} className="text-accent text-xs hover:underline cursor-pointer self-start mt-0.5">
              + add prompt
            </button>
          </div>
        </Field>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
