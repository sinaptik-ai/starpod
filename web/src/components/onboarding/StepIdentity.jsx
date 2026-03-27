import { useState, useRef, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'

export default function StepIdentity({ data, updateData, onNext }) {
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)
  const inputRef = useRef(null)

  useEffect(() => {
    const t = setTimeout(() => inputRef.current?.focus(), 100)
    return () => clearTimeout(t)
  }, [])

  const handleNext = async () => {
    if (!data.agentName.trim()) return
    setSaving(true)
    setError(null)
    try {
      const resp = await fetch('/api/settings/general', { headers: apiHeaders() })
      if (!resp.ok) throw new Error('Failed to load config')
      const config = await resp.json()
      config.agent_name = data.agentName.trim()

      const saveResp = await fetch('/api/settings/general', {
        method: 'PUT', headers: apiHeaders(),
        body: JSON.stringify(config),
      })
      if (!saveResp.ok) throw new Error('Failed to save')
      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  return (
    <div>
      <h2 className="ob-heading">Name your agent</h2>
      <p className="ob-desc">Give your AI assistant a name. This is how it will introduce itself.</p>

      <div className="ob-field-group">
        <label className="ob-label">Agent name</label>
        <input
          ref={inputRef}
          type="text"
          className="ob-input-lg"
          value={data.agentName}
          onChange={e => updateData({ agentName: e.target.value })}
          placeholder="Nova"
          onKeyDown={e => { if (e.key === 'Enter' && data.agentName.trim()) handleNext() }}
        />
      </div>

      {error && <p className="ob-error">{error}</p>}

      <div className="ob-actions ob-actions--end">
        <button
          onClick={handleNext}
          disabled={!data.agentName.trim() || saving}
          className="ob-btn-primary"
        >
          {saving ? 'Saving...' : 'Continue'}
        </button>
      </div>
    </div>
  )
}
