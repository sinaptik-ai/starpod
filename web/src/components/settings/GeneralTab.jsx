import { useState, useEffect } from 'react'
import { apiHeaders, fetchModels } from '../../lib/api'
import { Card, Row, Field, Input, Select, ModelSelect, Toggle, SaveBar } from './fields'

export default function GeneralTab() {
  const [config, setConfig] = useState(null)
  const [models, setModels] = useState({})
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/general', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
    fetchModels().then(m => setModels(m || {}))
  }, [])

  if (!config) return <div className="text-dim text-sm py-8 text-center">Loading...</div>

  const providers = Object.keys(models)
  const currentProvider = config.provider || providers[0] || ''
  const providerModels = models[currentProvider] || []
  const cp = config.compaction_provider || ''
  const compactionModels = cp ? (models[cp] || []) : providerModels

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

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
      <Card title="Model">
        <Row label="Provider">
          <Select value={currentProvider} onChange={v => { set('provider', v); set('model', (models[v] || [])[0] || '') }} options={providers} />
        </Row>
        <Row label="Model" mono>
          <ModelSelect value={config.model || ''} onChange={v => set('model', v)} models={providerModels} />
        </Row>
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
          <Input value={config.timezone || ''} onChange={v => set('timezone', v)} placeholder="America/New_York" mono />
        </Row>
        <Row label="Followup mode" helpTip="Inject: followup messages join the current turn. Queue: waits for the turn to finish first.">
          <Select value={config.followup_mode || 'inject'} onChange={v => set('followup_mode', v)} options={[
            { value: 'inject', label: 'Inject' }, { value: 'queue', label: 'Queue' },
          ]} />
        </Row>
      </Card>

      <Card title="Compaction" desc="summarization model for long conversations">
        <Row label="Provider">
          <Select value={cp} onChange={v => { set('compaction_provider', v || null); set('compaction_model', (models[v] || [])[0] || '') }} options={[{ value: '', label: 'Same as primary' }, ...providers.map(p => ({ value: p, label: p }))]} />
        </Row>
        <Row label="Model" mono>
          <ModelSelect value={config.compaction_model || ''} onChange={v => set('compaction_model', v)} models={compactionModels} />
        </Row>
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
