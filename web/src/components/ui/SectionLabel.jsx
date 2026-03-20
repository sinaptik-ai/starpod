import React from 'react'

export default function SectionLabel({ children, className = '' }) {
  return (
    <div className={`font-mono text-[10px] text-dim uppercase tracking-wider${className ? ' ' + className : ''}`}>
      {children}
    </div>
  )
}
