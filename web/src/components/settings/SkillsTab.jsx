import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Textarea, SaveBar } from './fields'
import SectionLabel from '../ui/SectionLabel'
import { Loading, Empty } from '../ui/EmptyState'

// Steps: name → description → context → generating/review
const STEP_NAME = 'name'
const STEP_DESC = 'description'
const STEP_CONTEXT = 'context'
const STEP_GENERATING = 'generating'

export default function SkillsTab() {
  const [skills, setSkills] = useState([])
  const [loading, setLoading] = useState(true)
  const [editName, setEditName] = useState(null)
  const [editDesc, setEditDesc] = useState('')
  const [editBody, setEditBody] = useState('')
  const [editSaving, setEditSaving] = useState(false)
  const [editStatus, setEditStatus] = useState(null)
  const [confirmDelete, setConfirmDelete] = useState(null)
  const [error, setError] = useState(null)

  // Wizard state
  const [wizardOpen, setWizardOpen] = useState(false)
  const [wizardStep, setWizardStep] = useState(STEP_NAME)
  const [wizName, setWizName] = useState('')
  const [wizDesc, setWizDesc] = useState('')
  const [wizContext, setWizContext] = useState('')
  const [wizError, setWizError] = useState(null)

  const load = async () => {
    try {
      const r = await fetch('/api/settings/skills', { headers: apiHeaders() })
      if (r.ok) setSkills((await r.json()) || [])
    } catch { setError('Failed to load') }
    setLoading(false)
  }

  useEffect(() => { load() }, [])

  // Reset wizard
  const openWizard = () => {
    setWizardOpen(true)
    setWizardStep(STEP_NAME)
    setWizName('')
    setWizDesc('')
    setWizContext('')
    setWizError(null)
  }

  const closeWizard = () => {
    setWizardOpen(false)
    setWizError(null)
  }

  // Create skill (blank or with generated content)
  const createSkill = async (name, description, body) => {
    setError(null)
    try {
      const r = await fetch('/api/settings/skills', {
        method: 'POST', headers: apiHeaders(),
        body: JSON.stringify({ name, description, body }),
      })
      if (r.ok) { closeWizard(); await load() }
      else { const d = await r.json().catch(() => ({})); setWizError(d.error || 'Failed to create') }
    } catch (e) { setWizError(e.message) }
  }

  // Generate with AI then create
  const generateAndCreate = async () => {
    setWizardStep(STEP_GENERATING)
    setWizError(null)
    try {
      const r = await fetch('/api/settings/skills/generate', {
        method: 'POST', headers: apiHeaders(),
        body: JSON.stringify({
          name: wizName.trim(),
          description: wizDesc.trim() || null,
          prompt: wizContext.trim() || null,
        }),
      })
      if (r.ok) {
        const gen = await r.json()
        const desc = wizDesc.trim() || gen.description
        await createSkill(wizName.trim(), desc, gen.body)
      } else {
        const d = await r.json().catch(() => ({}))
        setWizError(d.error || 'AI generation failed')
        setWizardStep(STEP_CONTEXT) // go back so user can retry
      }
    } catch (e) {
      setWizError(e.message)
      setWizardStep(STEP_CONTEXT)
    }
  }

  // Create blank (skip AI)
  const createBlank = async () => {
    await createSkill(wizName.trim(), wizDesc.trim(), '')
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

  if (loading) return <Loading />

  return (
    <>
      {/* Header + new skill button */}
      {!wizardOpen && (
        <div className="flex items-center justify-between mb-4">
          <div className="text-dim text-xs">{skills.length} skill{skills.length !== 1 ? 's' : ''}</div>
          <button onClick={openWizard} className="s-save-btn text-xs">
            New skill
          </button>
        </div>
      )}

      {/* Wizard / Create */}
      {wizardOpen && (
        <div className="mb-4 p-4 rounded-lg border border-border-subtle bg-elevated/50">
          <div className="flex items-center justify-between mb-3">
            <SectionLabel>New Skill</SectionLabel>
            <button onClick={closeWizard} className="text-xs text-muted hover:text-primary cursor-pointer transition-colors">
              cancel
            </button>
          </div>

          {/* Step 1: Name */}
          <div className="mb-3">
            <SectionLabel className="mb-1.5">Name</SectionLabel>
            <input
              type="text"
              className="s-input font-mono text-xs w-full"
              value={wizName}
              onChange={e => setWizName(e.target.value)}
              placeholder="my-skill-name"
              disabled={wizardStep === STEP_GENERATING}
              autoFocus
              onKeyDown={e => {
                if (e.key === 'Enter' && wizName.trim() && wizardStep === STEP_NAME) {
                  setWizardStep(STEP_DESC)
                }
              }}
            />
          </div>

          {/* Step 2: Description */}
          {wizardStep !== STEP_NAME && (
            <div className="mb-3">
              <SectionLabel className="mb-1.5">
                Description <span className="text-dim/50">(optional)</span>
              </SectionLabel>
              <input
                type="text"
                className="s-input text-xs w-full"
                value={wizDesc}
                onChange={e => setWizDesc(e.target.value)}
                placeholder="What does this skill do?"
                disabled={wizardStep === STEP_GENERATING}
                autoFocus
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    setWizardStep(STEP_CONTEXT)
                  }
                }}
              />
            </div>
          )}

          {/* Step 3: Extra context */}
          {(wizardStep === STEP_CONTEXT || wizardStep === STEP_GENERATING) && (
            <div className="mb-3">
              <SectionLabel className="mb-1.5">
                Extra instructions / context <span className="text-dim/50">(optional)</span>
              </SectionLabel>
              <Textarea
                value={wizContext}
                onChange={setWizContext}
                rows={4}
                placeholder="Any additional context or instructions for generating the skill body..."
                disabled={wizardStep === STEP_GENERATING}
              />
            </div>
          )}

          {wizError && <div className="text-err text-xs mb-3">{wizError}</div>}

          {/* Action buttons */}
          <div className="flex gap-2 items-center">
            {wizardStep === STEP_NAME && (
              <button
                onClick={() => setWizardStep(STEP_DESC)}
                disabled={!wizName.trim()}
                className="s-save-btn whitespace-nowrap"
              >
                Next
              </button>
            )}
            {wizardStep === STEP_DESC && (
              <button
                onClick={() => setWizardStep(STEP_CONTEXT)}
                className="s-save-btn whitespace-nowrap"
              >
                Next
              </button>
            )}
            {wizardStep === STEP_CONTEXT && (
              <>
                <button onClick={generateAndCreate} className="s-save-btn whitespace-nowrap">
                  Generate with AI
                </button>
                <button onClick={createBlank} className="text-xs text-muted hover:text-primary cursor-pointer transition-colors">
                  or create blank
                </button>
              </>
            )}
            {wizardStep === STEP_GENERATING && (
              <div className="flex items-center gap-2 text-xs text-dim">
                <span className="inline-block w-3 h-3 border-2 border-dim/30 border-t-primary rounded-full animate-spin" />
                Generating skill...
              </div>
            )}
          </div>
        </div>
      )}

      {error && <div className="text-err text-xs mb-3">{error}</div>}

      {skills.length === 0 && !wizardOpen ? (
        <Empty text="No skills" />
      ) : (
        <div className="flex flex-col gap-1.5">
          {skills.map(s => (
            <div key={s.name} className="skill-card">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm text-primary truncate">{s.name}</span>
                    {s.version && <span className="text-[10px] text-dim bg-elevated px-1.5 py-0.5 rounded shrink-0">{s.version}</span>}
                  </div>
                  {s.description && <div className="text-dim text-xs mt-1 line-clamp-2">{s.description}</div>}
                </div>
                <div className="flex gap-3 shrink-0 pt-0.5">
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
                    <SectionLabel className="mb-1.5">Description</SectionLabel>
                    <input
                      type="text"
                      className="s-input text-xs w-full"
                      value={editDesc}
                      onChange={e => setEditDesc(e.target.value)}
                      placeholder="Short description of what this skill does"
                    />
                  </div>
                  <SectionLabel className="mb-1.5">Body</SectionLabel>
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
