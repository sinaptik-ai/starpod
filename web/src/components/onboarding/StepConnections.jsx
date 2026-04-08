import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { apiHeaders } from '../../lib/api'
import { LOGOS, FALLBACK_LOGO } from '../settings/connectorLogos'

// ── Categories ────────────────────────────────────────────────────────────
// Hardcoded grouping by connector template name. Anything not listed
// drops into "Other" so new connectors don't disappear from the grid.

const CATEGORIES = [
  { label: 'Communication', items: ['slack', 'discord', 'telegram', 'twilio', 'sendgrid', 'smtp'] },
  { label: 'Developer', items: ['github', 'github-apps', 'vercel', 'cloudflare', 'sentry', 'datadog'] },
  { label: 'Productivity', items: ['notion', 'linear', 'jira', 'google-calendar', 'google-sheets', 'hubspot'] },
  { label: 'Data', items: ['postgres', 'mysql', 'redis', 'mongodb', 'elasticsearch', 'supabase'] },
  { label: 'Cloud', items: ['aws', 'google-cloud', 'azure', 'digitalocean'] },
  { label: 'Commerce', items: ['stripe', 'shopify', 'meta-ads'] },
  { label: 'AI', items: ['openai', 'anthropic'] },
]

function buildCategorized(templates) {
  const byName = new Map(templates.map(t => [t.name, t]))
  const claimed = new Set()
  const sections = []
  for (const cat of CATEGORIES) {
    const items = cat.items
      .map(name => byName.get(name))
      .filter(Boolean)
      .sort((a, b) => a.display_name.localeCompare(b.display_name))
    items.forEach(t => claimed.add(t.name))
    if (items.length > 0) sections.push({ label: cat.label, items })
  }
  const other = templates
    .filter(t => !claimed.has(t.name))
    .sort((a, b) => a.display_name.localeCompare(b.display_name))
  if (other.length > 0) sections.push({ label: 'Other', items: other })
  return sections
}

// ── Tile (module-level, pure) ────────────────────────────────────────────

function ConnectorTile({ tpl, isEquipped, isReady, onClick }) {
  const cls =
    'ob-conn-tile' +
    (isEquipped ? ' ob-conn-tile--equipped' : '') +
    (isReady ? ' ob-conn-tile--ready' : '')
  return (
    <button type="button" onClick={onClick} className={cls} title={tpl.display_name}>
      {isEquipped && (
        <span className="ob-conn-check" aria-hidden="true">
          <svg viewBox="0 0 12 12" width="10" height="10">
            <path d="M2 6.5L5 9l5-6" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </span>
      )}
      <span className="ob-conn-logo">{LOGOS[tpl.name] || FALLBACK_LOGO}</span>
      <span className="ob-conn-name">{tpl.display_name}</span>
    </button>
  )
}

// ── Step component ───────────────────────────────────────────────────────

