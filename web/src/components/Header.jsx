import React from 'react'
import { useApp } from '../contexts/AppContext'

function Header() {
  const { state, dispatch } = useApp()
  const { wsStatus } = state

  function toggleSidebar() {
    dispatch({ type: 'TOGGLE_SIDEBAR' })
  }

  function newChat() {
    dispatch({ type: 'NEW_CHAT' })
  }

  return (
    <header className="flex items-center gap-3 px-4 h-12 shrink-0 border-b border-border-subtle">
      <button
        onClick={toggleSidebar}
        className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
        aria-label="Sessions"
      >
        <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
          <path d="M3 12h18M3 6h18M3 18h18" />
        </svg>
      </button>
      <button
        onClick={newChat}
        className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
        id="new-chat-header-btn"
        title="New chat"
      >
        <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
          <path d="M12 20h9M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
        </svg>
      </button>
      <div className="flex items-center gap-2">
        <span className="font-mono text-sm font-bold tracking-tight text-primary">starpod</span>
      </div>
      <div className="ml-auto flex items-center gap-2 text-xs text-muted select-none">
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 dot-${wsStatus}`} />
        <span>{wsStatus}</span>
      </div>
    </header>
  )
}

export default Header
