import { useState, useEffect } from 'react'
import { apiHeaders, fetchModels } from '../../lib/api'

const ENV_KEY_MAP = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  gemini: 'GEMINI_API_KEY',
  groq: 'GROQ_API_KEY',
  deepseek: 'DEEPSEEK_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
}

export default function StepModel({ data, updateData, onNext, onBack }) {
  const [catalog, setCatalog] = useState({})
  const [provider, setProvider] = useState(data.provider || 'anthropic')
  const [model, setModel] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [braveKey, setBraveKey] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)
  const [showBrave, setShowBrave] = useState(false)

  useEffect(() => {
    fetchModels().then(m => {
      setCatalog(m || {})
      const models = m?.[provider] || []
      if (!model && models.length > 0) setModel(models[0])
    })
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const providers = Object.keys(catalog)
  const providerModels = catalog[provider] || []
  const needsKey = provider !== 'ollama'
  const envKeyName = ENV_KEY_MAP[provider] || `${provider.toUpperCase()}_API_KEY`

  const handleProviderChange = (p) => {
    setProvider(p)
    const models = catalog[p] || []
    setModel(models[0] || '')
    setApiKey('')
    updateData({ provider: p })
  }

  const handleNext = async () => {
    if (!model) return
    if (needsKey && !apiKey.trim()) return
    setSaving(true)
    setError(null)
    try {
      const spec = `${provider}/${model}`

      const resp = await fetch('/api/settings/general', { headers: apiHeaders() })
      if (!resp.ok) throw new Error('Failed to load config')
      const config = await resp.json()
      config.models = [spec]

      const saveResp = await fetch('/api/settings/general', {
        method: 'PUT', headers: apiHeaders(),
        body: JSON.stringify(config),
      })
      if (!saveResp.ok) throw new Error('Failed to save model')

      if (needsKey && apiKey.trim()) {
        const vaultResp = await fetch(`/api/settings/vault/${encodeURIComponent(envKeyName)}`, {
          method: 'PUT', headers: apiHeaders(),
          body: JSON.stringify({ value: apiKey.trim() }),
        })
        if (!vaultResp.ok) throw new Error('Failed to save API key')
      }

      if (braveKey.trim()) {
        await fetch('/api/settings/vault/BRAVE_API_KEY', {
          method: 'PUT', headers: apiHeaders(),
          body: JSON.stringify({ value: braveKey.trim() }),
        })
      }

      updateData({ model: spec })
      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  return (
    <div>
      <h2 className="ob-heading">Choose a model</h2>
      <p className="ob-desc">Select the LLM provider and model, then add your API key.</p>

      <div className="ob-field-group">
        <label className="ob-label">Provider</label>
        <select
          className="ob-select"
          value={provider}
          onChange={e => handleProviderChange(e.target.value)}
        >
          {providers.map(p => <option key={p} value={p}>{p}</option>)}
        </select>
      </div>

      <div className="ob-field-group">
        <label className="ob-label">Model</label>
        <select
          className="ob-select ob-select--mono"
          value={model}
          onChange={e => setModel(e.target.value)}
        >
          {providerModels.map(m => <option key={m} value={m}>{m}</option>)}
        </select>
      </div>

      {needsKey && (
        <div className="ob-field-group">
          <label className="ob-label">{envKeyName}</label>
          <input
            type="password"
            className="ob-input ob-input--mono"
            value={apiKey}
            onChange={e => setApiKey(e.target.value)}
            placeholder="sk-..."
            onKeyDown={e => { if (e.key === 'Enter') handleNext() }}
          />
        </div>
      )}

      {!needsKey && (
        <p className="ob-hint">No API key required for local models.</p>
      )}

      {!showBrave ? (
        <button onClick={() => setShowBrave(true)} className="ob-link">
          + Add web search key (optional)
        </button>
      ) : (
        <div className="ob-field-group">
          <label className="ob-label">
            BRAVE_API_KEY <span className="ob-label-hint">(optional)</span>
          </label>
          <input
            type="password"
            className="ob-input ob-input--mono"
            value={braveKey}
            onChange={e => setBraveKey(e.target.value)}
            placeholder="BSA..."
          />
        </div>
      )}

      {error && <p className="ob-error">{error}</p>}

      <div className="ob-actions">
        <button onClick={onBack} className="ob-btn-back">Back</button>
        <button
          onClick={handleNext}
          disabled={!model || (needsKey && !apiKey.trim()) || saving}
          className="ob-btn-primary"
        >
          {saving ? 'Saving...' : 'Continue'}
        </button>
      </div>
    </div>
  )
}
