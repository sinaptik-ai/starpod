import { useState } from 'react'
import { apiHeaders } from '../../lib/api'

export default function StepRole({ data, updateData, onNext, onBack }) {
  const [mode, setMode] = useState(null) // null | 'custom'
  const [prompt, setPrompt] = useState('')
  const [generating, setGenerating] = useState(false)
  const [preview, setPreview] = useState(null)
  const [error, setError] = useState(null)
  const [saving, setSaving] = useState(false)

  const handleSkip = () => onNext()

  const handleGenerate = async () => {
    if (!prompt.trim()) return
    setGenerating(true)
    setError(null)
    setPreview(null)
    try {
      const resp = await fetch('/api/settings/setup/generate-role', {
        method: 'POST', headers: apiHeaders(),
        body: JSON.stringify({ prompt: prompt.trim(), agent_name: data.agentName }),
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
      if (preview.soul_md) {
        await fetch('/api/settings/files/SOUL.md', {
          method: 'PUT', headers: apiHeaders(),
          body: JSON.stringify({ content: preview.soul_md }),
        })
      }
      if (preview.heartbeat_md && preview.heartbeat_md.trim()) {
        await fetch('/api/settings/files/HEARTBEAT.md', {
          method: 'PUT', headers: apiHeaders(),
          body: JSON.stringify({ content: preview.heartbeat_md }),
        })
      }
      updateData({ generatedSkills: preview.skills || [] })
      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  // Choice screen
  if (mode === null) {
    return (
      <div>
        <h2 className="ob-heading">Define a role</h2>
        <p className="ob-desc">
          What should {data.agentName} do? Describe a role and we'll generate its
          configuration, skills, and API integrations.
        </p>

        <div className="ob-choices">
          <button onClick={() => setMode('custom')} className="ob-choice-card ob-choice-card--primary">
            <div className="ob-choice-title">Describe a role</div>
            <div className="ob-choice-desc">
              Tell us what your agent should do — we'll generate its soul, skills, and required API keys.
            </div>
          </button>

          <button onClick={handleSkip} className="ob-choice-card">
            <div className="ob-choice-title ob-choice-title--muted">Use default</div>
            <div className="ob-choice-desc">
              Start with a general-purpose assistant. Customize later in Settings.
            </div>
          </button>
        </div>

        <div className="ob-actions">
          <button onClick={onBack} className="ob-btn-back">Back</button>
          <div />
        </div>
      </div>
    )
  }

  // Custom prompt / preview
  return (
    <div>
      <h2 className="ob-heading">Describe the role</h2>
      <p className="ob-desc">
        What should {data.agentName} do? Be specific about tools, integrations, and behaviors.
      </p>

      {!preview ? (
        <>
          <textarea
            className="ob-textarea"
            rows={5}
            value={prompt}
            onChange={e => setPrompt(e.target.value)}
            placeholder="A DevOps assistant that monitors GitHub PRs, posts Slack updates, and manages deployments..."
            disabled={generating}
            autoFocus
          />

          {error && <p className="ob-error">{error}</p>}

          {generating && (
            <div className="ob-generating">
              <span className="ob-spinner" />
              <span>Generating role configuration...</span>
            </div>
          )}

          <div className="ob-actions">
            <button onClick={() => setMode(null)} className="ob-btn-back">Back</button>
            <button
              onClick={handleGenerate}
              disabled={!prompt.trim() || generating}
              className="ob-btn-primary"
            >
              Generate
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="ob-preview-panel">
            <div className="ob-preview-header">SOUL.md</div>
            <div className="ob-preview-body">
              {preview.soul_md?.slice(0, 800)}{preview.soul_md?.length > 800 ? '...' : ''}
            </div>
          </div>

          {preview.heartbeat_md && preview.heartbeat_md.trim() && (
            <div className="ob-preview-panel">
              <div className="ob-preview-header">HEARTBEAT.md</div>
              <div className="ob-preview-body ob-preview-body--short">
                {preview.heartbeat_md}
              </div>
            </div>
          )}

          {preview.skills && preview.skills.length > 0 && (
            <div className="ob-preview-panel">
              <div className="ob-preview-header">
                Skills ({preview.skills.length})
              </div>
              <div className="ob-preview-skills">
                {preview.skills.map(s => (
                  <div key={s.name} className="ob-preview-skill">
                    <div className="ob-preview-skill-name">
                      {s.name}
                      {s.env?.secrets && Object.keys(s.env.secrets).length > 0 && (
                        <span className="ob-badge">
                          {Object.keys(s.env.secrets).length} key{Object.keys(s.env.secrets).length !== 1 ? 's' : ''}
                        </span>
                      )}
                    </div>
                    {s.description && <div className="ob-preview-skill-desc">{s.description}</div>}
                  </div>
                ))}
              </div>
            </div>
          )}

          {error && <p className="ob-error">{error}</p>}

          <div className="ob-actions">
            <button
              onClick={() => { setPreview(null); setError(null) }}
              className="ob-btn-back"
            >
              Regenerate
            </button>
            <button onClick={handleApply} disabled={saving} className="ob-btn-primary">
              {saving ? 'Applying...' : 'Apply & continue'}
            </button>
          </div>
        </>
      )}
    </div>
  )
}
