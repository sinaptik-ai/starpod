import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Textarea, SaveBar } from './fields'

export default function FileTab({ fileName, description, rows = 20 }) {
  const [content, setContent] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    setLoading(true); setStatus(null)
    fetch(`/api/settings/files/${encodeURIComponent(fileName)}`, { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => { setContent(d.content || ''); setLoading(false) })
      .catch(() => { setStatus({ type: 'error', text: 'Failed to load' }); setLoading(false) })
  }, [fileName])

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch(`/api/settings/files/${encodeURIComponent(fileName)}`, {
        method: 'PUT', headers: apiHeaders(), body: JSON.stringify({ content }),
      })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  if (loading) return <div className="text-dim text-sm py-8 text-center">Loading...</div>

  return (
    <div>
      {description && <div className="text-dim text-xs mb-3 leading-relaxed">{description}</div>}
      <div className="border border-border-subtle rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-3 py-1.5 bg-surface border-b border-border-subtle">
          <span className="font-mono text-[11px] text-dim">{fileName}</span>
        </div>
        <Textarea value={content} onChange={setContent} rows={rows} />
      </div>
      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
