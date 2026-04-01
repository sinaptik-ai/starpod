import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card } from './fields'
import { Loading } from '../ui/EmptyState'

export default function VaultTab() {
  const [entries, setEntries] = useState(null)
  const [proxyEnabled, setProxyEnabled] = useState(false)
  const [status, setStatus] = useState(null)
  const [adding, setAdding] = useState(false)
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')
  const [newHosts, setNewHosts] = useState([])
  const [newHostInput, setNewHostInput] = useState('')
  const [editingKey, setEditingKey] = useState(null)
  const [editValue, setEditValue] = useState('')
  const [saving, setSaving] = useState(null)
  const [confirmDelete, setConfirmDelete] = useState(null)
  // Metadata editing (allowed_hosts)
  const [editingMeta, setEditingMeta] = useState(null)
  const [metaHosts, setMetaHosts] = useState([])
  const [metaHostInput, setMetaHostInput] = useState('')

  const load = () => {
    fetch('/api/settings/vault', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => {
        setEntries(d.entries || [])
        setProxyEnabled(d.proxy_enabled || false)
      })
      .catch(() => setStatus({ type: 'error', text: 'Failed to load vault' }))
  }

  useEffect(load, [])

  if (!entries) return <Loading />

  const handleAdd = async () => {
    if (!newKey.trim() || !newValue.trim()) return
    setSaving('__new__')
    setStatus(null)
    try {
      const resp = await fetch(`/api/settings/vault/${encodeURIComponent(newKey.trim())}`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({
          value: newValue,
          allowed_hosts: newHosts.length > 0 ? newHosts : null,
        }),
      })
      if (resp.ok) {
        setNewKey('')
        setNewValue('')
        setNewHosts([])
        setNewHostInput('')
        setAdding(false)
        setStatus({ type: 'ok', text: `Added ${newKey.trim()}` })
        load()
      } else {
        const err = await resp.json().catch(() => ({}))
        setStatus({ type: 'error', text: err.error || 'Failed to add' })
      }
    } catch (e) {
      setStatus({ type: 'error', text: e.message })
    }
    setSaving(null)
  }

  const handleUpdate = async (key) => {
    if (!editValue.trim()) return
    setSaving(key)
    setStatus(null)
    try {
      // Preserve existing metadata when just updating the value
      const entry = entries.find(e => e.key === key)
      const resp = await fetch(`/api/settings/vault/${encodeURIComponent(key)}`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({
          value: editValue,
          allowed_hosts: entry?.allowed_hosts || null,
        }),
      })
      if (resp.ok) {
        setEditingKey(null)
        setEditValue('')
        setStatus({ type: 'ok', text: `Updated ${key}` })
        load()
      } else {
        setStatus({ type: 'error', text: 'Failed to update' })
      }
    } catch (e) {
      setStatus({ type: 'error', text: e.message })
    }
    setSaving(null)
  }

  const handleMetaSave = async (key) => {
    setSaving(key)
    setStatus(null)
    try {
      const resp = await fetch(`/api/settings/vault/${encodeURIComponent(key)}/meta`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({
          allowed_hosts: metaHosts.length > 0 ? metaHosts : null,
        }),
      })
      if (resp.ok) {
        setEditingMeta(null)
        setMetaHosts([])
        setMetaHostInput('')
        setStatus({ type: 'ok', text: `Updated ${key} settings` })
        load()
      } else {
        setStatus({ type: 'error', text: 'Failed to update settings' })
      }
    } catch (e) {
      setStatus({ type: 'error', text: e.message })
    }
    setSaving(null)
  }

  const handleDelete = async (key) => {
    setSaving(key)
    setStatus(null)
    setConfirmDelete(null)
    try {
      const resp = await fetch(`/api/settings/vault/${encodeURIComponent(key)}`, {
        method: 'DELETE',
        headers: apiHeaders(),
      })
      if (resp.ok) {
        setStatus({ type: 'ok', text: `Deleted ${key}` })
        load()
      } else {
        setStatus({ type: 'error', text: 'Failed to delete' })
      }
    } catch (e) {
      setStatus({ type: 'error', text: e.message })
    }
    setSaving(null)
  }

  const addHost = (host, setHosts, setInput) => {
    const h = host.trim().toLowerCase()
    if (h) {
      setHosts(prev => prev.includes(h) ? prev : [...prev, h])
      setInput('')
    }
  }

  const removeHost = (host, setHosts) => {
    setHosts(prev => prev.filter(h => h !== host))
  }

  const handleHostKeyDown = (e, hosts, setHosts, setInput) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault()
      addHost(e.target.value, setHosts, setInput)
    } else if (e.key === 'Backspace' && !e.target.value && hosts.length > 0) {
      setHosts(prev => prev.slice(0, -1))
    }
  }

  const HostTags = ({ hosts, onRemove }) => (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px' }}>
      {hosts.map(h => (
        <span key={h} style={{
          display: 'inline-flex', alignItems: 'center', gap: '4px',
          padding: '2px 8px', fontSize: '11px', fontFamily: 'var(--font-mono)',
          border: '1px solid var(--color-border-main)', color: 'var(--color-secondary)',
        }}>
          {h}
          <button
            onClick={() => onRemove(h)}
            style={{ background: 'none', border: 'none', color: 'var(--color-dim)',
              cursor: 'pointer', padding: 0, fontSize: '11px', lineHeight: 1 }}
          >&times;</button>
        </span>
      ))}
    </div>
  )

  return (
    <>
      <Card
        title="Vault"
        desc="Encrypted credential storage. Values are AES-256 encrypted and never sent to the browser."
      >
        <div className="s-card-body" style={{ padding: 0 }}>
          {/* Add button / form */}
          {adding ? (
            <div className="s-row" style={{ flexDirection: 'column', alignItems: 'stretch', gap: '8px', padding: '12px 16px' }}>
              <input
                className="s-input"
                placeholder="KEY_NAME"
                value={newKey}
                onChange={e => setNewKey(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, ''))}
                style={{ fontFamily: 'var(--font-mono)', fontSize: '13px' }}
                autoFocus
              />
              <input
                className="s-input"
                type="password"
                placeholder="Value"
                value={newValue}
                onChange={e => setNewValue(e.target.value)}
              />

              {/* Allowed hosts */}
              {proxyEnabled && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                  <span style={{ fontSize: '11px', color: 'var(--color-dim)' }}>
                    Allowed hosts {newHosts.length === 0 && '(unrestricted)'}
                  </span>
                  <HostTags hosts={newHosts} onRemove={h => removeHost(h, setNewHosts)} />
                  <input
                    className="s-input"
                    placeholder="api.example.com (Enter to add)"
                    value={newHostInput}
                    onChange={e => setNewHostInput(e.target.value)}
                    onKeyDown={e => handleHostKeyDown(e, newHosts, setNewHosts, setNewHostInput)}
                    onBlur={() => addHost(newHostInput, setNewHosts, setNewHostInput)}
                    style={{ fontFamily: 'var(--font-mono)', fontSize: '12px' }}
                  />
                </div>
              )}

              <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
                <button
                  className="s-save-btn"
                  style={{ background: 'transparent', color: 'var(--color-muted)', border: '1px solid var(--color-border-main)' }}
                  onClick={() => { setAdding(false); setNewKey(''); setNewValue(''); setNewHosts([]); setNewHostInput('') }}
                >
                  Cancel
                </button>
                <button
                  className="s-save-btn"
                  disabled={!newKey.trim() || !newValue.trim() || saving === '__new__'}
                  onClick={handleAdd}
                >
                  {saving === '__new__' ? 'Adding...' : 'Add'}
                </button>
              </div>
            </div>
          ) : (
            <div
              className="s-row"
              style={{ cursor: 'pointer', justifyContent: 'center', color: 'var(--color-muted)' }}
              onClick={() => setAdding(true)}
            >
              + Add variable
            </div>
          )}

          {/* Entries */}
          {entries.length === 0 && !adding && (
            <div style={{ padding: '24px 16px', textAlign: 'center', color: 'var(--color-dim)' }}>
              No vault entries yet
            </div>
          )}

          {entries.map(entry => (
            <div key={entry.key} className="s-row" style={{ flexDirection: 'column', alignItems: 'stretch', gap: '4px' }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
                  <span style={{ fontFamily: 'var(--font-mono)', fontSize: '13px', color: 'var(--color-primary)' }}>
                    {entry.key}
                  </span>
                  {entry.is_system && (
                    <span style={{
                      fontSize: '10px', padding: '1px 6px',
                      border: '1px solid var(--color-border-main)',
                      color: 'var(--color-dim)', textTransform: 'uppercase', letterSpacing: '0.05em',
                    }}>
                      system
                    </span>
                  )}
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                  {editingKey !== entry.key && editingMeta !== entry.key && (
                    <>
                      <button
                        style={{ background: 'none', border: 'none', color: 'var(--color-muted)', cursor: 'pointer', padding: '4px', fontSize: '12px' }}
                        onClick={() => { setEditingKey(entry.key); setEditValue('') }}
                        title="Update value"
                      >
                        edit
                      </button>
                      {proxyEnabled && (
                        <button
                          style={{ background: 'none', border: 'none', color: 'var(--color-muted)', cursor: 'pointer', padding: '4px', fontSize: '12px' }}
                          onClick={() => {
                            setEditingMeta(entry.key)
                            setMetaHosts(entry.allowed_hosts || [])
                            setMetaHostInput('')
                          }}
                          title="Edit proxy settings"
                        >
                          hosts
                        </button>
                      )}
                    </>
                  )}
                  {confirmDelete === entry.key ? (
                    <div style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
                      <span style={{ fontSize: '12px', color: 'var(--color-err)' }}>delete?</span>
                      <button
                        style={{ background: 'none', border: 'none', color: 'var(--color-err)', cursor: 'pointer', padding: '4px', fontSize: '12px' }}
                        onClick={() => handleDelete(entry.key)}
                      >
                        yes
                      </button>
                      <button
                        style={{ background: 'none', border: 'none', color: 'var(--color-muted)', cursor: 'pointer', padding: '4px', fontSize: '12px' }}
                        onClick={() => setConfirmDelete(null)}
                      >
                        no
                      </button>
                    </div>
                  ) : (
                    <button
                      style={{ background: 'none', border: 'none', color: 'var(--color-dim)', cursor: 'pointer', padding: '4px', fontSize: '12px' }}
                      onClick={() => setConfirmDelete(entry.key)}
                      title="Delete"
                      disabled={saving === entry.key}
                    >
                      del
                    </button>
                  )}
                </div>
              </div>

              {/* Allowed hosts display (when not editing) */}
              {proxyEnabled && entry.allowed_hosts && entry.allowed_hosts.length > 0 && editingMeta !== entry.key && (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px', marginTop: '2px' }}>
                  {entry.allowed_hosts.map(h => (
                    <span key={h} style={{
                      padding: '1px 6px', fontSize: '10px', fontFamily: 'var(--font-mono)',
                      border: '1px solid var(--color-border-subtle)', color: 'var(--color-dim)',
                    }}>
                      {h}
                    </span>
                  ))}
                </div>
              )}
              {proxyEnabled && (!entry.allowed_hosts || entry.allowed_hosts.length === 0) && editingMeta !== entry.key && (
                <span style={{ fontSize: '10px', color: 'var(--color-dim)', marginTop: '2px' }}>
                  unrestricted
                </span>
              )}

              {/* Inline edit form */}
              {editingKey === entry.key && (
                <div style={{ display: 'flex', gap: '8px', marginTop: '4px' }}>
                  <input
                    className="s-input"
                    type="password"
                    placeholder="New value"
                    value={editValue}
                    onChange={e => setEditValue(e.target.value)}
                    style={{ flex: 1 }}
                    autoFocus
                  />
                  <button
                    className="s-save-btn"
                    style={{ background: 'transparent', color: 'var(--color-muted)', border: '1px solid var(--color-border-main)', padding: '4px 12px', fontSize: '12px' }}
                    onClick={() => { setEditingKey(null); setEditValue('') }}
                  >
                    Cancel
                  </button>
                  <button
                    className="s-save-btn"
                    style={{ padding: '4px 12px', fontSize: '12px' }}
                    disabled={!editValue.trim() || saving === entry.key}
                    onClick={() => handleUpdate(entry.key)}
                  >
                    {saving === entry.key ? 'Saving...' : 'Save'}
                  </button>
                </div>
              )}

              {/* Metadata edit form (allowed_hosts) */}
              {editingMeta === entry.key && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', marginTop: '4px', padding: '8px', border: '1px solid var(--color-border-subtle)' }}>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                    <span style={{ fontSize: '11px', color: 'var(--color-dim)' }}>
                      Allowed hosts {metaHosts.length === 0 && '(unrestricted)'}
                    </span>
                    <HostTags hosts={metaHosts} onRemove={h => removeHost(h, setMetaHosts)} />
                    <input
                      className="s-input"
                      placeholder="api.example.com (Enter to add)"
                      value={metaHostInput}
                      onChange={e => setMetaHostInput(e.target.value)}
                      onKeyDown={e => handleHostKeyDown(e, metaHosts, setMetaHosts, setMetaHostInput)}
                      onBlur={() => addHost(metaHostInput, setMetaHosts, setMetaHostInput)}
                      style={{ fontFamily: 'var(--font-mono)', fontSize: '12px' }}
                      autoFocus
                    />
                  </div>

                  <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
                    <button
                      className="s-save-btn"
                      style={{ background: 'transparent', color: 'var(--color-muted)', border: '1px solid var(--color-border-main)', padding: '4px 12px', fontSize: '12px' }}
                      onClick={() => { setEditingMeta(null); setMetaHosts([]); setMetaHostInput('') }}
                    >
                      Cancel
                    </button>
                    <button
                      className="s-save-btn"
                      style={{ padding: '4px 12px', fontSize: '12px' }}
                      disabled={saving === entry.key}
                      onClick={() => handleMetaSave(entry.key)}
                    >
                      {saving === entry.key ? 'Saving...' : 'Save'}
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      </Card>

      {/* Status feedback */}
      {status && (
        <div style={{
          padding: '8px 16px',
          fontSize: '13px',
          color: status.type === 'ok' ? 'var(--color-ok)' : 'var(--color-err)',
        }}>
          {status.text}
        </div>
      )}
    </>
  )
}
