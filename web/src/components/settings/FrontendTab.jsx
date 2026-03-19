import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Field, Input, SaveBar } from './fields'

export default function FrontendTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/frontend', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load settings' }))
  }, [])

  if (!config) return <div className="text-dim text-sm">Loading...</div>

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const updatePrompt = (index, val) => {
    const next = [...(config.suggested_prompts || [])]
    next[index] = val
    set('suggested_prompts', next)
  }

  const removePrompt = (index) => {
    const next = [...(config.suggested_prompts || [])]
    next.splice(index, 1)
    set('suggested_prompts', next)
  }

  const addPrompt = () => {
    set('suggested_prompts', [...(config.suggested_prompts || []), ''])
  }

  const save = async () => {
    setSaving(true)
    setStatus(null)
    try {
      const resp = await fetch('/api/settings/frontend', {
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

  return (
    <div>
      <Field label="Greeting">
        <Input id="greeting" value={config.greeting || ''} onChange={v => set('greeting', v)} placeholder="How can I help you today?" />
      </Field>

      <Field label="Suggested prompts">
        <div>
          {(config.suggested_prompts || []).map((prompt, i) => (
            <div key={i} className="prompt-item">
              <input
                type="text"
                className="settings-input"
                value={prompt}
                onChange={e => updatePrompt(i, e.target.value)}
                placeholder="Enter a suggested prompt..."
              />
              <button onClick={() => removePrompt(i)} title="Remove">&times;</button>
            </div>
          ))}
          <button
            onClick={addPrompt}
            className="text-accent text-xs font-medium hover:text-accent-soft transition-colors cursor-pointer mt-1"
          >
            + Add prompt
          </button>
        </div>
      </Field>

      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
