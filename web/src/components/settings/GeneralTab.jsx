import { useState, useEffect } from 'react'
import { apiHeaders, fetchModels } from '../../lib/api'
import { Section, Field, Input, Select, ModelSelect, SaveBar } from './fields'

export default function GeneralTab() {
  const [config, setConfig] = useState(null)
  const [models, setModels] = useState({})
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/general', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load settings' }))

    fetchModels().then(m => setModels(m || {}))
  }, [])

  if (!config) return <div className="text-dim text-sm">Loading...</div>

  const providers = Object.keys(models)
  const currentProvider = config.provider || providers[0] || ''
  const providerModels = models[currentProvider] || []
  const compactionProvider = config.compaction_provider || ''
  const compactionModels = compactionProvider && compactionProvider !== '' ? (models[compactionProvider] || []) : providerModels

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))

  const save = async () => {
    setSaving(true)
    setStatus(null)
    try {
      const resp = await fetch('/api/settings/general', {
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
      <Section title="Model" />
      <Field label="Provider">
        <Select id="provider" value={currentProvider} onChange={v => { set('provider', v); set('model', (models[v] || [])[0] || '') }} options={providers} />
      </Field>
      <Field label="Model">
        <ModelSelect value={config.model || ''} onChange={v => set('model', v)} models={providerModels} />
      </Field>
      <Field label="Reasoning effort" helpTip="Higher = more &quot;thinking time&quot; before responding. Low is fastest, High gives most thorough answers. Only supported by some models.">
        <Select id="reasoning_effort" value={config.reasoning_effort || 'medium'} onChange={v => set('reasoning_effort', v)} options={[
          { value: 'low', label: 'Low' },
          { value: 'medium', label: 'Medium' },
          { value: 'high', label: 'High' },
        ]} />
      </Field>

      <Section title="Agent" />
      <Field label="Agent name">
        <Input id="agent_name" value={config.agent_name || ''} onChange={v => set('agent_name', v)} placeholder="Aster" />
      </Field>
      <Field label="Timezone">
        <Input id="timezone" value={config.timezone || ''} onChange={v => set('timezone', v)} placeholder="America/New_York" />
      </Field>
      <Field label="Followup mode" helpTip="Inject: followup messages get woven into the current conversation turn. Queue: waits until the current turn finishes, then starts a new one.">
        <Select id="followup_mode" value={config.followup_mode || 'inject'} onChange={v => set('followup_mode', v)} options={[
          { value: 'inject', label: 'Inject' },
          { value: 'queue', label: 'Queue' },
        ]} />
      </Field>

      <Section title="Compaction" />
      <div className="text-dim text-xs mb-4">When a conversation grows too long, the agent compacts it using a summarization model. You can use a different (cheaper/faster) model for this.</div>
      <Field label="Compaction provider">
        <Select id="compaction_provider" value={compactionProvider} onChange={v => { set('compaction_provider', v); set('compaction_model', (models[v] || [])[0] || '') }} options={[{ value: '', label: 'Same as primary' }, ...providers.map(p => ({ value: p, label: p }))]} />
      </Field>
      <Field label="Compaction model">
        <ModelSelect value={config.compaction_model || ''} onChange={v => set('compaction_model', v)} models={compactionModels} />
      </Field>

      <Section title="Limits" />
      <div className="grid grid-cols-2 gap-4">
        <Field label="Max turns">
          <Input id="max_turns" type="number" value={config.max_turns ?? ''} onChange={v => set('max_turns', v === '' ? null : Number(v))} placeholder="200" />
        </Field>
        <Field label="Max tokens">
          <Input id="max_tokens" type="number" value={config.max_tokens ?? ''} onChange={v => set('max_tokens', v === '' ? null : Number(v))} placeholder="16384" />
        </Field>
      </div>

      <Section title="Server" />
      <Field label="Server address" helpTip="Requires restart to take effect.">
        <Input id="server_addr" value={config.server_addr || ''} onChange={v => set('server_addr', v)} placeholder="0.0.0.0:3001" />
      </Field>

      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