export default function StepConnections({ data, updateData, onNext, onBack }) {
  const [templates, setTemplates] = useState([])
  const [connectors, setConnectors] = useState([])
  const [vaultKeys, setVaultKeys] = useState(new Set())
  const [oauthProxyUrl, setOauthProxyUrl] = useState(null)
  const [proxyProviders, setProxyProviders] = useState({})
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState('')
  const [pending, setPending] = useState(new Set()) // template names in-flight
  const [expanded, setExpanded] = useState(null) // chip → inline credential pane
  const [secretValues, setSecretValues] = useState({})
  const [busy, setBusy] = useState(null)
  const [oauthSession, setOauthSession] = useState(null)
  const [error, setError] = useState(null)
  const [toast, setToast] = useState(null)
  const toastTimer = useRef(null)
  // Custom API connector panel state. Opened by the "+ Custom API" tile,
  // lets the user paste a name + api key + docs URL; the server fetches
  // the docs, generates a SKILL.md via an LLM call, and registers a
  // connector of type "custom".
  const [customOpen, setCustomOpen] = useState(false)
  const [customForm, setCustomForm] = useState({
    name: '',
    api_key: '',
    docs_url: '',
    description: '',
  })
  const [customBusy, setCustomBusy] = useState(false)
  const [customResult, setCustomResult] = useState(null)

  const showToast = useCallback((text) => {
    setToast(text)
    if (toastTimer.current) clearTimeout(toastTimer.current)
    toastTimer.current = setTimeout(() => setToast(null), 2000)
  }, [])

  useEffect(() => () => {
    if (toastTimer.current) clearTimeout(toastTimer.current)
  }, [])

  const reload = useCallback(async () => {
    try {
      const [tplResp, connResp, vaultResp] = await Promise.all([
        fetch('/api/settings/connector-templates', { headers: apiHeaders() }),
        fetch('/api/settings/connectors', { headers: apiHeaders() }),
        fetch('/api/settings/vault', { headers: apiHeaders() }),
      ])
      const t = await tplResp.json()
      const c = await connResp.json()
      const v = await vaultResp.json()
      setTemplates(Array.isArray(t) ? t : t.templates || [])
      setConnectors(Array.isArray(c) ? c : c.connectors || [])
      const entries = v.entries || (Array.isArray(v) ? v : [])
      setVaultKeys(new Set(entries.map(e => e.key)))
    } catch (e) {
      setError(e.message)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    Promise.all([
      reload(),
      fetch('/api/config', { headers: apiHeaders() })
        .then(r => r.ok ? r.json() : null)
        .catch(() => null),
    ]).then(async ([, cfg]) => {
      if (cancelled) return
      if (cfg?.oauth_proxy_url) {
        setOauthProxyUrl(cfg.oauth_proxy_url)
        try {
          const resp = await fetch(`${cfg.oauth_proxy_url}/api/v1/oauth/providers`)
          if (resp.ok) {
            const data = await resp.json()
            const arr = Array.isArray(data) ? data : data.providers || []
            const map = {}
            arr.forEach(p => { map[p.name] = { scopes: p.scopes } })
            if (!cancelled) setProxyProviders(map)
          }
        } catch {
          // proxy unreachable — fall back to manual credential entry
        }
      }
      setLoading(false)
    })
    return () => { cancelled = true }
  }, [reload])

  const categorized = useMemo(() => buildCategorized(templates), [templates])

  const searchResults = useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return null
    return templates
      .filter(
        t =>
          t.name.toLowerCase().includes(q) ||
          t.display_name.toLowerCase().includes(q) ||
          (t.description || '').toLowerCase().includes(q),
      )
      .sort((a, b) => a.display_name.localeCompare(b.display_name))
  }, [templates, search])

  // Manifest — one chip per equipped connector instance.
  const manifest = useMemo(
    () =>
      connectors.map(c => {
        const tpl = templates.find(t => t.name === c.type)
        return {
          type: c.type,
          instance: c.name,
          name: tpl?.display_name || c.type,
          description: tpl?.description || '',
          ready: c.status === 'connected',
          tpl,
        }
      }),
    [connectors, templates],
  )

  // Rich data handed off to Step 3 — everything in the manifest counts,
  // even un-credentialed. The role generator should know about the tool,
  // and the user can wire it up later.
  const connectedRich = useMemo(
    () =>
      manifest.map(m => ({
        type: m.type,
        instance: m.instance,
        name: m.name,
        description: m.description,
      })),
    [manifest],
  )

  const needsSetupCount = manifest.filter(m => !m.ready).length

  // ── Equip / Unequip (optimistic) ────────────────────────────────────────

  const equipTemplate = useCallback(
    async (tpl) => {
      setError(null)
      setPending(prev => { const n = new Set(prev); n.add(tpl.name); return n })
      try {
        const resp = await fetch('/api/settings/connectors', {
          method: 'POST',
          headers: apiHeaders(),
          body: JSON.stringify({ type: tpl.name }),
        })
        if (!resp.ok) {
          const d = await resp.json().catch(() => ({}))
          throw new Error(d.error || `Failed to add ${tpl.display_name}`)
        }
        await reload()
      } catch (e) {
        setError(e.message)
      } finally {
        setPending(prev => { const n = new Set(prev); n.delete(tpl.name); return n })
      }
    },
    [reload],
  )

  const unequipTemplate = useCallback(
    async (tpl, instanceName) => {
      setError(null)
      setPending(prev => { const n = new Set(prev); n.add(tpl.name); return n })
      // Optimistic local removal so the tile toggles off instantly.
      setConnectors(prev => prev.filter(c => c.name !== instanceName))
      if (expanded === instanceName) setExpanded(null)
      try {
        const resp = await fetch(
          `/api/settings/connectors/${encodeURIComponent(instanceName)}`,
          { method: 'DELETE', headers: apiHeaders() },
        )
        if (!resp.ok) throw new Error('Failed to remove')
        await reload()
      } catch (e) {
        setError(e.message)
        await reload() // rollback from server truth
      } finally {
        setPending(prev => { const n = new Set(prev); n.delete(tpl.name); return n })
      }
    },
    [reload, expanded],
  )

  const handleTileClick = useCallback(
    (tpl) => {
      if (pending.has(tpl.name)) return
      const existing = connectors.find(c => c.type === tpl.name)
      if (existing) {
        unequipTemplate(tpl, existing.name)
      } else {
        equipTemplate(tpl)
      }
    },
    [connectors, pending, equipTemplate, unequipTemplate],
  )

  // ── Optional credential flow (chip-initiated, opt-in) ──────────────────

  const expandedEntry = expanded ? manifest.find(m => m.instance === expanded) : null
  const expandedTpl = expandedEntry?.tpl || null

  const proxyKey = useMemo(() => {
    if (!expandedTpl) return null
    return Object.keys(proxyProviders).find(k => {
      const base = k.replace(/-connector$/, '')
      return expandedTpl.name === base || expandedTpl.name === k
    })
  }, [expandedTpl, proxyProviders])

  const handleProxyOAuth = async () => {
    if (!expandedTpl || !expandedEntry || !proxyKey || !oauthProxyUrl) return
    const tpl = expandedTpl
    const connName = expandedEntry.instance
    setBusy('oauth')
    setError(null)
    try {
      const sessionId = crypto.randomUUID()
      const scopes = proxyProviders[proxyKey]?.scopes?.join(',') || ''
      const url = `${oauthProxyUrl}/api/v1/oauth/connect/${encodeURIComponent(proxyKey)}?session=${sessionId}&scopes=${encodeURIComponent(scopes)}`

      const w = 600
      const h = 700
      const left = window.screenX + (window.innerWidth - w) / 2
      const top = window.screenY + (window.innerHeight - h) / 2
      window.open(url, 'oauth', `width=${w},height=${h},left=${left},top=${top}`)
      setOauthSession({ sessionId, connName })

      const maxAttempts = 60
      for (let i = 0; i < maxAttempts; i++) {
        await new Promise(r => setTimeout(r, 3000))
        let payload
        try {
          const resp = await fetch(`${oauthProxyUrl}/api/v1/oauth/sessions/${sessionId}`)
          if (!resp.ok) continue
          payload = await resp.json()
        } catch {
          continue
        }
        if (payload.status === 'completed' && payload.access_token) {
          const conn = connectors.find(c => c.name === connName)
          const tokenKey =
            conn?.oauth_token_key ||
            connName.toUpperCase().replace(/-/g, '_') + '_TOKEN'
          await fetch(`/api/settings/vault/${encodeURIComponent(tokenKey)}`, {
            method: 'PUT',
            headers: apiHeaders(),
            body: JSON.stringify({ value: payload.access_token }),
          })
          if (payload.refresh_token) {
            const refreshKey =
              connName.toUpperCase().replace(/-/g, '_') + '_REFRESH_TOKEN'
            await fetch(`/api/settings/vault/${encodeURIComponent(refreshKey)}`, {
              method: 'PUT',
              headers: apiHeaders(),
              body: JSON.stringify({ value: payload.refresh_token }),
            })
          }
          const update = { status: 'connected' }
          if (payload.refresh_token) {
            update.oauth_refresh_key =
              connName.toUpperCase().replace(/-/g, '_') + '_REFRESH_TOKEN'
          }
          if (payload.expires_in) {
            update.oauth_expires_at = new Date(
              Date.now() + payload.expires_in * 1000,
            ).toISOString()
          }
          await fetch(`/api/settings/connectors/${encodeURIComponent(connName)}`, {
            method: 'PUT',
            headers: apiHeaders(),
            body: JSON.stringify(update),
          })
          await reload()
          setOauthSession(null)
          setBusy(null)
          setExpanded(null)
          showToast(`${tpl.display_name} wired up`)
          return
        }
        if (payload.status === 'error') {
          throw new Error(payload.error_message || 'OAuth failed')
        }
      }
      throw new Error('OAuth timed out')
    } catch (e) {
      setError(e.message)
      setOauthSession(null)
      setBusy(null)
    }
  }

  const handleSaveSecrets = async () => {
    if (!expandedTpl || !expandedEntry) return
    const tpl = expandedTpl
    const connName = expandedEntry.instance
    for (const key of tpl.secrets || []) {
      const provided = secretValues[key]?.trim()
      const alreadyStored = vaultKeys.has(key)
      if (!provided && !alreadyStored) {
        setError(`${key} is required`)
        return
      }
    }
    setBusy('save')
    setError(null)
    try {
      for (const key of tpl.secrets || []) {
        const value = secretValues[key]?.trim()
        if (!value) continue
        const r = await fetch(`/api/settings/vault/${encodeURIComponent(key)}`, {
          method: 'PUT',
          headers: apiHeaders(),
          body: JSON.stringify({ value }),
        })
        if (!r.ok) throw new Error(`Failed to save ${key}`)
      }
      await fetch(`/api/settings/connectors/${encodeURIComponent(connName)}`, {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({ status: 'connected' }),
      })
      await reload()
      setSecretValues({})
      setExpanded(null)
      showToast(`${tpl.display_name} wired up`)
    } catch (e) {
      setError(e.message)
    }
    setBusy(null)
  }

  // ── Custom API creation ────────────────────────────────────────────────

  const openCustomPanel = useCallback(() => {
    setError(null)
    setCustomResult(null)
    setExpanded(null)
    setCustomForm({ name: '', api_key: '', docs_url: '', description: '' })
    setCustomOpen(true)
  }, [])

  const closeCustomPanel = useCallback(() => {
    setCustomOpen(false)
    setCustomBusy(false)
    setCustomResult(null)
  }, [])

  const handleCreateCustom = useCallback(async () => {
    const name = customForm.name.trim().toLowerCase()
    const api_key = customForm.api_key.trim()
    const docs_url = customForm.docs_url.trim()
    const description = customForm.description.trim()
    if (!name) { setError('Name is required'); return }
    if (!/^[a-z0-9-]+$/.test(name)) {
      setError('Name must be lowercase letters, digits, and hyphens only')
      return
    }
    if (name.length > 63) { setError('Name must be ≤ 63 characters'); return }
    if (!api_key) { setError('API key is required'); return }
    if (!docs_url) { setError('Docs URL is required'); return }
    setError(null)
    setCustomBusy(true)
    try {
      const resp = await fetch('/api/settings/connectors/custom', {
        method: 'POST',
        headers: apiHeaders(),
        body: JSON.stringify({
          name,
          api_key,
          docs_url,
          ...(description ? { description } : {}),
        }),
      })
      if (!resp.ok) {
        const d = await resp.json().catch(() => ({}))
        throw new Error(d.error || 'Failed to create custom connector')
      }
      const result = await resp.json()
      await reload()
      setCustomResult(result)
      setCustomForm({ name: '', api_key: '', docs_url: '', description: '' })
      showToast(`${result.connector?.display_name || name} wired up`)
    } catch (e) {
      setError(e.message)
    } finally {
      setCustomBusy(false)
    }
  }, [customForm, reload, showToast])

  const handleChipClick = useCallback((instanceName) => {
    setError(null)
    setSecretValues({})
    setExpanded(prev => (prev === instanceName ? null : instanceName))
  }, [])

  const handleChipRemove = useCallback(
    (e, entry) => {
      e.stopPropagation()
      unequipTemplate(entry.tpl || { name: entry.type, display_name: entry.name }, entry.instance)
    },
    [unequipTemplate],
  )

  const handleContinue = () => {
    updateData({ connectors: connectedRich })
    onNext()
  }

  if (loading) {
    return <div className="ob-loading"><span className="ob-spinner" /></div>
  }

  const renderGrid = (items) => (
    <div className="ob-conn-grid">
      {items.map(tpl => {
        const conn = connectors.find(c => c.type === tpl.name)
        return (
          <ConnectorTile
            key={tpl.name}
            tpl={tpl}
            isEquipped={!!conn}
            isReady={conn?.status === 'connected'}
            onClick={() => handleTileClick(tpl)}
          />
        )
      })}
    </div>
  )

  return (
    <div>
      <h2 className="ob-heading">Pick your tools</h2>
      <p className="ob-desc">
        Tap anything {data.agentName} should be able to use. Credentials are
        optional — you can wire them up later.
      </p>

      {/* Manifest strip — the signature element */}
      <div className={`ob-manifest ${manifest.length === 0 ? 'ob-manifest--empty' : ''}`}>
        {manifest.length === 0 ? (
          <span className="ob-manifest-empty">No tools equipped yet</span>
        ) : (
          manifest.map(m => {
            const isOpen = expanded === m.instance
            return (
              <button
                key={m.instance}
                type="button"
                className={
                  'ob-chip' +
                  (m.ready ? ' ob-chip--ready' : ' ob-chip--pending') +
                  (isOpen ? ' ob-chip--open' : '')
                }
                onClick={() => handleChipClick(m.instance)}
                title={m.ready ? `${m.name} — wired up` : `${m.name} — tap to wire up`}
              >
                <span className="ob-chip-logo">{LOGOS[m.type] || FALLBACK_LOGO}</span>
                <span className="ob-chip-name">{m.name}</span>
                <span className={`ob-chip-status ob-chip-status--${m.ready ? 'ok' : 'warn'}`} />
                <span
                  role="button"
                  tabIndex={-1}
                  className="ob-chip-remove"
                  onClick={(e) => handleChipRemove(e, m)}
                  aria-label={`Remove ${m.name}`}
                >
                  ×
                </span>
              </button>
            )
          })
        )}
      </div>

      {/* Inline credential pane (opt-in, chip-triggered) */}
      {expandedEntry && expandedTpl && (
        <div className="ob-conn-panel">
          <div className="ob-conn-panel-header">
            <span className="ob-conn-panel-logo">
              {LOGOS[expandedTpl.name] || FALLBACK_LOGO}
            </span>
            <div>
              <div className="ob-conn-panel-name">{expandedTpl.display_name}</div>
              {expandedTpl.description && (
                <div className="ob-conn-panel-desc">{expandedTpl.description}</div>
              )}
            </div>
            <button
              type="button"
              className="ob-conn-panel-close"
              onClick={() => setExpanded(null)}
              aria-label="Close"
            >
              ×
            </button>
          </div>

          {expandedTpl.socket_mode ? (
            <div className="ob-conn-panel-body">
              <p className="ob-hint">
                {expandedTpl.display_name} uses a guided multi-step setup. Open{' '}
                <strong>Settings → Connectors</strong> after onboarding to complete it.
              </p>
            </div>
          ) : oauthProxyUrl && proxyKey ? (
            <div className="ob-conn-panel-body">
              <p className="ob-hint">OAuth managed by Starpod — no credentials needed.</p>
              {oauthSession ? (
                <div className="ob-conn-waiting">
                  <span className="ob-spinner ob-spinner--sm" />
                  Waiting for authorization…
                </div>
              ) : (
                <button
                  type="button"
                  className="ob-btn-primary ob-btn-primary--sm"
                  onClick={handleProxyOAuth}
                  disabled={busy === 'oauth'}
                >
                  {expandedEntry.ready
                    ? `Reconnect with ${expandedTpl.display_name}`
                    : `Connect with ${expandedTpl.display_name}`}
                </button>
              )}
            </div>
          ) : (expandedTpl.secrets?.length || 0) > 0 ? (
            <div className="ob-conn-panel-body">
              {expandedTpl.secrets.map(key => {
                const stored = vaultKeys.has(key)
                return (
                  <div key={key} className="ob-field-group">
                    <label className="ob-label ob-label--sm">
                      {key}
                      {stored && <span className="ob-stored"> · saved</span>}
                    </label>
                    <input
                      type="password"
                      className="ob-input ob-input--mono"
                      value={secretValues[key] || ''}
                      onChange={e =>
                        setSecretValues(prev => ({ ...prev, [key]: e.target.value }))
                      }
                      placeholder={stored ? '••••••• (replace)' : `Enter ${key}`}
                      autoComplete="off"
                    />
                  </div>
                )
              })}
              <div className="ob-actions ob-actions--end">
                <button
                  type="button"
                  className="ob-btn-primary ob-btn-primary--sm"
                  onClick={handleSaveSecrets}
                  disabled={busy === 'save'}
                >
                  {busy === 'save' ? 'Saving…' : 'Save'}
                </button>
              </div>
            </div>
          ) : (
            <div className="ob-conn-panel-body">
              <p className="ob-hint">No credentials required.</p>
            </div>
          )}
        </div>
      )}

      {customOpen && (
        <div className="ob-conn-panel">
          <div className="ob-conn-panel-header">
            <span className="ob-conn-panel-logo" aria-hidden="true">+</span>
            <div>
              <div className="ob-conn-panel-name">Custom API</div>
              <div className="ob-conn-panel-desc">
                Paste an API key and a docs URL. Starpod reads the docs and
                writes a skill that teaches {data.agentName} how to call it.
              </div>
            </div>
            <button
              type="button"
              className="ob-conn-panel-close"
              onClick={closeCustomPanel}
              aria-label="Close"
            >
              ×
            </button>
          </div>
          {customResult ? (
            <div className="ob-conn-panel-body">
              <p className="ob-hint">
                <strong>{customResult.connector?.display_name}</strong> wired
                up. Skill <code>{customResult.skill_name}</code> created and
                key stored in <code>${customResult.env_var}</code>.
              </p>
              <details>
                <summary>View generated skill description</summary>
                <p className="ob-hint" style={{ marginTop: '0.5rem' }}>
                  {customResult.generated_description}
                </p>
              </details>
              <div className="ob-actions ob-actions--end">
                <button
                  type="button"
                  className="ob-btn-primary ob-btn-primary--sm"
                  onClick={closeCustomPanel}
                >
                  Done
                </button>
              </div>
            </div>
          ) : (
            <div className="ob-conn-panel-body">
              <div className="ob-field-group">
                <label className="ob-label ob-label--sm">
                  Name
                  <span className="ob-stored"> · kebab-case</span>
                </label>
                <input
                  type="text"
                  className="ob-input ob-input--mono"
                  value={customForm.name}
                  onChange={e =>
                    setCustomForm(prev => ({ ...prev, name: e.target.value }))
                  }
                  placeholder="e.g. semrush"
                  autoComplete="off"
                  disabled={customBusy}
                />
              </div>
              <div className="ob-field-group">
                <label className="ob-label ob-label--sm">API key</label>
                <input
                  type="password"
                  className="ob-input ob-input--mono"
                  value={customForm.api_key}
                  onChange={e =>
                    setCustomForm(prev => ({ ...prev, api_key: e.target.value }))
                  }
                  placeholder="Paste the API key"
                  autoComplete="off"
                  disabled={customBusy}
                />
              </div>
              <div className="ob-field-group">
                <label className="ob-label ob-label--sm">Docs URL</label>
                <input
                  type="url"
                  className="ob-input ob-input--mono"
                  value={customForm.docs_url}
                  onChange={e =>
                    setCustomForm(prev => ({ ...prev, docs_url: e.target.value }))
                  }
                  placeholder="https://developer.example.com/api"
                  autoComplete="off"
                  disabled={customBusy}
                />
              </div>
              <div className="ob-field-group">
                <label className="ob-label ob-label--sm">
                  Description
                  <span className="ob-stored"> · optional</span>
                </label>
                <input
                  type="text"
                  className="ob-input"
                  value={customForm.description}
                  onChange={e =>
                    setCustomForm(prev => ({ ...prev, description: e.target.value }))
                  }
                  placeholder="What is this API used for?"
                  autoComplete="off"
                  disabled={customBusy}
                />
              </div>
              <div className="ob-actions ob-actions--end">
                <button
                  type="button"
                  className="ob-btn-primary ob-btn-primary--sm"
                  onClick={handleCreateCustom}
                  disabled={customBusy}
                >
                  {customBusy ? 'Generating skill…' : 'Create'}
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      <div className="ob-conn-toolbar">
        <input
          type="text"
          className="ob-input ob-input--mono"
          placeholder="Filter connectors…"
          value={search}
          onChange={e => setSearch(e.target.value)}
        />
      </div>

      {searchResults ? (
        searchResults.length === 0 ? (
          <div className="ob-empty">No connectors match "{search}"</div>
        ) : (
          renderGrid(searchResults)
        )
      ) : (
        <>
          {categorized.map(section => (
            <div key={section.label} className="ob-conn-section">
              <div className="ob-conn-section-label">{section.label}</div>
              {renderGrid(section.items)}
            </div>
          ))}
          <div className="ob-conn-section">
            <div className="ob-conn-section-label">Custom</div>
            <div className="ob-conn-grid">
              <button
                type="button"
                onClick={openCustomPanel}
                className={
                  'ob-conn-tile' + (customOpen ? ' ob-conn-tile--equipped' : '')
                }
                title="Add a custom API by key + docs URL"
              >
                <span className="ob-conn-logo" aria-hidden="true">+</span>
                <span className="ob-conn-name">Custom API</span>
              </button>
            </div>
          </div>
        </>
      )}

      {error && <p className="ob-error">{error}</p>}

      {toast && <div className="ob-toast">✓ {toast}</div>}

      <div className="ob-actions">
        <button onClick={onBack} className="ob-btn-back" type="button">Back</button>
        <button onClick={handleContinue} className="ob-btn-primary" type="button">
          {manifest.length === 0
            ? 'Skip for now'
            : needsSetupCount > 0
              ? `Continue · ${needsSetupCount} to wire up later`
              : 'Continue'}
        </button>
      </div>
    </div>
  )
}
