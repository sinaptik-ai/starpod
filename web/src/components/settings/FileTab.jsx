import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Textarea, SaveBar } from './fields'

export default function FileTab({ fileName, description, rows = 20 }) {
  const [content, setContent] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    setLoading(true)
    setStatus(null)
    fetch(`/api/settings/files/${encodeURIComponent(fileName)}`, { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => { setContent(d.content || ''); setLoading(false) })
      .catch(() => { setStatus({ type: 'error', text: 'Failed to load file' }); setLoading(false) })
  }, [fileName])

  const save = async () => {
    setSaving(true)
    setStatus(null)
    try {
      const resp = await fetch(`/api/settings/files/${encodeURIComponent(fileName)}`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({ content }),
      })
      if (resp.ok) setStatus({ type: 'ok', text: 'Saved' })
      else setStatus({ type: 'error', text: 'Save failed: ' + resp.statusText })
    } catch (e) {
      setStatus({ type: 'error', text: 'Save failed: ' + e.message })
    }
    setSaving(false)
  }

  if (loading) return <div className="text-dim text-sm">Loading...</div>

  return (
    <div>
      {description && <div className="text-dim text-xs mb-4">{description}</div>}
      <Textarea id={`file-${fileName}`} value={content} onChange={setContent} rows={rows} />
      <SaveBar onSave={save} saving={saving} status={status} />
    </div>
  )
}
