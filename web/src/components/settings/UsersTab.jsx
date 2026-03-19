import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Textarea, SaveBar } from './fields'

export default function UsersTab() {
  const [users, setUsers] = useState([])
  const [loading, setLoading] = useState(true)
  const [newId, setNewId] = useState('')
  const [creating, setCreating] = useState(false)
  const [editId, setEditId] = useState(null)
  const [editContent, setEditContent] = useState('')
  const [editSaving, setEditSaving] = useState(false)
  const [editStatus, setEditStatus] = useState(null)
  const [confirmDelete, setConfirmDelete] = useState(null)
  const [error, setError] = useState(null)

  const load = async () => {
    try {
      const r = await fetch('/api/settings/users', { headers: apiHeaders() })
      if (r.ok) setUsers((await r.json()) || [])
    } catch { setError('Failed to load') }
    setLoading(false)
  }

  useEffect(() => { load() }, [])

  const create = async () => {
    if (!newId.trim()) return
    setCreating(true); setError(null)
    try {
      const r = await fetch('/api/settings/users', { method: 'POST', headers: apiHeaders(), body: JSON.stringify({ id: newId.trim() }) })
      if (r.ok) { setNewId(''); await load() }
      else { const d = await r.json().catch(() => ({})); setError(d.error || 'Failed') }
    } catch (e) { setError(e.message) }
    setCreating(false)
  }

  const startEdit = async (uid) => {
    if (editId === uid) { setEditId(null); return }
    setEditId(uid); setEditStatus(null)
    try {
      const r = await fetch(`/api/settings/users/${encodeURIComponent(uid)}`, { headers: apiHeaders() })
      if (r.ok) { const d = await r.json(); setEditContent(d.user_md || d.content || '') }
    } catch { setEditStatus({ type: 'error', text: 'Failed to load' }) }
  }

  const saveEdit = async () => {
    setEditSaving(true); setEditStatus(null)
    try {
      const r = await fetch(`/api/settings/users/${encodeURIComponent(editId)}`, { method: 'PUT', headers: apiHeaders(), body: JSON.stringify({ content: editContent }) })
      setEditStatus(r.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setEditStatus({ type: 'error', text: e.message }) }
    setEditSaving(false)
  }

  const doDelete = async (uid) => {
    setConfirmDelete(null)
    try {
      const r = await fetch(`/api/settings/users/${encodeURIComponent(uid)}`, { method: 'DELETE', headers: apiHeaders() })
      if (r.ok) { if (editId === uid) setEditId(null); await load() }
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
          value={newId}
          onChange={e => setNewId(e.target.value)}
          placeholder="user-id"
          onKeyDown={e => e.key === 'Enter' && create()}
        />
        <button onClick={create} disabled={creating || !newId.trim()} className="s-save-btn whitespace-nowrap">
          Create
        </button>
      </div>

      {error && <div className="text-err text-xs mb-3">{error}</div>}

      {users.length === 0 ? (
        <div className="text-dim text-sm text-center py-8">No users</div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {users.map(u => (
            <div key={u.id} className="user-card">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <span className="font-mono text-sm text-primary">{u.id}</span>
                  <span className="text-dim text-xs">{u.daily_log_count ?? 0} logs</span>
                </div>
                <div className="flex gap-3">
                  <button onClick={() => startEdit(u.id)} className="text-xs text-muted hover:text-primary transition-colors cursor-pointer">
                    {editId === u.id ? 'close' : 'edit'}
                  </button>
                  <button onClick={() => setConfirmDelete(u.id)} className="text-xs text-muted hover:text-err transition-colors cursor-pointer">
                    delete
                  </button>
                </div>
              </div>

              {editId === u.id && (
                <div className="mt-3 pt-3 border-t border-border-subtle">
                  <div className="font-mono text-[10px] text-dim mb-1.5 uppercase tracking-wider">USER.md</div>
                  <Textarea value={editContent} onChange={setEditContent} rows={10} />
                  <SaveBar onSave={saveEdit} saving={editSaving} status={editStatus} />
                </div>
              )}

              {confirmDelete === u.id && (
                <div className="mt-3 pt-3 border-t border-border-subtle flex items-center gap-3">
                  <span className="text-xs text-err">Delete all data for this user?</span>
                  <button onClick={() => doDelete(u.id)} className="text-xs bg-err/10 text-err px-3 py-1 rounded cursor-pointer hover:bg-err/20 transition-colors">Yes</button>
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
