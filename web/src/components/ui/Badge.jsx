import React from 'react'

const variants = {
  accent: 'bg-accent/10 text-accent',
  muted: 'bg-elevated text-muted',
  ok: 'bg-ok-muted text-ok',
  err: 'bg-err-muted text-err',
  warn: 'bg-orange-muted text-orange',
  running: 'bg-accent-muted text-accent-soft badge-running',
}

export default function Badge({ children, variant = 'muted', className = '' }) {
  const base = 'text-[10px] px-1.5 py-0.5 rounded-none font-medium uppercase tracking-wider'
  const color = variants[variant] || variants.muted
  return (
    <span className={`${base} ${color}${className ? ' ' + className : ''}`}>
      {children}
    </span>
  )
}
