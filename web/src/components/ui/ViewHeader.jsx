import React from 'react'
import { useApp } from '../../contexts/AppContext'
import { SidebarOpenIcon } from './Icons'

export default function ViewHeader({ title, center, left, right, border = true }) {
  const { dispatch } = useApp()

  return (
    <div className={`grid grid-cols-[auto_1fr_auto] items-center gap-3 px-3 h-12 shrink-0${border ? ' border-b border-border-subtle' : ''}`}>
      <div className="flex items-center gap-2">
        <button
          onClick={() => dispatch({ type: 'OPEN_SIDEBAR' })}
          className="mobile-sidebar-toggle"
          aria-label="Open sidebar"
        >
          <SidebarOpenIcon />
        </button>
        {left || <h2 className="text-sm font-semibold text-primary tracking-tight">{title}</h2>}
      </div>
      {center ? (
        <div className="min-w-0 text-center">
          {center}
        </div>
      ) : <div />}
      {right ? <div className="flex items-center gap-2">{right}</div> : <div />}
    </div>
  )
}
