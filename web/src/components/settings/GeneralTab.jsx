import { useState, useEffect } from 'react'
import { apiHeaders, fetchModels } from '../../lib/api'
import { Card, Row, Field, Input, Select, ModelSelect, Toggle, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

/** Parse "provider/model" → { provider, model } */
function parseSpec(spec) {
  const idx = spec.indexOf('/')
  return idx >= 0 ? { provider: spec.slice(0, idx), model: spec.slice(idx + 1) } : { provider: '', model: spec }
}

/** Build "provider/model" from parts */
function buildSpec(provider, model) {
  return `${provider}/${model}`
}

export default function GeneralTab() {
  const [config, setConfig] = useState(null)
  const [catalog, setCatalog] = useState({}) // full model catalog by provider
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/general', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
    fetchModels().then(m => setCatalog(m || {}))
  }, [])

  if (!config) return <Loading />

  const configModels = config.models || []
  const providers = Object.keys(catalog)

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  // Update a model at a given index in the models list
  const setModelAt = (idx, spec) => {
    const next = [...configModels]
    next[idx] = spec
    set('models', next)
  }

  const addModel = () => {
    const firstProvider = providers[0] || 'anthropic'
    const firstModel = (catalog[firstProvider] || [])[0] || 'model'
    set('models', [...configModels, buildSpec(firstProvider, firstModel)])
  }

  const removeModel = (idx) => {
    set('models', configModels.filter((_, i) => i !== idx))
  }

  // Parse compaction model
  const compactionSpec = config.compaction_model ? parseSpec(config.compaction_model) : null

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/general', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Models" desc="first model is the default">
        {configModels.map((spec, idx) => {
          const { provider, model } = parseSpec(spec)
          const providerModels = catalog[provider] || []
          return (
            <div key={idx} className="flex items-center gap-2 mb-2">
              <Select
                value={provider}
                onChange={v => setModelAt(idx, buildSpec(v, (catalog[v] || [])[0] || model))}
                options={providers}
                className="w-32 shrink-0"
              />
              <div className="flex-1">
                <ModelSelect
                  value={model}
                  onChange={v => setModelAt(idx, buildSpec(provider, v))}
                  models={providerModels}
                />
              </div>
              {configModels.length > 1 && (
                <button
                  className="text-dim hover:text-err text-xs px-1"
                  onClick={() => removeModel(idx)}
                  title="Remove model"
                >
                  ×
                </button>
              )}
            </div>
          )
        })}
        <button
          className="text-xs text-muted hover:text-primary mt-1"
          onClick={addModel}
        >
          + Add model
        </button>
        <Row label="Reasoning" helpTip="Higher = more thinking time. Low is fastest, High most thorough. Only some models support this.">
          <Select value={config.reasoning_effort || ''} onChange={v => set('reasoning_effort', v || null)} options={[
            { value: '', label: 'Default' }, { value: 'low', label: 'Low' }, { value: 'medium', label: 'Medium' }, { value: 'high', label: 'High' },
          ]} />
        </Row>
      </Card>

      <Card title="Agent">
        <Row label="Name">
          <Input value={config.agent_name || ''} onChange={v => set('agent_name', v)} placeholder="Aster" />
        </Row>
        <Row label="Timezone">
          <Input value={config.timezone || ''} onChange={v => set('timezone', v)} placeholder="Europe/Rome" mono />
        </Row>
        <Row label="Followup mode" helpTip="Inject: followup messages join the current turn. Queue: waits for the turn to finish first.">
          <Select value={config.followup_mode || 'inject'} onChange={v => set('followup_mode', v)} options={[
            { value: 'inject', label: 'Inject' }, { value: 'queue', label: 'Queue' },
          ]} />
        </Row>
      </Card>

      <Card title="Compaction" desc="summarization model for long conversations">
        <Row label="Model" mono helpTip="provider/model format, or leave empty for same as primary">
          <Input
            value={config.compaction_model || ''}
            onChange={v => set('compaction_model', v || null)}
            placeholder="same as primary"
            mono
          />
        </Row>
      </Card>

      <Card title="Self-improve" desc="beta — agent learns from experience">
        <Toggle checked={config.self_improve} onChange={v => set('self_improve', v)}
          label="Enabled" helpTip="When on, the agent proactively creates skills from complex tasks and updates outdated skills during use." />
      </Card>

      <Card title="Limits">
        <Row label="Max turns" sub="per request">
          <Input type="number" value={config.max_turns ?? ''} onChange={v => set('max_turns', v === '' ? null : Number(v))} placeholder="200" />
        </Row>
        <Row label="Max tokens" sub="response limit">
          <Input type="number" value={config.max_tokens ?? ''} onChange={v => set('max_tokens', v === '' ? null : Number(v))} placeholder="16384" />
        </Row>
      </Card>

      <Card title="Server">
        <Row label="Bind address" helpTip="Requires restart to take effect.">
          <Input value={config.server_addr || ''} onChange={v => set('server_addr', v)} placeholder="0.0.0.0:3001" mono />
        </Row>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
