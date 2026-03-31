import { useState, useEffect } from 'react'
import { useApp } from '../../contexts/AppContext'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Field, Toggle, SaveBar } from './fields'
import { Loading } from '../ui/EmptyState'

export default function AttachmentsTab() {
  const { refreshConfig } = useApp()
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/attachments', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const set = (key, val) => setConfig(prev => ({ ...prev, [key]: val }))
  const extensions = config.allowed_extensions || []

  const updateExt = (i, val) => { const next = [...extensions]; next[i] = val.toLowerCase().replace(/[^a-z0-9]/g, ''); set('allowed_extensions', next) }
  const removeExt = (i) => { const next = [...extensions]; next.splice(i, 1); set('allowed_extensions', next) }
  const addExt = () => set('allowed_extensions', [...extensions, ''])

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      // Filter out empty extensions before saving
      const payload = { ...config, allowed_extensions: (config.allowed_extensions || []).filter(e => e.trim()) }
      const resp = await fetch('/api/settings/attachments', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(payload) })
      if (resp.ok) { setStatus({ type: 'ok', text: 'Saved' }); refreshConfig() }
      else setStatus({ type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  const sizeMB = config.max_file_size ? (config.max_file_size / (1024 * 1024)).toFixed(0) : ''

  return (
    <>
      <Card title="File uploads" desc="Control which files users can attach to messages">
        <Row label="Enabled" helpTip="When disabled, users cannot attach files to messages.">
          <Toggle checked={config.enabled} onChange={v => set('enabled', v)} />
        </Row>
        <Row label="Max file size" sub="MB" helpTip="Maximum size per file in megabytes. Files larger than this will be rejected.">
          <Input
            type="number"
            value={sizeMB}
            onChange={v => set('max_file_size', v === '' ? null : Number(v) * 1024 * 1024)}
            placeholder="20"
          />
        </Row>
        <Field label="Allowed extensions" desc="Leave empty to allow all file types. When set, only files with these extensions are accepted.">
          <div className="flex flex-col gap-1.5">
            {extensions.map((ext, i) => (
              <div key={i} className="prompt-item">
                <input
                  type="text"
                  className="s-input font-mono text-xs"
                  value={ext}
                  onChange={e => updateExt(i, e.target.value)}
                  placeholder="pdf"
                />
                <button onClick={() => removeExt(i)} title="Remove">&times;</button>
              </div>
            ))}
            <button onClick={addExt} className="text-accent text-xs hover:underline cursor-pointer self-start mt-0.5">
              + add extension
            </button>
          </div>
        </Field>
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
