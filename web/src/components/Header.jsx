import React, { useCallback } from 'react'
import { useApp } from '../contexts/AppContext'
import IconButton from './ui/IconButton'
import { SidebarOpenIcon, ComposeIcon, ChevronDownIcon, EllipsisIcon } from './ui/Icons'

function Header({ onToggleSidebar, onNewChat }) {
  const { state, dispatch } = useApp()
  const { wsStatus, sidebarOpen, previewUrl, currentSessionId, sessions } = state
  const sidebarVisible = sidebarOpen && !previewUrl
  const isTransient = sidebarOpen && !!previewUrl

  const cfg = window.__STARPOD__ || {}
  const modelName = cfg.model || 'claude'
  const agentName = cfg.agent_name || 'starpod'

  // Find current session title
  const currentSession = currentSessionId ? sessions.find(s => s.id === currentSessionId) : null
  const sessionTitle = currentSession?.title || currentSession?.summary || null

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

  function openModelSettings() {
    dispatch({ type: 'SHOW_SETTINGS' })
    dispatch({ type: 'SET_SETTINGS_TAB', payload: 'general' })
  }

  return (
    <header className="flex items-center gap-2 px-3 h-12 shrink-0" id="main-header">
      {!sidebarVisible && (
        <IconButton onClick={toggleSidebar} onMouseEnter={isTransient ? peekSidebar : undefined} aria-label="Open sidebar">
          <SidebarOpenIcon />
        </IconButton>
      )}
      <IconButton onClick={newChat} id="new-chat-header-btn" title="New chat" aria-label="New chat">
        <ComposeIcon className="w-4 h-4 stroke-current fill-none stroke-2" />
      </IconButton>

      {/* Model pill */}
      <button
        className="model-pill"
        onClick={openModelSettings}
        title="Change model"
      >
        <span>{agentName}</span>
        <ChevronDownIcon className="w-2.5 h-2.5 opacity-50" />
      </button>

      {/* Session title (centered) */}
      {sessionTitle && (
        <div className="flex-1 min-w-0 flex justify-center">
          <span className="text-xs text-muted truncate max-w-[300px] select-none">
            {sessionTitle}
          </span>
        </div>
      )}
      {!sessionTitle && <div className="flex-1" />}

      {/* Right side */}
      <div className="flex items-center gap-1.5">
        <span
          className={`w-2 h-2 rounded-full shrink-0 dot-${wsStatus}`}
          title={`WebSocket: ${wsStatus}`}
        />
        {wsStatus !== 'connected' && (
          <span className="text-[11px] text-muted select-none">
            {wsStatus === 'connecting' ? 'connecting...' : 'offline'}
          </span>
        )}
        <IconButton
          onClick={() => dispatch({ type: state.settingsVisible ? 'HIDE_SETTINGS' : 'SHOW_SETTINGS' })}
          aria-label="More options"
          title="Settings"
        >
          <EllipsisIcon className="w-4 h-4" />
        </IconButton>
      </div>
    </header>
  )
}

export default Header
