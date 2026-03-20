import { useState } from 'react'
import Tooltip from './Tooltip'

// Card-like section grouping
export function Card({ title, desc, children }) {
  return (
    <div className="s-card">
      {title && (
        <div className="s-card-header">
          <span className="s-card-title">{title}</span>
          {desc && <span className="s-card-desc">{desc}</span>}
        </div>
      )}
      <div className="s-card-body">{children}</div>
    </div>
  )
}

// Horizontal field: label left, control right
export function Row({ label, helpTip, mono, sub, children }) {
  return (
    <div className="s-row">
      <div className="s-row-label">
        <span>{label}{helpTip && <Tooltip text={helpTip} />}</span>
        {sub && <span className="s-row-sub">{sub}</span>}
      </div>
      <div className={`s-row-control${mono ? ' font-mono' : ''}`}>{children}</div>
    </div>
  )
}

// Full-width field (for textareas, wide inputs)
export function Field({ label, helpTip, desc, children }) {
  return (
    <div className="s-field">
      {label && (
        <label className="s-field-label">
          {label}{helpTip && <Tooltip text={helpTip} />}
        </label>
      )}
      {desc && <div className="s-field-desc">{desc}</div>}
      {children}
    </div>
  )
}

export function Input({ value, onChange, type = 'text', mono, className = '', ...props }) {
  return (
    <input
      type={type}
      value={value ?? ''}
      onChange={e => onChange(e.target.value)}
      className={`s-input${mono ? ' font-mono text-xs' : ''}${className ? ' ' + className : ''}`}
      {...props}
    />
  )
}

export function Select({ value, onChange, options }) {
  return (
    <select value={value ?? ''} onChange={e => onChange(e.target.value)} className="s-input s-select">
      {options.map(o => {
        const val = typeof o === 'string' ? o : o.value
        const label = typeof o === 'string' ? o : o.label
        return <option key={val} value={val}>{label}</option>
      })}
    </select>
  )
}

export function Toggle({ checked, onChange, label, helpTip }) {
  return (
    <div className="s-toggle-row">
      <div className="s-row-label">
        <span>{label}{helpTip && <Tooltip text={helpTip} />}</span>
      </div>
      <label className="s-toggle">
        <input type="checkbox" checked={checked ?? false} onChange={e => onChange(e.target.checked)} />
        <div className="s-toggle-track" />
      </label>
    </div>
  )
}

export function Textarea({ value, onChange, rows = 10, mono = true }) {
  return (
    <textarea
      rows={rows}
      value={value ?? ''}
      onChange={e => onChange(e.target.value)}
      className={`s-input s-textarea${mono ? ' font-mono text-xs' : ''}`}
    />
  )
}

export function ModelSelect({ value, onChange, models = [] }) {
  const isCustom = value && models.length > 0 && !models.includes(value)
  const [showCustom, setShowCustom] = useState(isCustom)
  const [customValue, setCustomValue] = useState(isCustom ? value : '')

  return (
    <div className="flex gap-2 items-center">
      <select
        className="s-input s-select flex-1"
        value={showCustom ? '__custom__' : (value || '')}
        onChange={e => {
          if (e.target.value === '__custom__') { setShowCustom(true) }
          else { setShowCustom(false); setCustomValue(''); onChange(e.target.value) }
        }}
      >
        {models.map(m => <option key={m} value={m}>{m}</option>)}
        <option value="__custom__">Custom...</option>
      </select>
      {showCustom && (
        <input
          type="text"
          className="s-input font-mono text-xs flex-1"
          value={customValue}
          onChange={e => { setCustomValue(e.target.value); onChange(e.target.value) }}
          placeholder="model-id"
          autoFocus
        />
      )}
    </div>
  )
}

export function SaveBar({ onSave, saving, status }) {
  return (
    <div className="s-save-bar">
      <button onClick={onSave} disabled={saving} className="s-save-btn">
        {saving ? 'Saving...' : 'Save'}
      </button>
      {status && (
        <span className={`s-save-status ${status.type === 'error' ? 'text-err' : status.type === 'ok' ? 'text-ok' : 'text-dim'}`}>
          {status.text}
        </span>
      )}
    </div>
  )
}

// Kbd hint
export function Kbd({ children }) {
  return <kbd className="s-kbd">{children}</kbd>
}
