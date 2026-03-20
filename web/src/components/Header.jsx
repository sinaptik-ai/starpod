import React from 'react'
import { useApp } from '../contexts/AppContext'

function Header({ onToggleSidebar }) {
  const { state, dispatch } = useApp()
  const { wsStatus } = state

  function toggleSidebar() {
    dispatch({ type: 'TOGGLE_SIDEBAR' })
    if (onToggleSidebar) onToggleSidebar()
  }

  function newChat() {
    dispatch({ type: 'NEW_CHAT' })
  }

  return (
    <header className="flex items-center gap-3 px-4 h-12 shrink-0 border-b border-border-subtle">
      {!state.sidebarOpen && (
        <button
          onClick={toggleSidebar}
          className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
          aria-label="Open sidebar"
        >
          <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
          </svg>
        </button>
      )}
      {!state.sidebarOpen && (
        <span className="font-mono text-sm font-bold tracking-tight text-primary">starpod</span>
      )}
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
      <div className="ml-auto flex items-center gap-2 text-xs text-muted select-none">
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 dot-${wsStatus}`} />
        <span>{wsStatus}</span>
      </div>
    </header>
  )
}

export default Header
