import { useState, useEffect, useRef, useCallback } from 'react'
import { apiHeaders, fetchModels } from '../../lib/api'

const ENV_KEY_MAP = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  gemini: 'GEMINI_API_KEY',
  groq: 'GROQ_API_KEY',
  deepseek: 'DEEPSEEK_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
}

export default function StepAgent({ data, updateData, onNext }) {
  const [catalog, setCatalog] = useState({})
  const [provider, setProvider] = useState(data.provider || 'anthropic')
  const [model, setModel] = useState(data.model || '')
  const [apiKey, setApiKey] = useState('')
  const [browserEnabled, setBrowserEnabled] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)
  const nameRef = useRef(null)

  // Focus name input on mount
  useEffect(() => {
    const t = setTimeout(() => nameRef.current?.focus(), 100)
    return () => clearTimeout(t)
  }, [])

  // Load model catalog once
  useEffect(() => {
    let cancelled = false
    fetchModels().then(m => {
      if (cancelled) return
      const cat = m || {}
      setCatalog(cat)
      const models = cat[provider] || []
      if (!model && models.length > 0) setModel(models[0])
    })
    return () => { cancelled = true }
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const providers = Object.keys(catalog)
  const providerModels = catalog[provider] || []
  const needsKey = provider !== 'ollama'
  const envKeyName = ENV_KEY_MAP[provider] || `${provider.toUpperCase()}_API_KEY`

  const handleProviderChange = useCallback((p) => {
    setProvider(p)
    setModel((catalog[p] || [])[0] || '')
    setApiKey('')
    updateData({ provider: p })
  }, [catalog, updateData])

  const canContinue =
    data.agentName.trim() &&
    model &&
    (!needsKey || apiKey.trim()) &&
    !saving

  const handleNext = async () => {
    if (!canContinue) return
    setSaving(true)
    setError(null)
    try {
      // Load → patch → save general config (name + model)
      const resp = await fetch('/api/settings/general', { headers: apiHeaders() })
      if (!resp.ok) throw new Error('Failed to load config')
      const config = await resp.json()
      const spec = `${provider}/${model}`
      config.agent_name = data.agentName.trim()
      config.models = [spec]

      const saveResp = await fetch('/api/settings/general', {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify(config),
      })
      if (!saveResp.ok) throw new Error('Failed to save settings')

      // LLM API key into vault
      if (needsKey && apiKey.trim()) {
        const vaultResp = await fetch(
          `/api/settings/vault/${encodeURIComponent(envKeyName)}`,
          {
            method: 'PUT',
            headers: apiHeaders(),
            body: JSON.stringify({ value: apiKey.trim() }),
          },
        )
        if (!vaultResp.ok) throw new Error('Failed to save API key')
      }

      // Browser automation toggle (best-effort — endpoint may not exist on older builds)
      try {
        const browserResp = await fetch('/api/settings/browser', { headers: apiHeaders() })
        if (browserResp.ok) {
          const browserCfg = await browserResp.json()
          if (browserCfg.enabled !== browserEnabled) {
            await fetch('/api/settings/browser', {
              method: 'PUT',
              headers: apiHeaders(),
              body: JSON.stringify({ ...browserCfg, enabled: browserEnabled }),
            })
          }
        }
      } catch {
        // browser settings unavailable — non-fatal
      }

      window.__STARPOD__ = {
        ...(window.__STARPOD__ || {}),
        agent_name: data.agentName.trim(),
      }
      updateData({ model: spec, browserEnabled })
      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  const handleNameKeyDown = (e) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      // jump focus into provider rather than submitting prematurely
      const next = e.currentTarget.form?.querySelector('select')
      next?.focus()
    }
  }

  const handleKeyKeyDown = (e) => {
    if (e.key === 'Enter' && canContinue) handleNext()
  }

  return (
    <form
      onSubmit={(e) => { e.preventDefault(); handleNext() }}
      noValidate
    >
      <h2 className="ob-heading">Set up your agent</h2>
      <p className="ob-desc">
        Give it a name, pick a brain, and decide whether it can browse the web.
      </p>

      {/* Name */}
      <div className="ob-field-group">
        <label className="ob-label" htmlFor="ob-name">Name</label>
        <input
          id="ob-name"
          ref={nameRef}
          type="text"
          className="ob-input-lg"
          value={data.agentName}
          onChange={e => updateData({ agentName: e.target.value })}
          placeholder="SEO Specialist"
          onKeyDown={handleNameKeyDown}
        />
      </div>

      {/* Model row */}
      <div className="ob-row-2">
        <div className="ob-field-group">
          <label className="ob-label" htmlFor="ob-provider">Provider</label>
          <select
            id="ob-provider"
            className="ob-select"
            value={provider}
            onChange={e => handleProviderChange(e.target.value)}
          >
            {providers.length === 0 && <option value={provider}>{provider}</option>}
            {providers.map(p => <option key={p} value={p}>{p}</option>)}
          </select>
        </div>

        <div className="ob-field-group">
          <label className="ob-label" htmlFor="ob-model">Model</label>
          <select
            id="ob-model"
            className="ob-select ob-select--mono"
            value={model}
            onChange={e => setModel(e.target.value)}
          >
            {providerModels.length === 0 && <option value="">—</option>}
            {providerModels.map(m => <option key={m} value={m}>{m}</option>)}
          </select>
        </div>
      </div>

      {needsKey ? (
        <div className="ob-field-group">
          <label className="ob-label" htmlFor="ob-key">{envKeyName}</label>
          <input
            id="ob-key"
            type="password"
            className="ob-input ob-input--mono"
            value={apiKey}
            onChange={e => setApiKey(e.target.value)}
            placeholder="sk-..."
            onKeyDown={handleKeyKeyDown}
            autoComplete="off"
          />
        </div>
      ) : (
        <p className="ob-hint">No API key required for local models.</p>
      )}

      {/* Browser toggle */}
      <div className="ob-cap-row">
        <div className="ob-cap-info">
          <div className="ob-cap-name">Browser automation</div>
          <div className="ob-cap-desc">
            Let your agent open pages and extract content (CDP / Lightpanda, beta).
          </div>
        </div>
        <button
          type="button"
          onClick={() => setBrowserEnabled(v => !v)}
          className={`ob-toggle ${browserEnabled ? 'ob-toggle--on' : ''}`}
          aria-pressed={browserEnabled}
          aria-label="Toggle browser automation"
        >
          <span className="ob-toggle-thumb" />
        </button>
      </div>

      {error && <p className="ob-error">{error}</p>}

      <div className="ob-actions ob-actions--end">
        <button
          type="submit"
          disabled={!canContinue}
          className="ob-btn-primary"
        >
          {saving ? 'Saving...' : 'Continue'}
        </button>
      </div>
    </form>
  )
}
