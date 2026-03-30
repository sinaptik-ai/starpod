import { useState, useEffect } from 'react'
import { apiHeaders, fetchModels, fetchSystemVersion, triggerUpdate, pollHealthForVersion } from '../../lib/api'
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

function VersionCard() {
  const [info, setInfo] = useState(null)
  const [updating, setUpdating] = useState(false)
  const [updatePhase, setUpdatePhase] = useState(null) // 'downloading' | 'restarting' | 'done' | 'error'
  const [errorMsg, setErrorMsg] = useState(null)

  useEffect(() => {
    fetchSystemVersion().then(v => setInfo(v))
  }, [])

  const handleUpdate = async () => {
    if (!info?.latest) return
    setUpdating(true)
    setUpdatePhase('downloading')
    setErrorMsg(null)
    try {
      const result = await triggerUpdate()
      setUpdatePhase('restarting')
      const ok = await pollHealthForVersion(info.latest)
      if (ok) {
        setUpdatePhase('done')
        setTimeout(() => window.location.reload(), 1500)
      } else {
        setUpdatePhase('error')
        setErrorMsg('Restart timed out. Check server logs or restore from .starpod/backups/')
      }
    } catch (e) {
      setUpdatePhase('error')
      setErrorMsg(e.message)
      setUpdating(false)
    }
  }

  if (!info) {
    return (
      <Card title="Version">
        <Row label="Current version">
          <span className="text-secondary font-mono text-xs">loading...</span>
        </Row>
      </Card>
    )
  }

  return (
    <Card title="Version">
      <Row label="Current version">
        <span className="text-primary font-mono text-xs">v{info.current}</span>
      </Row>

      {info.update_available ? (
        <>
          <Row label="Latest version">
            <span className="text-primary font-mono text-xs">
              v{info.latest}
              {info.release_notes_url && (
                <>
                  {' '}&middot;{' '}
                  <a
                    href={info.release_notes_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-accent hover:underline"
                  >
                    What&apos;s new &#8599;
                  </a>
                </>
              )}
            </span>
          </Row>

          {!updating ? (
            <div className="px-4 pb-3 pt-1">
              <button
                onClick={handleUpdate}
                className="text-xs px-3 py-1.5 border border-accent text-accent hover:bg-accent hover:text-bg transition-colors cursor-pointer"
              >
                Update to v{info.latest}
              </button>
            </div>
          ) : (
            <div className="px-4 pb-3 pt-1">
              {updatePhase === 'downloading' && (
                <div className="text-xs text-secondary">Downloading v{info.latest}...</div>
              )}
              {updatePhase === 'restarting' && (
                <div className="text-xs text-secondary">Restarting... Starpod will reload automatically.</div>
              )}
              {updatePhase === 'done' && (
                <div className="text-xs text-ok">Updated successfully. Reloading...</div>
              )}
              {updatePhase === 'error' && (
                <div className="text-xs text-err">{errorMsg}</div>
              )}
            </div>
          )}
        </>
      ) : info.latest ? (
        <Row label="">
          <span className="text-dim text-xs">You&apos;re on the latest version</span>
        </Row>
      ) : (
        <Row label="">
          <span className="text-dim text-xs">Could not check for updates</span>
        </Row>
      )}
    </Card>
  )
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
      <VersionCard />

      <Card title="Models" desc="first model is the default">
        <div className="s-model-list">
          {configModels.map((spec, idx) => {
            const { provider, model } = parseSpec(spec)
            const providerModels = catalog[provider] || []
            return (
              <div key={idx} className="s-model-row">
                <div className="s-model-row-left">
                  {idx === 0 && <span className="s-model-badge">default</span>}
                  {idx > 0 && <span className="s-model-index">{idx + 1}</span>}
                  <select
                    className="s-model-provider"
                    value={provider}
                    onChange={e => setModelAt(idx, buildSpec(e.target.value, (catalog[e.target.value] || [])[0] || model))}
                  >
                    {providers.map(p => <option key={p} value={p}>{p}</option>)}
                  </select>
                </div>
                <div className="s-model-row-right">
                  <ModelSelect
                    value={model}
                    onChange={v => setModelAt(idx, buildSpec(provider, v))}
                    models={providerModels}
                  />
                  {configModels.length > 1 && (
                    <button
                      className="s-model-remove"
                      onClick={() => removeModel(idx)}
                      title="Remove model"
                    >
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>
                    </button>
                  )}
                </div>
              </div>
            )
          })}
        </div>
        <div className="s-model-actions">
          <button className="s-model-add" onClick={addModel}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M12 5v14M5 12h14"/></svg>
            Add model
          </button>
        </div>
        <Row label="Reasoning" helpTip="Higher = more thinking time. Low is fastest, High most thorough. Only some models support this.">
          <Select value={config.reasoning_effort || ''} onChange={v => set('reasoning_effort', v || null)} options={[
            { value: '', label: 'Default' }, { value: 'low', label: 'Low' }, { value: 'medium', label: 'Medium' }, { value: 'high', label: 'High' },
          ]} />
        </Row>
      </Card>

      <Card title="Agent">
        <Row label="Name">
          <Input value={config.agent_name || ''} onChange={v => set('agent_name', v)} placeholder="Nova" />
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

      <Card title="Secret Proxy (beta)" desc="When enabled, vault secrets are returned as opaque tokens. A local proxy swaps them for real values in outbound HTTP, preventing secrets from leaking into the LLM context.">
        <Toggle
          label="Enabled"
          checked={config.proxy_enabled || false}
          onChange={v => set('proxy_enabled', v)}
          helpTip="Requires restart. In production (serve), uses kernel-level network isolation on Linux."
        />
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
