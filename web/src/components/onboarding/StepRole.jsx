import { useState, useMemo, useRef, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'

// Step 3 — describe what the agent should do.
//
// Generates SOUL.md / HEARTBEAT.md / suggested skills via the existing
// /api/settings/setup/generate-role endpoint, then on Apply installs the
// suggested skills directly (the previous standalone "skills" step is folded
// in here so the wizard ends in three meaningful steps instead of four).

export default function StepRole({ data, updateData, onNext, onBack }) {
  const [prompt, setPrompt] = useState('')
  const [generating, setGenerating] = useState(false)
  const [preview, setPreview] = useState(null)
  const [error, setError] = useState(null)
  const [saving, setSaving] = useState(false)
  const textareaRef = useRef(null)

  useEffect(() => {
    const t = setTimeout(() => textareaRef.current?.focus(), 100)
    return () => clearTimeout(t)
  }, [])

  // data.connectors is now an array of rich objects:
  // { type, instance, name (display_name), description }
  // Older versions stored plain strings — normalize for safety.
  const connectors = useMemo(
    () =>
      (data.connectors || []).map(c =>
        typeof c === 'string' ? { name: c, description: '' } : c,
      ),
    [data.connectors],
  )
  const hasConnectors = connectors.length > 0
  const connectorNames = connectors.map(c => c.name)

  const subtitle = useMemo(() => {
    if (!hasConnectors) {
      return `Describe what ${data.agentName} should do. We'll generate its personality, daily routine, and skills.`
    }
    const names = connectorNames
    const list =
      names.length === 1
        ? names[0]
        : names.length === 2
          ? `${names[0]} and ${names[1]}`
          : `${names.slice(0, -1).join(', ')}, and ${names[names.length - 1]}`
    return `${data.agentName} can use ${list}. Describe what it should do — we'll wire skills automatically.`
  }, [connectorNames, hasConnectors, data.agentName])

  const handleGenerate = async () => {
    if (!prompt.trim()) return
    setGenerating(true)
    setError(null)
    setPreview(null)
    try {
      // Inject rich connector context into the prompt so the LLM picks
      // compatible skills and understands what each integration is for.
      const integrationsBlock = hasConnectors
        ? '\n\nAvailable integrations:\n' +
          connectors
            .map(c =>
              c.description ? `- ${c.name}: ${c.description}` : `- ${c.name}`,
            )
            .join('\n')
        : ''
      const augmentedPrompt = prompt.trim() + integrationsBlock

      const resp = await fetch('/api/settings/setup/generate-role', {
        method: 'POST',
        headers: apiHeaders(),
        body: JSON.stringify({
          prompt: augmentedPrompt,
          agent_name: data.agentName,
          // Structured hint — backend may ignore unknown fields safely.
          connectors: connectors.map(c => ({
            type: c.type,
            name: c.name,
            description: c.description,
          })),
        }),
      })
      if (!resp.ok) {
        const d = await resp.json().catch(() => ({}))
        throw new Error(d.error || 'Generation failed')
      }
      setPreview(await resp.json())
    } catch (e) {
      setError(e.message)
    }
    setGenerating(false)
  }

  const handleApply = async () => {
    if (!preview) return
    setSaving(true)
    setError(null)
    try {
      // 1. SOUL.md
      if (preview.soul_md) {
        await fetch('/api/settings/files/SOUL.md', {
          method: 'PUT',
          headers: apiHeaders(),
          body: JSON.stringify({ content: preview.soul_md }),
        })
      }

      // 2. HEARTBEAT.md
      if (preview.heartbeat_md && preview.heartbeat_md.trim()) {
        await fetch('/api/settings/files/HEARTBEAT.md', {
          method: 'PUT',
          headers: apiHeaders(),
          body: JSON.stringify({ content: preview.heartbeat_md }),
        })
      }

      // 3. Skills — install everything the generator returned.
      const skills = preview.skills || []
      for (const skill of skills) {
        const payload = {
          name: skill.name,
          description: skill.description,
          body: skill.body,
        }
        if (skill.env?.secrets && Object.keys(skill.env.secrets).length > 0) {
          payload.env = { secrets: skill.env.secrets }
        }
        const resp = await fetch('/api/settings/skills', {
          method: 'POST',
          headers: apiHeaders(),
          body: JSON.stringify(payload),
        })
        if (!resp.ok) {
          const d = await resp.json().catch(() => ({}))
          if (!d.error?.includes('already exists')) {
            throw new Error(d.error || `Failed to create skill "${skill.name}"`)
          }
        }
      }

      updateData({ generatedSkills: skills })
      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  // Preview screen
  if (preview) {
    const skills = preview.skills || []
    return (
      <div>
        <h2 className="ob-heading">Review</h2>
        <p className="ob-desc">
          Here's what we'll set up for {data.agentName}. Apply or regenerate.
        </p>

        <div className="ob-preview-panel">
          <div className="ob-preview-header">SOUL.md</div>
          <div className="ob-preview-body">
            {preview.soul_md?.slice(0, 800)}
            {preview.soul_md && preview.soul_md.length > 800 ? '…' : ''}
          </div>
        </div>

        {preview.heartbeat_md?.trim() && (
          <div className="ob-preview-panel">
            <div className="ob-preview-header">HEARTBEAT.md</div>
            <div className="ob-preview-body ob-preview-body--short">
              {preview.heartbeat_md}
            </div>
          </div>
        )}

        {skills.length > 0 && (
          <div className="ob-preview-panel">
            <div className="ob-preview-header">Skills ({skills.length})</div>
            <div className="ob-preview-skills">
              {skills.map(s => {
                const secretCount = Object.keys(s.env?.secrets || {}).length
                return (
                  <div key={s.name} className="ob-preview-skill">
                    <div className="ob-preview-skill-name">
                      {s.name}
                      {secretCount > 0 && (
                        <span className="ob-badge">
                          {secretCount} key{secretCount !== 1 ? 's' : ''}
                        </span>
                      )}
                    </div>
                    {s.description && (
                      <div className="ob-preview-skill-desc">{s.description}</div>
                    )}
                  </div>
                )
              })}
            </div>
          </div>
        )}

        {error && <p className="ob-error">{error}</p>}

        <div className="ob-actions">
          <button
            type="button"
            onClick={() => { setPreview(null); setError(null) }}
            className="ob-btn-back"
          >
            Regenerate
          </button>
          <button
            type="button"
            onClick={handleApply}
            disabled={saving}
            className="ob-btn-primary"
          >
            {saving ? 'Applying…' : 'Apply & finish'}
          </button>
        </div>
      </div>
    )
  }

  // Prompt screen
  return (
    <div>
      <h2 className="ob-heading">What should {data.agentName} do?</h2>
      <p className="ob-desc">{subtitle}</p>

      <textarea
        ref={textareaRef}
        className="ob-textarea"
        rows={6}
        value={prompt}
        onChange={e => setPrompt(e.target.value)}
        placeholder={
          hasConnectors
            ? `Audit on-page SEO weekly, monitor competitor rankings, and file issues for content gaps in GitHub.`
            : `A research assistant that summarizes long PDFs, drafts replies, and tracks open questions in a notebook.`
        }
        disabled={generating}
      />

      {error && <p className="ob-error">{error}</p>}

      {generating && (
        <div className="ob-generating">
          <span className="ob-spinner" />
          <span>Generating role configuration…</span>
        </div>
      )}

      <div className="ob-actions">
        <button type="button" onClick={onBack} className="ob-btn-back">Back</button>
        <button
          type="button"
          onClick={handleGenerate}
          disabled={!prompt.trim() || generating}
          className="ob-btn-primary"
        >
          Generate
        </button>
      </div>
    </div>
  )
}
