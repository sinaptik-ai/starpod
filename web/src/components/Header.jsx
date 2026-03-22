import React, { useCallback } from 'react'
import { useApp } from '../contexts/AppContext'
import IconButton from './ui/IconButton'
import { SidebarOpenIcon, ComposeIcon } from './ui/Icons'

function Header({ onToggleSidebar, onNewChat }) {
  const { state, dispatch } = useApp()
  const { wsStatus, sidebarOpen, previewUrl } = state
  const sidebarVisible = sidebarOpen && !previewUrl
  const isTransient = sidebarOpen && !!previewUrl

  const peekSidebar = useCallback(() => {
    const el = document.getElementById('sidebar')
    if (el) el.classList.add('peeking')
  }, [])

  function toggleSidebar() {
    if (isTransient) {
      peekSidebar()
      return
    }
    dispatch({ type: 'TOGGLE_SIDEBAR' })
    if (onToggleSidebar) onToggleSidebar()
  }

  function newChat() {
    if (onNewChat) onNewChat()
    else dispatch({ type: 'NEW_CHAT' })
  }

  return (
    <header className="flex items-center gap-3 px-4 h-12 shrink-0 border-b border-border-subtle">
      {!sidebarVisible && (
        <IconButton onClick={toggleSidebar} onMouseEnter={isTransient ? peekSidebar : undefined} aria-label="Open sidebar">
          <SidebarOpenIcon />
        </IconButton>
      )}
      <IconButton onClick={newChat} id="new-chat-header-btn" title="New chat" aria-label="New chat">
        <ComposeIcon className="w-4 h-4 stroke-current fill-none stroke-2" />
      </IconButton>
      {!sidebarVisible && (
        <div className="flex items-center gap-2">
          <span className="font-mono text-sm font-bold tracking-tight text-primary">starpod</span>
        </div>
      )}
      <div className="ml-auto flex items-center gap-2 text-xs text-muted select-none" title={`WebSocket: ${wsStatus}`}>
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 dot-${wsStatus}`} />
        <span>{wsStatus === 'connected' ? 'online' : wsStatus === 'connecting' ? 'connecting...' : 'offline'}</span>
      </div>
    </header>
  )
}

export default Header
