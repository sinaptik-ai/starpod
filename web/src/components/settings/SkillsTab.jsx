import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Textarea, SaveBar } from './fields'

export default function SkillsTab() {
  const [skills, setSkills] = useState([])
  const [loading, setLoading] = useState(true)
  const [newName, setNewName] = useState('')
  const [creating, setCreating] = useState(false)
  const [editName, setEditName] = useState(null)
  const [editDesc, setEditDesc] = useState('')
  const [editBody, setEditBody] = useState('')
  const [editSaving, setEditSaving] = useState(false)
  const [editStatus, setEditStatus] = useState(null)
  const [confirmDelete, setConfirmDelete] = useState(null)
  const [error, setError] = useState(null)

  const load = async () => {
    try {
      const r = await fetch('/api/settings/skills', { headers: apiHeaders() })
      if (r.ok) setSkills((await r.json()) || [])
    } catch { setError('Failed to load') }
    setLoading(false)
  }

  useEffect(() => { load() }, [])

  const create = async () => {
    if (!newName.trim()) return
    setCreating(true); setError(null)
    try {
      const r = await fetch('/api/settings/skills', {
        method: 'POST', headers: apiHeaders(),
        body: JSON.stringify({ name: newName.trim(), description: '', body: '' }),
      })
      if (r.ok) { setNewName(''); await load() }
      else { const d = await r.json().catch(() => ({})); setError(d.error || 'Failed') }
    } catch (e) { setError(e.message) }
    setCreating(false)
  }

  const startEdit = async (name) => {
    if (editName === name) { setEditName(null); return }
    setEditName(name); setEditStatus(null)
    try {
      const r = await fetch(`/api/settings/skills/${encodeURIComponent(name)}`, { headers: apiHeaders() })
      if (r.ok) {
        const d = await r.json()
        setEditDesc(d.description || '')
        setEditBody(d.body || '')
      }
    } catch { setEditStatus({ type: 'error', text: 'Failed to load' }) }
  }

  const saveEdit = async () => {
    setEditSaving(true); setEditStatus(null)
    try {
      const r = await fetch(`/api/settings/skills/${encodeURIComponent(editName)}`, {
        method: 'PUT', headers: apiHeaders(),
        body: JSON.stringify({ description: editDesc, body: editBody }),
      })
      if (r.ok) {
        setEditStatus({ type: 'ok', text: 'Saved' })
        await load()
      } else {
        setEditStatus({ type: 'error', text: 'Failed' })
      }
    } catch (e) { setEditStatus({ type: 'error', text: e.message }) }
    setEditSaving(false)
  }

  const doDelete = async (name) => {
    setConfirmDelete(null)
    try {
      const r = await fetch(`/api/settings/skills/${encodeURIComponent(name)}`, { method: 'DELETE', headers: apiHeaders() })
      if (r.ok) { if (editName === name) setEditName(null); await load() }
      else setError('Failed to delete')
    } catch (e) { setError(e.message) }
  }

  if (loading) return <div className="text-dim text-sm py-8 text-center">Loading...</div>

  return (
    <>
      {/* Create */}
      <div className="flex gap-2 mb-4">
        <input
          type="text"
          className="s-input font-mono text-xs flex-1"
          value={newName}
          onChange={e => setNewName(e.target.value)}
          placeholder="skill-name"
          onKeyDown={e => e.key === 'Enter' && create()}
        />
        <button onClick={create} disabled={creating || !newName.trim()} className="s-save-btn whitespace-nowrap">
          Create
        </button>
      </div>

      {error && <div className="text-err text-xs mb-3">{error}</div>}

      {skills.length === 0 ? (
        <div className="text-dim text-sm text-center py-8">No skills</div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {skills.map(s => (
            <div key={s.name} className="skill-card">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3 min-w-0">
                  <span className="font-mono text-sm text-primary">{s.name}</span>
                  {s.version && <span className="text-[10px] text-dim bg-elevated px-1.5 py-0.5 rounded">{s.version}</span>}
                  {s.description && <span className="text-dim text-xs truncate">{s.description}</span>}
                </div>
                <div className="flex gap-3 shrink-0 ml-3">
                  <button onClick={() => startEdit(s.name)} className="text-xs text-muted hover:text-primary transition-colors cursor-pointer">
                    {editName === s.name ? 'close' : 'edit'}
                  </button>
                  <button onClick={() => setConfirmDelete(s.name)} className="text-xs text-muted hover:text-err transition-colors cursor-pointer">
                    delete
                  </button>
                </div>
              </div>

              {editName === s.name && (
                <div className="mt-3 pt-3 border-t border-border-subtle">
                  <div className="mb-3">
                    <div className="font-mono text-[10px] text-dim mb-1.5 uppercase tracking-wider">Description</div>
                    <input
                      type="text"
                      className="s-input text-xs w-full"
                      value={editDesc}
                      onChange={e => setEditDesc(e.target.value)}
                      placeholder="Short description of what this skill does"
                    />
                  </div>
                  <div className="font-mono text-[10px] text-dim mb-1.5 uppercase tracking-wider">Body</div>
                  <Textarea value={editBody} onChange={setEditBody} rows={15} />
                  <SaveBar onSave={saveEdit} saving={editSaving} status={editStatus} />
                </div>
              )}

              {confirmDelete === s.name && (
                <div className="mt-3 pt-3 border-t border-border-subtle flex items-center gap-3">
                  <span className="text-xs text-err">Delete this skill?</span>
                  <button onClick={() => doDelete(s.name)} className="text-xs bg-err/10 text-err px-3 py-1 rounded cursor-pointer hover:bg-err/20 transition-colors">Yes</button>
                  <button onClick={() => setConfirmDelete(null)} className="text-xs text-muted hover:text-primary cursor-pointer transition-colors">Cancel</button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </>
  )
}
