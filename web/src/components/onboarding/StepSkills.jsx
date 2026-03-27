import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'

export default function StepSkills({ data, onNext, onBack }) {
  const generated = data.generatedSkills || []
  const [skills, setSkills] = useState(() =>
    generated.map(s => ({
      ...s,
      enabled: true,
      envValues: buildEnvDefaults(s.env),
    }))
  )
  const [existing, setExisting] = useState([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    fetch('/api/settings/skills', { headers: apiHeaders() })
      .then(r => r.ok ? r.json() : [])
      .then(s => setExisting(s || []))
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const toggleSkill = (idx) => {
    setSkills(prev => prev.map((s, i) => i === idx ? { ...s, enabled: !s.enabled } : s))
  }

  const setEnvValue = (skillIdx, key, value) => {
    setSkills(prev => prev.map((s, i) => {
      if (i !== skillIdx) return s
      return { ...s, envValues: { ...s.envValues, [key]: value } }
    }))
  }

  const handleNext = async () => {
    const enabled = skills.filter(s => s.enabled)

    for (const skill of enabled) {
      const secrets = skill.env?.secrets || {}
      for (const [key, decl] of Object.entries(secrets)) {
        if (decl.required && !skill.envValues[key]?.trim()) {
          setError(`${key} is required for "${skill.name}"`)
          return
        }
      }
    }

    if (enabled.length === 0) {
      onNext()
      return
    }

    setSaving(true)
    setError(null)

    try {
      for (const skill of enabled) {
        const payload = {
          name: skill.name,
          description: skill.description,
          body: skill.body,
        }
        if (skill.env?.secrets && Object.keys(skill.env.secrets).length > 0) {
          payload.env = { secrets: skill.env.secrets }
        }

        const resp = await fetch('/api/settings/skills', {
          method: 'POST', headers: apiHeaders(),
          body: JSON.stringify(payload),
        })
        if (!resp.ok) {
          const d = await resp.json().catch(() => ({}))
          if (!d.error?.includes('already exists')) {
            throw new Error(d.error || `Failed to create skill "${skill.name}"`)
          }
        }

        for (const [key, value] of Object.entries(skill.envValues)) {
          if (value.trim()) {
            await fetch(`/api/settings/vault/${encodeURIComponent(key)}`, {
              method: 'PUT', headers: apiHeaders(),
              body: JSON.stringify({ value: value.trim() }),
            })
          }
        }
      }

      onNext()
    } catch (e) {
      setError(e.message)
    }
    setSaving(false)
  }

  if (loading) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: 200 }}>
        <span className="ob-spinner" />
      </div>
    )
  }

  const hasGenerated = skills.length > 0
  const enabledCount = skills.filter(s => s.enabled).length

  return (
    <div>
      <h2 className="ob-heading">Skills & integrations</h2>
      <p className="ob-desc">
        {hasGenerated
          ? 'Review the generated skills. Toggle each one and add any required API keys.'
          : existing.length > 0
            ? 'Your agent has these skills installed. You can manage them later in Settings.'
            : 'No skills to configure yet. You can add skills later in Settings.'}
      </p>

      {hasGenerated && (
        <div className="ob-skill-list">
          {skills.map((skill, idx) => {
            const secrets = skill.env?.secrets || {}
            const secretEntries = Object.entries(secrets)
            const hasSecrets = secretEntries.length > 0

            return (
              <div key={skill.name} className={`ob-skill-card ${skill.enabled ? 'ob-skill-card--active' : ''}`}>
                <div className="ob-skill-header">
                  <div className="ob-skill-info">
                    <div className="ob-skill-name">{skill.name}</div>
                    {skill.description && <div className="ob-skill-desc">{skill.description}</div>}
                  </div>
                  <button
                    onClick={() => toggleSkill(idx)}
                    className={`ob-toggle ${skill.enabled ? 'ob-toggle--on' : ''}`}
                    aria-label={`Toggle ${skill.name}`}
                  >
                    <span className="ob-toggle-thumb" />
                  </button>
                </div>

                {skill.enabled && hasSecrets && (
                  <div className="ob-skill-env">
                    {secretEntries.map(([key, decl]) => (
                      <div key={key} className="ob-skill-env-field">
                        <label className="ob-label ob-label--sm">
                          {key}
                          {decl.required && <span className="ob-required">*</span>}
                        </label>
                        {decl.description && <div className="ob-hint ob-hint--sm">{decl.description}</div>}
                        <input
                          type="password"
                          className="ob-input ob-input--mono"
                          value={skill.envValues[key] || ''}
                          onChange={e => setEnvValue(idx, key, e.target.value)}
                          placeholder={`Enter ${key}`}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}

      {existing.length > 0 && (
        <>
          {hasGenerated && <div className="ob-section-label">Already installed</div>}
          <div className="ob-skill-list">
            {existing.map(s => (
              <div key={s.name} className="ob-skill-card ob-skill-card--readonly">
                <div className="ob-skill-header">
                  <div className="ob-skill-info">
                    <div className="ob-skill-name">{s.name}</div>
                    {s.description && <div className="ob-skill-desc">{s.description}</div>}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </>
      )}

      {error && <p className="ob-error">{error}</p>}

      <div className="ob-actions">
        <button onClick={onBack} className="ob-btn-back">Back</button>
        <button onClick={handleNext} disabled={saving} className="ob-btn-primary">
          {saving
            ? 'Installing...'
            : hasGenerated && enabledCount > 0
              ? `Install ${enabledCount} skill${enabledCount !== 1 ? 's' : ''}`
              : 'Skip'}
        </button>
      </div>
    </div>
  )
}

function buildEnvDefaults(env) {
  if (!env?.secrets) return {}
  const vals = {}
  for (const key of Object.keys(env.secrets)) vals[key] = ''
  return vals
}
