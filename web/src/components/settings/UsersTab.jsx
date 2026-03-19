import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { escapeHtml } from '../../lib/utils'
import { SaveBar } from './fields'

export default function UsersTab() {
  const [users, setUsers] = useState([])
  const [loading, setLoading] = useState(true)
  const [newUserId, setNewUserId] = useState('')
  const [creating, setCreating] = useState(false)
  const [editingId, setEditingId] = useState(null)
  const [editContent, setEditContent] = useState('')
  const [editSaving, setEditSaving] = useState(false)
  const [editStatus, setEditStatus] = useState(null)
  const [deleteConfirm, setDeleteConfirm] = useState(null)
  const [error, setError] = useState(null)

  const fetchUsers = async () => {
    try {
      const resp = await fetch('/api/settings/users', { headers: apiHeaders() })
      if (resp.ok) {
        const data = await resp.json()
        setUsers(data.users || [])
      }
    } catch {
      setError('Failed to load users')
    }
    setLoading(false)
  }

  useEffect(() => { fetchUsers() }, [])

  const createUser = async () => {
    if (!newUserId.trim()) return
    setCreating(true)
    setError(null)
    try {
      const resp = await fetch('/api/settings/users', {
        method: 'POST',
        headers: apiHeaders(),
        body: JSON.stringify({ user_id: newUserId.trim() }),
      })
      if (resp.ok) {
        setNewUserId('')
        await fetchUsers()
      } else {
        setError('Failed to create user: ' + resp.statusText)
      }
    } catch (e) {
      setError('Failed to create user: ' + e.message)
    }
    setCreating(false)
  }

  const startEdit = async (userId) => {
    if (editingId === userId) {
      setEditingId(null)
      return
    }
    setEditingId(userId)
    setEditStatus(null)
    try {
      const resp = await fetch(`/api/settings/users/${encodeURIComponent(userId)}`, { headers: apiHeaders() })
      if (resp.ok) {
        const data = await resp.json()
        setEditContent(data.content || '')
      }
    } catch {
      setEditStatus({ type: 'error', text: 'Failed to load user profile' })
    }
  }

  const saveEdit = async () => {
    setEditSaving(true)
    setEditStatus(null)
    try {
      const resp = await fetch(`/api/settings/users/${encodeURIComponent(editingId)}`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({ content: editContent }),
      })
      if (resp.ok) setEditStatus({ type: 'ok', text: 'Saved' })
      else setEditStatus({ type: 'error', text: 'Save failed: ' + resp.statusText })
    } catch (e) {
      setEditStatus({ type: 'error', text: 'Save failed: ' + e.message })
    }
    setEditSaving(false)
  }

  const deleteUser = async (userId) => {
    setDeleteConfirm(null)
    try {
      const resp = await fetch(`/api/settings/users/${encodeURIComponent(userId)}`, {
        method: 'DELETE',
        headers: apiHeaders(),
      })
      if (resp.ok) {
        if (editingId === userId) setEditingId(null)
        await fetchUsers()
      } else {
        setError('Failed to delete user: ' + resp.statusText)
      }
    } catch (e) {
      setError('Failed to delete user: ' + e.message)
    }
  }

  if (loading) return <div className="text-dim text-sm">Loading...</div>

  return (
    <div>
      {/* Create user */}
      <div className="flex gap-2 mb-6">
        <input
          type="text"
          className="settings-input flex-1"
          value={newUserId}
          onChange={e => setNewUserId(e.target.value)}
          placeholder="New user ID..."
          onKeyDown={e => e.key === 'Enter' && createUser()}
        />
        <button
          onClick={createUser}
          disabled={creating || !newUserId.trim()}
          className="bg-accent text-white px-4 py-2 rounded-lg text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-default whitespace-nowrap"
        >
          Create
        </button>
      </div>

      {error && <div className="text-err text-xs mb-4">{error}</div>}

      {/* User list */}
      {users.length === 0 ? (
        <div className="text-dim text-sm">No users yet.</div>
      ) : (
        <div>
          {users.map(user => (
            <div key={user.id} className="user-card">
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-primary text-sm font-medium">{escapeHtml(user.id)}</div>
                  <div className="text-dim text-xs mt-0.5">{user.daily_log_count ?? 0} daily log{(user.daily_log_count ?? 0) !== 1 ? 's' : ''}</div>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => startEdit(user.id)}
                    className="text-xs text-muted hover:text-primary transition-colors cursor-pointer"
                  >
                    {editingId === user.id ? 'Close' : 'Edit'}
                  </button>
                  <button
                    onClick={() => setDeleteConfirm(user.id)}
                    className="text-xs text-muted hover:text-err transition-colors cursor-pointer"
                  >
                    Delete
                  </button>
                </div>
              </div>

              {/* Edit area */}
              {editingId === user.id && (
                <div className="mt-3 pt-3 border-t border-border-subtle">
                  <div className="text-dim text-xs mb-2">USER.md</div>
                  <textarea
                    className="settings-input"
                    rows={10}
                    value={editContent}
                    onChange={e => setEditContent(e.target.value)}
                  />
                  <SaveBar onSave={saveEdit} saving={editSaving} status={editStatus} />
                </div>
              )}

              {/* Delete confirm */}
              {deleteConfirm === user.id && (
                <div className="mt-3 pt-3 border-t border-border-subtle flex items-center gap-3">
                  <span className="text-xs text-err">Delete this user and all their data?</span>
                  <button
                    onClick={() => deleteUser(user.id)}
                    className="text-xs bg-err text-white px-3 py-1 rounded-lg cursor-pointer hover:opacity-80 transition-opacity"
                  >
                    Confirm
                  </button>
                  <button
                    onClick={() => setDeleteConfirm(null)}
                    className="text-xs text-muted hover:text-primary cursor-pointer transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
