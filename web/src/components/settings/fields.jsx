import { useState } from 'react'

export function Section({ title }) {
  return <div className="settings-section-header">{title}</div>
}

export function Field({ label, desc, helpTip, children }) {
  return (
    <div className="settings-field">
      <label>
        {label}
        {helpTip && <span className="help-icon" title={helpTip}>?</span>}
      </label>
      {desc && <div className="field-desc">{desc}</div>}
      {children}
    </div>
  )
}

export function Input({ id, value, onChange, type = 'text', ...props }) {
  return <input id={id} type={type} value={value} onChange={e => onChange(e.target.value)} className="settings-input" {...props} />
}

export function Select({ id, value, onChange, options }) {
  return (
    <select id={id} value={value} onChange={e => onChange(e.target.value)} className="settings-input">
      {options.map(o => {
        const val = typeof o === 'string' ? o : o.value
        const label = typeof o === 'string' ? o : o.label
        return <option key={val} value={val}>{label}</option>
      })}
    </select>
  )
}

export function Toggle({ id, checked, onChange }) {
  return (
    <label className="toggle-switch">
      <input type="checkbox" id={id} checked={checked} onChange={e => onChange(e.target.checked)} />
      <div className="toggle-track" />
    </label>
  )
}

export function Textarea({ id, value, onChange, rows = 10 }) {
  return <textarea id={id} rows={rows} value={value} onChange={e => onChange(e.target.value)} className="settings-input" />
}

export function SaveBar({ onSave, saving, status }) {
  return (
    <div className="settings-save-bar">
      <button onClick={onSave} disabled={saving} className="bg-accent text-white px-6 py-2.5 rounded-xl text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-default">
        Save changes
      </button>
      {status && <span className={`ml-3 text-xs ${status.type === 'error' ? 'text-err' : status.type === 'ok' ? 'text-ok' : 'text-dim'}`}>{status.text}</span>}
    </div>
  )
}

export function ModelSelect({ value, onChange, models = [] }) {
  const isCustom = value && !models.includes(value)
  const [showCustom, setShowCustom] = useState(isCustom)
  const [customValue, setCustomValue] = useState(isCustom ? value : '')

  return (
    <div className="flex gap-2">
      <select className="settings-input flex-1" value={showCustom ? '__custom__' : value} onChange={e => {
        if (e.target.value === '__custom__') { setShowCustom(true) }
        else { setShowCustom(false); onChange(e.target.value) }
      }}>
        {models.map(m => <option key={m} value={m}>{m}</option>)}
        <option value="__custom__">Custom...</option>
      </select>
      {showCustom && <input type="text" className="settings-input flex-1" value={customValue} onChange={e => { setCustomValue(e.target.value); onChange(e.target.value) }} placeholder="Model name..." />}
    </div>
  )
}
