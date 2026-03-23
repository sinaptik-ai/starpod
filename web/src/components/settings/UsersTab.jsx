import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import SectionLabel from '../ui/SectionLabel'
import Badge from '../ui/Badge'
import { Loading, Empty } from '../ui/EmptyState'
import { ChevronDownIcon } from '../ui/Icons'

export default function UsersTab() {
  const [users, setUsers] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)

  // Create form
  const [showCreate, setShowCreate] = useState(false)
  const [newEmail, setNewEmail] = useState('')
  const [newName, setNewName] = useState('')
  const [newRole, setNewRole] = useState('user')
  const [creating, setCreating] = useState(false)

  // Expanded user
  const [expandedId, setExpandedId] = useState(null)
  const [editName, setEditName] = useState('')
  const [editEmail, setEditEmail] = useState('')
  const [editRole, setEditRole] = useState('user')
  const [editSaving, setEditSaving] = useState(false)
  const [editStatus, setEditStatus] = useState(null)

  // API keys
  const [apiKeys, setApiKeys] = useState([])
  const [keysLoading, setKeysLoading] = useState(false)
  const [newKeyLabel, setNewKeyLabel] = useState('')
  const [createdKey, setCreatedKey] = useState(null)
  const [keyError, setKeyError] = useState(null)

  // Telegram link
  const [telegramLink, setTelegramLink] = useState(null)
  const [telegramLoading, setTelegramLoading] = useState(false)
  const [newTelegramId, setNewTelegramId] = useState('')
  const [newTelegramUsername, setNewTelegramUsername] = useState('')
  const [telegramError, setTelegramError] = useState(null)
  const [telegramEnabled, setTelegramEnabled] = useState(false)

  const load = async () => {
    try {
      const r = await fetch('/api/settings/auth/users', { headers: apiHeaders() })
      if (r.ok) setUsers((await r.json()) || [])
      else setError('Failed to load users')
    } catch { setError('Failed to load users') }
    setLoading(false)
  }

  useEffect(() => {
    load()
    fetch('/api/settings/channels', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setTelegramEnabled(d?.telegram?.enabled ?? false))
      .catch(() => {})
  }, [])

  const create = async () => {
    setCreating(true); setError(null)
    try {
      const body = { role: newRole }
      if (newEmail.trim()) body.email = newEmail.trim()
      if (newName.trim()) body.display_name = newName.trim()
      const r = await fetch('/api/settings/auth/users', {
        method: 'POST', headers: apiHeaders(), body: JSON.stringify(body),
      })
      if (r.ok) {
        setNewEmail(''); setNewName(''); setNewRole('user'); setShowCreate(false)
        await load()
      } else {
        const d = await r.json().catch(() => ({}))
        setError(d.error || 'Failed to create user')
      }
    } catch (e) { setError(e.message) }
    setCreating(false)
  }

  const expand = async (user) => {
    if (expandedId === user.id) { setExpandedId(null); return }
    setExpandedId(user.id)
    setEditName(user.display_name || '')
    setEditEmail(user.email || '')
    setEditRole(user.role)
    setEditStatus(null)
    setCreatedKey(null)
    setKeyError(null)
    setTelegramLink(null)
    setTelegramError(null)
    setNewTelegramId('')
    setNewTelegramUsername('')
    // Load API keys
    setKeysLoading(true)
    try {
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(user.id)}/api-keys`, { headers: apiHeaders() })
      if (r.ok) setApiKeys((await r.json()) || [])
    } catch {}
    setKeysLoading(false)
    // Load telegram link
    if (telegramEnabled) {
      setTelegramLoading(true)
      try {
        const tr = await fetch(`/api/settings/auth/users/${encodeURIComponent(user.id)}/telegram`, { headers: apiHeaders() })
        if (tr.ok) {
          const data = await tr.json()
          setTelegramLink(data.telegram_id ? data : null)
        }
      } catch {}
      setTelegramLoading(false)
    }
  }

  const saveEdit = async () => {
    setEditSaving(true); setEditStatus(null)
    try {
      const body = { role: editRole }
      if (editEmail.trim()) body.email = editEmail.trim()
      if (editName.trim()) body.display_name = editName.trim()
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}`, {
        method: 'PUT', headers: apiHeaders(), body: JSON.stringify(body),
      })
      if (r.ok) {
        setEditStatus({ type: 'ok', text: 'Saved' })
        await load()
      } else {
        const d = await r.json().catch(() => ({}))
        setEditStatus({ type: 'error', text: d.error || 'Failed' })
      }
    } catch (e) { setEditStatus({ type: 'error', text: e.message }) }
    setEditSaving(false)
  }

  const toggleActive = async (user) => {
    const action = user.is_active ? 'deactivate' : 'activate'
    try {
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(user.id)}/${action}`, {
        method: 'POST', headers: apiHeaders(),
      })
      if (r.ok) await load()
      else {
        const d = await r.json().catch(() => ({}))
        setError(d.error || `Failed to ${action}`)
      }
    } catch (e) { setError(e.message) }
  }

  const createApiKey = async () => {
    setKeyError(null); setCreatedKey(null)
    try {
      const body = {}
      if (newKeyLabel.trim()) body.label = newKeyLabel.trim()
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}/api-keys`, {
        method: 'POST', headers: apiHeaders(), body: JSON.stringify(body),
      })
      if (r.ok) {
        const data = await r.json()
        setCreatedKey(data.key)
        setNewKeyLabel('')
        // Reload keys
        const kr = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}/api-keys`, { headers: apiHeaders() })
        if (kr.ok) setApiKeys((await kr.json()) || [])
      } else {
        const d = await r.json().catch(() => ({}))
        setKeyError(d.error || 'Failed to create key')
      }
    } catch (e) { setKeyError(e.message) }
  }

  const revokeKey = async (keyId) => {
    try {
      const r = await fetch(`/api/settings/auth/api-keys/${encodeURIComponent(keyId)}/revoke`, {
        method: 'POST', headers: apiHeaders(),
      })
      if (r.ok) {
        // Reload keys
        const kr = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}/api-keys`, { headers: apiHeaders() })
        if (kr.ok) setApiKeys((await kr.json()) || [])
      }
    } catch {}
  }

  const copyKey = (key) => {
    navigator.clipboard.writeText(key).catch(() => {})
  }

  const linkTelegram = async () => {
    setTelegramError(null)
    const tid = parseInt(newTelegramId, 10)
    if (!tid || isNaN(tid)) { setTelegramError('Invalid Telegram ID'); return }
    try {
      const body = { telegram_id: tid }
      if (newTelegramUsername.trim()) body.username = newTelegramUsername.trim()
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}/telegram`, {
        method: 'PUT', headers: apiHeaders(), body: JSON.stringify(body),
      })
      if (r.ok) {
        const data = await r.json()
        setTelegramLink(data)
        setNewTelegramId(''); setNewTelegramUsername('')
      } else {
        const d = await r.json().catch(() => ({}))
        setTelegramError(d.error || 'Failed to link')
      }
    } catch (e) { setTelegramError(e.message) }
  }

  const unlinkTelegram = async () => {
    setTelegramError(null)
    try {
      const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(expandedId)}/telegram`, {
        method: 'DELETE', headers: apiHeaders(),
      })
      if (r.ok) setTelegramLink(null)
      else {
        const d = await r.json().catch(() => ({}))
        setTelegramError(d.error || 'Failed to unlink')
      }
    } catch (e) { setTelegramError(e.message) }
  }

  if (loading) return <Loading />

  return (
    <>
      {/* Header + create button */}
      <div className="flex items-center justify-between mb-4">
        <div className="text-dim text-xs">{users.length} user{users.length !== 1 ? 's' : ''}</div>
        <button
          onClick={() => setShowCreate(!showCreate)}
          className="s-save-btn text-xs"
        >
          {showCreate ? 'Cancel' : 'New user'}
        </button>
      </div>

      {error && <div className="text-err text-xs mb-3">{error}</div>}

      {/* Create form */}
      {showCreate && (
        <div className="s-card mb-4">
          <div className="s-card-body">
            <div className="flex flex-col gap-2">
              <input
                type="text"
                className="s-input text-xs"
                value={newName}
                onChange={e => setNewName(e.target.value)}
                placeholder="Display name"
              />
              <input
                type="email"
                className="s-input text-xs"
                value={newEmail}
                onChange={e => setNewEmail(e.target.value)}
                placeholder="Email (optional)"
              />
              <div className="flex gap-2 items-center">
                <select
                  value={newRole}
                  onChange={e => setNewRole(e.target.value)}
                  className="s-input s-select text-xs flex-1"
                >
                  <option value="user">User</option>
                  <option value="admin">Admin</option>
                </select>
                <button onClick={create} disabled={creating} className="s-save-btn text-xs whitespace-nowrap">
                  {creating ? 'Creating...' : 'Create'}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* User list */}
      {users.length === 0 ? (
        <Empty text="No users yet. Create one to get started." />
      ) : (
        <div className="flex flex-col gap-1.5">
          {users.map(u => (
            <div key={u.id} className="user-card">
              <div
                className="flex items-center justify-between cursor-pointer"
                onClick={() => expand(u)}
              >
                <div className="flex items-center gap-3">
                  <span className="text-sm text-primary">
                    {u.display_name || u.email || u.id.slice(0, 8)}
                  </span>
                  <Badge variant={u.role === 'admin' ? 'accent' : 'muted'}>{u.role}</Badge>
                  {!u.is_active && <Badge variant="err">inactive</Badge>}
                </div>
                <div className="flex items-center gap-2">
                  {u.email && <span className="text-dim text-xs hidden sm:inline">{u.email}</span>}
                  <ChevronDownIcon className={`w-3 h-3 text-muted transition-transform ${expandedId === u.id ? 'rotate-180' : ''}`} />
                </div>
              </div>

              {/* Expanded panel */}
              {expandedId === u.id && (
                <div className="mt-3 pt-3 border-t border-border-subtle">
                  {/* Edit fields */}
                  <div className="flex flex-col gap-2 mb-4">
                    <SectionLabel>Profile</SectionLabel>
                    <input
                      type="text"
                      className="s-input text-xs"
                      value={editName}
                      onChange={e => setEditName(e.target.value)}
                      placeholder="Display name"
                    />
                    <input
                      type="email"
                      className="s-input text-xs"
                      value={editEmail}
                      onChange={e => setEditEmail(e.target.value)}
                      placeholder="Email"
                    />
                    <div className="flex gap-2 items-center">
                      <select
                        value={editRole}
                        onChange={e => setEditRole(e.target.value)}
                        className="s-input s-select text-xs flex-1"
                      >
                        <option value="user">User</option>
                        <option value="admin">Admin</option>
                      </select>
                      <button onClick={saveEdit} disabled={editSaving} className="s-save-btn text-xs">
                        {editSaving ? 'Saving...' : 'Save'}
                      </button>
                      <button
                        onClick={() => toggleActive(u)}
                        className={`text-xs px-3 py-1 rounded cursor-pointer transition-colors ${
                          u.is_active
                            ? 'bg-err/10 text-err hover:bg-err/20'
                            : 'bg-ok/10 text-ok hover:bg-ok/20'
                        }`}
                      >
                        {u.is_active ? 'Deactivate' : 'Activate'}
                      </button>
                    </div>
                    {editStatus && (
                      <span className={`text-xs ${editStatus.type === 'error' ? 'text-err' : 'text-ok'}`}>
                        {editStatus.text}
                      </span>
                    )}
                  </div>

                  {/* Features */}
                  <div className="pt-3 border-t border-border-subtle mb-4">
                    <SectionLabel className="mb-2">Features</SectionLabel>
                    <div className="flex items-center justify-between py-1.5 px-2 rounded bg-elevated/50">
                      <div className="flex flex-col">
                        <span className="text-xs text-primary">Filesystem Access</span>
                        <span className="text-[10px] text-dim">Browse and edit files in the instance sandbox</span>
                      </div>
                      <label className="s-toggle-wrap">
                        <input
                          type="checkbox"
                          checked={u.filesystem_enabled || false}
                          onChange={async (e) => {
                            const val = e.target.checked
                            try {
                              const r = await fetch(`/api/settings/auth/users/${encodeURIComponent(u.id)}`, {
                                method: 'PUT',
                                headers: apiHeaders(),
                                body: JSON.stringify({ filesystem_enabled: val }),
                              })
                              if (r.ok) await load()
                            } catch {}
                          }}
                          className="s-toggle-input"
                        />
                        <span className="s-toggle-track"><span className="s-toggle-thumb" /></span>
                      </label>
                    </div>
                  </div>

                  {/* API Keys */}
                  <div className="pt-3 border-t border-border-subtle">
                    <SectionLabel className="mb-2">API Keys</SectionLabel>

                    {/* Created key banner */}
                    {createdKey && (
                      <div className="bg-ok/10 border border-ok/20 rounded p-3 mb-3">
                        <div className="text-xs text-ok font-medium mb-1">Key created — copy it now, it won't be shown again</div>
                        <div className="flex gap-2 items-center">
                          <code className="text-xs font-mono text-primary bg-elevated px-2 py-1 rounded flex-1 break-all select-all">
                            {createdKey}
                          </code>
                          <button
                            onClick={() => copyKey(createdKey)}
                            className="text-xs text-muted hover:text-primary cursor-pointer shrink-0"
                          >
                            copy
                          </button>
                        </div>
                      </div>
                    )}

                    {keyError && <div className="text-err text-xs mb-2">{keyError}</div>}

                    {/* Create key */}
                    <div className="flex gap-2 mb-3">
                      <input
                        type="text"
                        className="s-input font-mono text-xs flex-1"
                        value={newKeyLabel}
                        onChange={e => setNewKeyLabel(e.target.value)}
                        placeholder="Key label (optional)"
                        onKeyDown={e => e.key === 'Enter' && createApiKey()}
                      />
                      <button onClick={createApiKey} className="s-save-btn text-xs whitespace-nowrap">
                        Create key
                      </button>
                    </div>

                    {/* Key list */}
                    {keysLoading ? (
                      <div className="text-dim text-xs">Loading keys...</div>
                    ) : apiKeys.length === 0 ? (
                      <div className="text-dim text-xs">No API keys</div>
                    ) : (
                      <div className="flex flex-col gap-1">
                        {apiKeys.map(k => (
                          <div key={k.id} className="flex items-center justify-between py-1.5 px-2 rounded bg-elevated/50">
                            <div className="flex items-center gap-2 min-w-0">
                              <code className="text-xs font-mono text-muted">{k.prefix}...</code>
                              {k.label && <span className="text-xs text-secondary truncate">{k.label}</span>}
                              {k.revoked_at && (
                                <Badge variant="err">revoked</Badge>
                              )}
                              {k.last_used_at && !k.revoked_at && (
                                <span className="text-[10px] text-dim">
                                  used {new Date(k.last_used_at).toLocaleDateString()}
                                </span>
                              )}
                            </div>
                            {!k.revoked_at && (
                              <button
                                onClick={() => revokeKey(k.id)}
                                className="text-xs text-muted hover:text-err cursor-pointer transition-colors shrink-0 ml-2"
                              >
                                revoke
                              </button>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Telegram Link */}
                  {telegramEnabled && (
                    <div className="pt-3 border-t border-border-subtle">
                      <SectionLabel className="mb-2">Telegram</SectionLabel>

                      {telegramError && <div className="text-err text-xs mb-2">{telegramError}</div>}

                      {telegramLoading ? (
                        <div className="text-dim text-xs">Loading...</div>
                      ) : telegramLink ? (
                        <div className="flex items-center justify-between py-1.5 px-2 rounded bg-elevated/50">
                          <div className="flex items-center gap-2">
                            <code className="text-xs font-mono text-primary">{telegramLink.telegram_id}</code>
                            {telegramLink.username && <span className="text-xs text-secondary">@{telegramLink.username}</span>}
                          </div>
                          <button
                            onClick={unlinkTelegram}
                            className="text-xs text-muted hover:text-err cursor-pointer transition-colors"
                          >
                            unlink
                          </button>
                        </div>
                      ) : (
                        <div className="flex gap-2">
                          <input
                            type="number"
                            className="s-input font-mono text-xs flex-1"
                            value={newTelegramId}
                            onChange={e => setNewTelegramId(e.target.value)}
                            placeholder="Telegram ID"
                          />
                          <input
                            type="text"
                            className="s-input text-xs flex-1"
                            value={newTelegramUsername}
                            onChange={e => setNewTelegramUsername(e.target.value)}
                            placeholder="Username (optional)"
                          />
                          <button onClick={linkTelegram} className="s-save-btn text-xs whitespace-nowrap">
                            Link
                          </button>
                        </div>
                      )}
                    </div>
                  )}

                  {/* User ID (small, for reference) */}
                  <div className="mt-3 pt-2 border-t border-border-subtle">
                    <span className="text-[10px] text-dim font-mono select-all">{u.id}</span>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </>
  )
}
